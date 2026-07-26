//! Frozen `MediaBackend` contract — the central interface of Nativis.
//!
//! Every media backend (image, video, shader, webview) implements this trait.
//! The runtime and render engine communicate *only* through these types.
//! No wgpu, Vulkan, or other driver types appear here.

use std::time::Duration;
use thiserror::Error;

use crate::clock::MediaClock;
use nativis_asset::AssetPath;
use nativis_rhi::{RhiContext, TextureHandle};

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

// ── RenderFrame ───────────────────────────────────────────────────────────────

/// A decoded, GPU-resident frame ready for rendering.
///
/// The render engine receives this value from `MediaBackend::current_frame()`.
/// It carries no GPU driver types — `TextureHandle` is an opaque RHI handle.
#[derive(Debug, Clone)]
pub struct RenderFrame {
    /// Handle to the GPU texture containing pixel data.
    pub texture: TextureHandle,
    /// Pixel width of the frame.
    pub width: u32,
    /// Pixel height of the frame.
    pub height: u32,
    /// Presentation timestamp within the media stream.
    pub pts: Duration,
    /// When `true`, the frame has no transparent pixels — the renderer may
    /// skip blending and use a simpler opaque blit pass.
    pub is_opaque: bool,
}

// ── FrameStatus ───────────────────────────────────────────────────────────────

/// The result of polling `MediaBackend::current_frame()`.
///
/// The render engine uses this to avoid unnecessary GPU work:
/// - `Ready`      → upload/bind and render the new frame.
/// - `Unchanged`  → reuse the previously bound texture.
/// - `EndOfStream`→ stop rendering or loop, depending on policy.
#[derive(Debug)]
pub enum FrameStatus {
    /// A new frame is available for rendering.
    Ready(RenderFrame),
    /// The media has not advanced; the last frame is still valid.
    Unchanged,
    /// The media stream has ended.
    EndOfStream,
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

    /// Open and initialize the media source.
    ///
    /// `rhi` must be stored internally for subsequent GPU uploads.
    fn open(
        &mut self,
        source: &AssetPath,
        rhi: &RhiContext,
        clock: &MediaClock,
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
