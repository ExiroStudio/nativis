//! `image_backend` — PNG / JPG / WebP image media backend plugin for Nativis.

use std::time::Duration;
use nativis_asset::AssetPath;
use nativis_core::{
    clock::MediaClock,
    contract::{FrameStatus, MediaBackend, MediaCapability, MediaError, RenderFrame},
};
use nativis_rhi::{RhiContext, TextureDescriptor, TextureFormat};
use tracing::info;

static CAPABILITIES: &[MediaCapability] = &[MediaCapability::Alpha];

/// Media backend for loading static image files into GPU textures.
pub struct ImageBackend {
    frame: Option<RenderFrame>,
}

impl ImageBackend {
    pub fn new() -> Self {
        Self { frame: None }
    }
}

impl Default for ImageBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaBackend for ImageBackend {
    fn name(&self) -> &'static str {
        "image_backend"
    }

    fn open(
        &mut self,
        source: &AssetPath,
        rhi: &RhiContext,
        _clock: &MediaClock,
    ) -> Result<(), MediaError> {
        let local_path = source
            .to_file_path()
            .ok_or_else(|| MediaError::Open(format!("Non-local URIs not supported by ImageBackend: {}", source.raw_uri())))?;

        let bytes = std::fs::read(&local_path)
            .map_err(|e| MediaError::Open(format!("Failed to read file {}: {}", local_path.display(), e)))?;

        let dynamic_img = image::load_from_memory(&bytes)
            .map_err(|e| MediaError::Decode(format!("Image decode error for {}: {}", source.raw_uri(), e)))?;

        let (width, height) = (dynamic_img.width(), dynamic_img.height());
        let rgba = dynamic_img.to_rgba8();

        let desc = TextureDescriptor::media_frame(width, height, TextureFormat::Rgba8UnormSrgb);
        let handle = rhi
            .create_texture(&desc)
            .map_err(|e| MediaError::GpuUpload(format!("GPU texture allocation failed: {}", e)))?;

        rhi.upload_texture(&handle, rgba.as_raw(), width * 4)
            .map_err(|e| MediaError::GpuUpload(format!("GPU texture upload failed: {}", e)))?;

        let has_alpha = dynamic_img.color().has_alpha();

        self.frame = Some(RenderFrame {
            texture: handle,
            width,
            height,
            pts: Duration::ZERO,
            is_opaque: !has_alpha,
        });

        info!(
            "ImageBackend successfully opened '{}' ({}x{}, alpha={})",
            source.raw_uri(),
            width,
            height,
            has_alpha
        );

        Ok(())
    }

    fn update(&mut self, _dt: Duration) -> Result<(), MediaError> {
        Ok(())
    }

    fn current_frame(&self) -> FrameStatus {
        if let Some(ref frame) = self.frame {
            FrameStatus::Ready(frame.clone())
        } else {
            FrameStatus::Unchanged
        }
    }

    fn capabilities(&self) -> &[MediaCapability] {
        CAPABILITIES
    }

    fn supports(&self, source: &AssetPath) -> bool {
        let ext = source.extension();
        matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tga")
    }

    fn close(&mut self) {
        self.frame = None;
    }
}
