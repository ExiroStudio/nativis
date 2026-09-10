//! `image_backend` — PNG / JPG / WebP image media backend plugin for Nativis.

use std::time::Duration;
use nativis_asset::AssetPath;
use nativis_core::{
    clock::MediaClock,
    contract::{FrameStatus, MediaBackend, MediaCapability, MediaError, Frame},
    resource::{CpuBuffer, ResourceManager},
};
use tracing::info;

static CAPABILITIES: &[MediaCapability] = &[MediaCapability::Alpha];

/// Media backend for loading static image files into CPU buffers.
pub struct ImageBackend {
    frame: Option<Frame>,
    pending: Option<Frame>,
    emit_on_next_update: bool,
}

impl ImageBackend {
    pub fn new() -> Self {
        Self {
            frame: None,
            pending: None,
            emit_on_next_update: false,
        }
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
        _clock: &MediaClock,
        resources: &ResourceManager,
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
        let has_alpha = dynamic_img.color().has_alpha();

        // Register the raw pixel buffer as a CPU resource
        let cpu_buffer = CpuBuffer {
            data: rgba.into_raw(),
            width,
            height,
        };
        let handle = resources.register(Box::new(cpu_buffer));

        self.frame = Some(Frame {
            resource: handle,
            width,
            height,
            pts: Duration::ZERO,
            is_opaque: !has_alpha,
        });
        self.emit_on_next_update = true;

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
        self.pending = if self.emit_on_next_update {
            self.emit_on_next_update = false;
            self.frame.clone()
        } else {
            None
        };
        Ok(())
    }

    fn current_frame(&self) -> FrameStatus {
        if let Some(ref frame) = self.pending {
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
        self.pending = None;
        self.emit_on_next_update = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nativis_core::contract::ResourceHandle;

    #[test]
    fn emits_a_static_frame_once() {
        let frame = Frame {
            resource: ResourceHandle(1),
            width: 1,
            height: 1,
            pts: Duration::ZERO,
            is_opaque: true,
        };
        let mut backend = ImageBackend {
            frame: Some(frame),
            pending: None,
            emit_on_next_update: true,
        };

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Ready(_)));

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Unchanged));
    }
}
