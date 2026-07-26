//! `RhiContext` — the GPU context passed to media backends at open time.
//!
//! Media backends receive this once during `open()` and store it internally.
//! It provides everything needed for GPU resource creation and texture upload
//! without leaking wgpu types into the core contract.

use crate::types::{RhiError, TextureDescriptor, TextureHandle};

use std::sync::Arc;

/// Opaque GPU context given to media backends during `open()`.
///
/// Backends use this to allocate textures, upload pixel data, and free
/// resources. They must not hold a reference past `close()`.
///
/// Internally wraps `Arc<wgpu::Device>` + `Arc<wgpu::Queue>` and a resource
/// pool managed by `WgpuBackend`. Cloning is cheap (Arc).
#[derive(Clone)]
pub struct RhiContext {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
}

impl RhiContext {
    pub(crate) fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { device, queue }
    }

    /// Allocate a new GPU texture.
    pub fn create_texture(&self, desc: &TextureDescriptor) -> Result<TextureHandle, RhiError> {
        let wgpu_format = desc.format.to_wgpu()
            .ok_or_else(|| RhiError::Backend("TextureFormat not directly supported".into()))?;

        let mut usage = wgpu::TextureUsages::empty();
        if desc.usage.contains(crate::types::TextureUsage::COPY_SRC) { usage |= wgpu::TextureUsages::COPY_SRC; }
        if desc.usage.contains(crate::types::TextureUsage::COPY_DST) { usage |= wgpu::TextureUsages::COPY_DST; }
        if desc.usage.contains(crate::types::TextureUsage::SAMPLED) { usage |= wgpu::TextureUsages::TEXTURE_BINDING; }
        if desc.usage.contains(crate::types::TextureUsage::RENDER_TARGET) { usage |= wgpu::TextureUsages::RENDER_ATTACHMENT; }
        if desc.usage.contains(crate::types::TextureUsage::STORAGE) { usage |= wgpu::TextureUsages::STORAGE_BINDING; }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: desc.label.as_deref(),
            size: wgpu::Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: desc.mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage,
            view_formats: &[],
        });

        // Store handle — for simplicity in this layer, the handle wraps
        // the raw wgpu::Texture view inline via the texture registry.
        // The wgpu::TextureView is created and returned as a TextureHandle
        // that embeds the view inside an Arc for thread safety.
        let view = Arc::new(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        // Store texture to prevent it being dropped.
        let _texture = Arc::new(texture);

        Ok(TextureHandle::from_arc_view(view, _texture, desc.width, desc.height))
    }

    /// Upload raw pixel data to a GPU texture.
    pub fn upload_texture(&self, handle: &TextureHandle, data: &[u8], bytes_per_row: u32) -> Result<(), RhiError> {
        let (_view_ref, tex_ref) = handle.raw_refs()
            .ok_or(RhiError::InvalidHandle)?;

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: tex_ref,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: handle.width(),
                height: handle.height(),
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Expose the wgpu device for cases where a backend needs it directly
    /// (e.g. building a render pipeline for shader-based sources).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Expose the wgpu queue for explicit command submission.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}
