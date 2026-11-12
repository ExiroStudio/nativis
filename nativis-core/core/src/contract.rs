//! Frozen `MediaBackend` contract — the central interface of Nativis.
//!
//! Every media backend (image, video, shader, webview) implements this trait.
//! The runtime and render engine communicate *only* through these types.
//! No wgpu, Vulkan, or other driver types appear here.

use std::time::Duration;
use thiserror::Error;

use crate::clock::MediaClock;
use nativis_asset::AssetPath;
use nativis_protocol::{
    NativisSurface, NATIVIS_FORMAT_NV12, NATIVIS_FORMAT_RGBA8888,
    NATIVIS_SURFACE_KIND_CPU_BUFFER, NATIVIS_SURFACE_KIND_CPU_PLANAR, NATIVIS_SURFACE_KIND_CPU_SHM,
};

// ── Resource System ──────────────────────────────────────────────────────────

/// Opaque handle to a media resource (CPU buffer, GPU texture, DMA-BUF, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceHandle(pub u64);

// ── Capability set ────────────────────────────────────────────────────────────

/// An individual media capability flag.
///
/// Backends return `&[MediaCapability]`. Adding a new variant never
/// breaks existing backends — they simply do not include it in their slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaCapability {
    /// Backend produces an audio stream alongside video.
    Audio,
    /// Backend supports random-access seeking.
    Seek,
    /// Backend can loop media seamlessly.
    Loop,
    /// Backend produces HDR-encoded frames (PQ / HLG).
    HDR,
    /// Backend produces frames with an alpha channel.
    Alpha,
    /// Backend can reload source content while running (e.g. live shaders).
    LiveReload,
    /// Backend carries subtitle / caption data.
    Subtitle,
}

// ── Source Classes (Phase 4) ──────────────────────────────────────────────────

/// Three distinct source classes defined in the Nativis engine (D1, D2):
///
/// 1. `Producer`    — Continuous frame streams (video, camera, capture) requiring buffer pools.
/// 2. `Declarative` — Static or procedural content (images, shaders) where host supplies time.
/// 3. `Embedded`   — Autonomous foreign surfaces (HTML, external windows) embedded into desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceClass {
    #[default]
    Producer,
    Declarative,
    Embedded,
}

// ── Scaling Modes (Phase 3) ───────────────────────────────────────────────────

/// Wallpaper presentation scaling mode per output (monitor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScalingMode {
    #[default]
    Fill,    // Crop to fill viewport preserving aspect ratio
    Fit,     // Letterbox to fit inside viewport
    Stretch, // Stretch to match viewport dimensions
    Center,  // Center 1:1 without scaling
    Tile,    // Repeat across viewport
    Span,    // Span single surface across all monitors
}

/// A decoded frame ready for transport or rendering.
///
/// Carries no implementation details. The resource could be in CPU RAM, GPU VRAM,
/// or a DMA-BUF file descriptor. The `FrameSink` will resolve it.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Opaque handle to the actual pixel data.
    pub resource: ResourceHandle,
    /// Pixel width of the frame.
    pub width: u32,
    /// Pixel height of the frame.
    pub height: u32,
    /// Presentation timestamp within the media stream.
    pub pts: Duration,
    /// When `true`, the frame has no transparent pixels.
    pub is_opaque: bool,
    /// Optional POD surface descriptor for zero-copy or direct handle passing (Phase 1).
    pub surface: Option<NativisSurface>,
}

impl Frame {
    /// Create a standard Frame backed by a ResourceHandle.
    pub fn new(
        resource: ResourceHandle,
        width: u32,
        height: u32,
        pts: Duration,
        is_opaque: bool,
    ) -> Self {
        Self {
            resource,
            width,
            height,
            pts,
            is_opaque,
            surface: None,
        }
    }
}

// ── FrameStatus (Phase 2 Event-Driven) ────────────────────────────────────────

