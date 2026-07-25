use nativis_rhi::TextureHandle;

/// Pixel memory layout of a decoded video frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Nv12,   // Y plane + interleaved UV — hardware decoder output
    P010,   // 10-bit NV12 for HDR / 10-bit content
}

/// Color space and transfer characteristics of the decoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    Bt709,   // HDTV — most SDR video
    Bt2020,  // UHDTV / HDR10
    LinearF, // Already linearised floating-point (e.g. EXR frames)
}

/// Current playback state of a media source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaState {
    Initializing,
    Playing,
    Paused,
    Buffering,
    Ended,
    Error(String),
}

/// A decoded media frame ready for GPU rendering. The `texture` field holds a
/// handle into the `IRhiBackend`'s resource pool — it must be released via
/// `IMediaSource::release_frame()` after the render pass has consumed it.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub texture:      TextureHandle,
    pub timestamp_ns: u64,
    pub duration_ns:  u64,
    pub width:        u32,
    pub height:       u32,
    pub format:       PixelFormat,
    pub color_space:  ColorSpace,
}
