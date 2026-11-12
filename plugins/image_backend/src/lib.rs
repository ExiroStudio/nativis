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
    /// Retained so that `close()` can free the registered `CpuBuffer` handle.
    resources: Option<ResourceManager>,
}

impl ImageBackend {
    pub fn new() -> Self {
        Self {
            frame: None,
            pending: None,
            emit_on_next_update: false,
            resources: None,
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
        // If re-opening (e.g. hot-swap), free the previously registered buffer first.
        if let (Some(res), Some(old_frame)) = (self.resources.as_ref(), self.frame.as_ref()) {
            res.free(old_frame.resource);
        }

        // Retain a clone of the ResourceManager so close() can free the handle later.
        self.resources = Some(resources.clone());

        let local_path = source
            .to_file_path()
            .ok_or_else(|| MediaError::Open(format!("Non-local URIs not supported by ImageBackend: {}", source.raw_uri())))?;

        let bytes = std::fs::read(&local_path)
            .map_err(|e| MediaError::Open(format!("Failed to read file {}: {}", local_path.display(), e)))?;

        let dynamic_img = image::load_from_memory(&bytes)
            .map_err(|e| MediaError::Decode(format!("Image decode error for {}: {}", source.raw_uri(), e)))?;
        // Drop raw compressed file bytes immediately to minimize peak memory
        drop(bytes);

        let (width, height) = (dynamic_img.width(), dynamic_img.height());
        let has_alpha = dynamic_img.color().has_alpha();
        // into_rgba8() consumes dynamic_img without duplicating the ~32 MB uncompressed pixel buffer
        let rgba = dynamic_img.into_rgba8();

        // Register the raw pixel buffer as a CPU resource
        let cpu_buffer = CpuBuffer {
            data: rgba.into_raw(),
            width,
            height,
        };
        let handle = resources.register(Box::new(cpu_buffer));

        self.frame = Some(Frame::new(
            handle,
            width,
            height,
            Duration::ZERO,
            !has_alpha,
        ));
        self.emit_on_next_update = true;

        info!(
            "ImageBackend successfully opened '{}' ({}x{}, alpha={})",
            source.raw_uri(),
            width,
            height,
            has_alpha
        );

        // Immediately reclaim temporary decoder arena memory back to OS
        #[cfg(target_os = "linux")]
        {
            extern "C" {
                fn malloc_trim(pad: usize) -> std::ffi::c_int;
            }
            unsafe { malloc_trim(0); }
        }

        Ok(())
    }


    fn source_class(&self) -> nativis_core::contract::SourceClass {
        nativis_core::contract::SourceClass::Declarative
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
            // Phase 2: Static image is idle after presenting its frame (0 wakeups/s).
            FrameStatus::Idle
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
        // Free the registered CpuBuffer from the ResourceManager before clearing the handle.
        // Without this, the pixel data (~32 MB for 4K) leaks until the ResourceManager is dropped.
        if let (Some(res), Some(frame)) = (self.resources.as_ref(), self.frame.as_ref()) {
            res.free(frame.resource);
        }

        self.frame = None;
        self.pending = None;
        self.emit_on_next_update = false;
        self.resources = None;

        // On Linux/glibc, aggressively return freed pages back to the OS.
        // malloc_trim(0) trims the heap down as far as possible.
        // size_t maps to usize on all Rust-supported platforms; we avoid adding a libc dep.
        #[cfg(target_os = "linux")]
        {
            extern "C" {
                fn malloc_trim(pad: usize) -> std::ffi::c_int;
            }
            // SAFETY: malloc_trim is a standard glibc function with no preconditions.
            unsafe { malloc_trim(0); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nativis_core::contract::ResourceHandle;

    #[test]
    fn emits_a_static_frame_once() {
        let frame = Frame::new(
            ResourceHandle(1),
            1,
            1,
            Duration::ZERO,
            true,
        );
        let mut backend = ImageBackend {
            frame: Some(frame),
            pending: None,
            emit_on_next_update: true,
            resources: None,
        };

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Ready(_)));

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Idle));
    }

}
