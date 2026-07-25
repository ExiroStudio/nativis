use crate::frame::{VideoFrame, MediaState};
use nativis_rhi::IRhiBackend;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("Initialization failed: {0}")]
    Init(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("GPU upload failed: {0}")]
    GpuUpload(String),
}

/// The core media abstraction contract. Every media type — static image, video
/// file, shader framebuffer, AI frame generator, camera stream — implements
/// this trait and the renderer interacts only with `VideoFrame`/`TextureHandle`.
///
/// The renderer NEVER knows where a frame comes from.
pub trait IMediaSource: Send + Sync {
    /// Human-readable name for debugging and logging.
    fn name(&self) -> &str;

    /// One-time GPU resource allocation. Called on the render thread before the
    /// main loop. The `rhi` reference allows creating GPU textures.
    fn initialize(&mut self, rhi: &mut dyn IRhiBackend) -> Result<(), MediaError>;

    /// Advance internal state against the master clock. Called every frame in
    /// the Media Update phase, before `acquire_frame()`.
    fn update(&mut self, clock_ns: u64);

    /// Returns the latest ready frame, or `None` if not yet available.
    /// The caller **must** call `release_frame()` after use.
    fn acquire_frame(&mut self) -> Option<VideoFrame>;

    /// Return the frame's GPU resources to the source's internal pool.
    fn release_frame(&mut self, frame: VideoFrame);

    /// Physical dimensions of the media content.
    fn dimensions(&self) -> (u32, u32);

    fn state(&self) -> MediaState;

    // ── Optional playback control (no-op defaults for static sources) ────────
    fn play(&mut self)  {}
    fn pause(&mut self) {}
    fn seek(&mut self, _ns: u64) {}
    fn set_looping(&mut self, _looping: bool) {}
}
