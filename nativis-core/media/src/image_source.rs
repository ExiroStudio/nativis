use crate::{
    frame::{VideoFrame, PixelFormat, ColorSpace, MediaState},
    source::{IMediaSource, MediaError},
};
use nativis_rhi::{RhiContext, TextureDescriptor, TextureFormat};
use tracing::{debug, info};

/// Static image media source. Loads a PNG/JPEG/WebP/etc. from disk, uploads
/// it to GPU once during `initialize()`, and returns the same `VideoFrame`
/// every call to `acquire_frame()`. Zero CPU cost per frame after init.
pub struct ImageSource {
    path:  String,
    width: u32,
    height: u32,
    state: MediaState,
    gpu_texture: Option<nativis_rhi::TextureHandle>,
    frame_in_flight: bool,
}

impl ImageSource {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            width: 0, height: 0,
            state: MediaState::Initializing,
            gpu_texture: None,
            frame_in_flight: false,
        }
    }
}

impl IMediaSource for ImageSource {
    fn name(&self) -> &str { &self.path }

    fn initialize(&mut self, rhi: &RhiContext) -> Result<(), MediaError> {
        info!("ImageSource: loading '{}'", self.path);

        let bytes = std::fs::read(&self.path)
            .map_err(|e| MediaError::Io(e))?;

        let img = image::load_from_memory(&bytes)
            .map_err(|e| MediaError::Decode(e.to_string()))?
            .into_rgba8();

        self.width  = img.width();
        self.height = img.height();

        let desc = TextureDescriptor::media_frame(self.width, self.height, TextureFormat::Rgba8Unorm);
        let handle = rhi.create_texture(&desc)
            .map_err(|e| MediaError::GpuUpload(e.to_string()))?;

        rhi.upload_texture(&handle, img.as_raw(), self.width * 4)
            .map_err(|e| MediaError::GpuUpload(e.to_string()))?;

        debug!("ImageSource: uploaded {}x{} to GPU slot {:?}", self.width, self.height, handle);

        self.gpu_texture = Some(handle);
        self.state = MediaState::Playing;
        Ok(())
    }

    fn update(&mut self, _clock_ns: u64) {
        // Static — nothing to update.
    }

    fn acquire_frame(&mut self) -> Option<VideoFrame> {
        if self.frame_in_flight { return None; }
        let texture = self.gpu_texture.clone()?;
        self.frame_in_flight = true;
        Some(VideoFrame {
            texture,
            timestamp_ns: 0,
            duration_ns:  u64::MAX, // static — never expires
            width:        self.width,
            height:       self.height,
            format:       PixelFormat::Rgba8,
            color_space:  ColorSpace::Srgb,
        })
    }

    fn release_frame(&mut self, _frame: VideoFrame) {
        self.frame_in_flight = false;
    }

    fn dimensions(&self) -> (u32, u32) { (self.width, self.height) }

    fn state(&self) -> MediaState { self.state.clone() }
}
