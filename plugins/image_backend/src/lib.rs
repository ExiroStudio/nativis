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
    handle: Option<nativis_core::contract::ResourceHandle>,
    pending: Option<Frame>,
    emit_on_next_update: bool,
    free_on_next_update: bool,
    /// Retained so that `update()` or `close()` can free the registered `CpuBuffer` handle.
    resources: Option<ResourceManager>,
    target_resolution: Option<(u32, u32)>,
}

impl ImageBackend {
    pub fn new() -> Self {
        Self {
            frame: None,
            handle: None,
            pending: None,
            emit_on_next_update: false,
            free_on_next_update: false,
            resources: None,
            target_resolution: None,
        }
    }

    /// Set an explicit target resolution to downscale images on load.
    pub fn with_target_resolution(mut self, width: u32, height: u32) -> Self {
        self.target_resolution = Some((width, height));
        self
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
        if let (Some(res), Some(old_handle)) = (self.resources.as_ref(), self.handle.take()) {
            res.free(old_handle);
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

        // Downscale to target monitor resolution if specified via with_target_resolution
        // or environment variable NATIVIS_TARGET_RESOLUTION (e.g. "1920x1080").
        // A 4K image downscaled to 1080p uses only ~8.3 MB instead of ~33 MB,
        // which brings total image RSS well under 30 MB.
        let target = self.target_resolution.or_else(|| {
            std::env::var("NATIVIS_TARGET_RESOLUTION").ok().and_then(|val| {
                let mut parts = val.split('x');
                let w: u32 = parts.next()?.parse().ok()?;
                let h: u32 = parts.next()?.parse().ok()?;
                Some((w, h))
            })
        });

        let dynamic_img = if let Some((max_w, max_h)) = target {
            if max_w > 0 && max_h > 0 && (dynamic_img.width() > max_w || dynamic_img.height() > max_h) {
                info!(
                    "ImageBackend: downscaling {}x{} to monitor resolution {}x{}",
                    dynamic_img.width(), dynamic_img.height(), max_w, max_h
                );
                dynamic_img.resize(max_w, max_h, image::imageops::FilterType::Triangle)
            } else {
                dynamic_img
            }
        } else {
            dynamic_img
        };

        let (width, height) = (dynamic_img.width(), dynamic_img.height());
        let has_alpha = dynamic_img.color().has_alpha();
        // into_rgba8() consumes dynamic_img without duplicating the uncompressed pixel buffer
        let rgba = dynamic_img.into_rgba8();

        // Register the raw pixel buffer as a CPU resource
        let cpu_buffer = CpuBuffer {
            data: rgba.into_raw(),
            width,
            height,
        };
        let handle = resources.register(Box::new(cpu_buffer));
        self.handle = Some(handle);

        self.frame = Some(Frame::new(
            handle,
            width,
            height,
            Duration::ZERO,
            !has_alpha,
        ));
        self.emit_on_next_update = true;
        self.free_on_next_update = false;

        info!(
            "ImageBackend successfully opened '{}' ({}x{}, alpha={})",
            source.raw_uri(),
            width,
            height,
            has_alpha
        );

        // Immediately reclaim temporary decoder arena memory back to OS.
        // malloc_trim is a glibc extension — guard against musl and other non-glibc targets
        // (Alpine Linux, static musl builds) where the symbol does not exist.
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
        // Free the CpuBuffer on the tick AFTER emitting the frame (Item 4).
        // By this tick, the sink has already processed and submitted the frame
        // (copied to POSIX SHM or uploaded to GPU swapchain). Freeing the ~33 MB
        // CpuBuffer from ResourceManager reduces daemon RSS to ~34 MB on 4K, or ~8 MB on 1080p.
        if self.free_on_next_update {
            if let (Some(res), Some(handle)) = (self.resources.as_ref(), self.handle.take()) {
                res.free(handle);
                #[cfg(all(target_os = "linux", target_env = "gnu"))]
                {
                    extern "C" {
                        fn malloc_trim(pad: usize) -> std::ffi::c_int;
                    }
                    unsafe { malloc_trim(0); }
                }
            }
            self.free_on_next_update = false;
        }

        self.pending = if self.emit_on_next_update {
            self.emit_on_next_update = false;
            self.free_on_next_update = true;
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
        // Free the registered CpuBuffer from the ResourceManager if still held.
        if let (Some(res), Some(handle)) = (self.resources.as_ref(), self.handle.take()) {
            res.free(handle);
        }

        self.frame = None;
        self.pending = None;
        self.emit_on_next_update = false;
        self.free_on_next_update = false;
        self.resources = None;

        // On Linux/glibc, aggressively return freed pages back to the OS.
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
            handle: Some(ResourceHandle(1)),
            pending: None,
            emit_on_next_update: true,
            free_on_next_update: false,
            resources: None,
            target_resolution: None,
        };

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Ready(_)));

        backend.update(Duration::ZERO).unwrap();
        assert!(matches!(backend.current_frame(), FrameStatus::Idle));
    }

}