/// The result of polling `MediaBackend::current_frame()`.
///
/// The render engine uses this to avoid unnecessary GPU work:
/// - `Ready`       → upload/bind and render the new frame.
/// - `Unchanged`   → reuse the previously bound texture.
/// - `Idle`        → static content is resting; 0 wakeups needed until an event occurs (Phase 2).
/// - `EndOfStream` → stop rendering or loop, depending on policy.
#[derive(Debug, Clone)]
pub enum FrameStatus {
    /// A new frame is available for rendering.
    Ready(Frame),
    /// The media has not advanced; the last frame is still valid.
    Unchanged,
    /// Phase 2: Static content is idle — zero polling wakeups required until state changes.
    Idle,
    /// The media stream has ended.
    EndOfStream,
}

// ── FrameSink & Negotiation (Phase 1) ─────────────────────────────────────────

/// Advertised surface kinds and pixel formats accepted by a FrameSink.
#[derive(Debug, Clone)]
pub struct SinkCapabilities {
    pub accepted_kinds: Vec<u32>,   // NATIVIS_SURFACE_KIND_*
    pub accepted_formats: Vec<u32>, // NATIVIS_FORMAT_*
}

/// Transport boundary. The runtime calls this to push frames to the platform.
pub trait FrameSink: Send + Sync {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError>;

    /// Phase 1: Negotiated capabilities advertised by the sink.
    /// Sinks advertise the surface kinds and formats they accept in preference order.
    fn capabilities(&self) -> SinkCapabilities {
        SinkCapabilities {
            accepted_kinds: vec![
                NATIVIS_SURFACE_KIND_CPU_PLANAR,
                NATIVIS_SURFACE_KIND_CPU_SHM,
                NATIVIS_SURFACE_KIND_CPU_BUFFER,
            ],
            accepted_formats: vec![NATIVIS_FORMAT_RGBA8888, NATIVIS_FORMAT_NV12],
        }
    }
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("Failed to open media source: {0}")]
    Open(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("GPU upload failed: {0}")]
    GpuUpload(String),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Surface rejected by sink: {0}")]
    SurfaceRejected(String),
}

// ── MediaBackend — frozen contract ────────────────────────────────────────────

/// The single interface all media backends must implement.
///
/// ## Contract rules (enforced by the "no layer skipping" invariant)
///
/// - `open()` is called **once** after construction. The `RhiContext` is
///   stored internally; it is **not** passed to `update()` on every frame.
/// - `update()` advances internal timing. GPU uploads happen here.
/// - `current_frame()` is cheap — it does not decode; it returns the result
///   of the last `update()`.
/// - `close()` must release all GPU resources allocated via the RHI.
/// - `supports()` is queried by `PluginRegistry` to route an `AssetPath` to
///   the correct backend — the runtime never hard-codes backend types.
pub trait MediaBackend: Send + Sync {
    /// Human-readable backend name (e.g. `"image_backend"`).
    fn name(&self) -> &'static str;

    /// The architectural classification of this source (Producer, Declarative, Embedded).
    fn source_class(&self) -> SourceClass {
        SourceClass::Producer
    }

    /// The backend can allocate resources via a context passed by the Application.
    fn open(
        &mut self,
        source: &AssetPath,
        clock: &MediaClock,
        resources: &crate::resource::ResourceManager,
    ) -> Result<(), MediaError>;

    /// Advance the media clock and upload any new decoded data to the GPU.
    ///
    /// Called once per frame from the runtime tick. Must be lightweight;
    /// heavy decoding work should happen on a background thread.
    fn update(&mut self, dt: Duration) -> Result<(), MediaError>;

    /// Return the current frame status without decoding.
    fn current_frame(&self) -> FrameStatus;

    /// Return the capabilities this backend supports.
    fn capabilities(&self) -> &[MediaCapability];

    /// Return `true` if this backend can handle the given `AssetPath`.
    ///
    /// The registry calls `supports()` on every registered backend until
    /// one returns `true`. The runtime never inspects backend types directly.
    fn supports(&self, source: &AssetPath) -> bool;

    /// Release all GPU resources and close decoders.
    fn close(&mut self);
}

