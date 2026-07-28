//! `WgpuBackend` — the concrete wgpu implementation of `IRhiBackend`.
//!
//! All wgpu types are confined to this file. Higher layers never see them.

use std::sync::Arc;
use tracing::info;

use crate::{
    backend::IRhiBackend,
    context::RhiContext,
    types::*,
};

/// wgpu-backed GPU context. Implements `IRhiBackend`.
pub struct WgpuBackend {
    #[allow(dead_code)]
    instance: wgpu::Instance,
    device:   Arc<wgpu::Device>,
    queue:    Arc<wgpu::Queue>,
    surface:  wgpu::Surface<'static>,
    config:   wgpu::SurfaceConfiguration,
    format:   wgpu::TextureFormat,

    // Current frame surface texture (acquired by begin_frame, consumed by present)
    current_frame: Option<wgpu::SurfaceTexture>,
    current_view:  Option<Arc<wgpu::TextureView>>,
    current_handle: Option<TextureHandle>,
    surface_w: u32,
    surface_h: u32,
}

impl WgpuBackend {
    /// Initialize the GPU context from a raw window handle.
    pub fn new<W>(window: &W, width: u32, height: u32) -> Result<Self, RhiError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = unsafe {
            instance.create_surface_unsafe(
                wgpu::SurfaceTargetUnsafe::from_window(window)
                    .map_err(|e| RhiError::SurfaceCreation(e.to_string()))?,
            )
        }.map_err(|e| RhiError::SurfaceCreation(e.to_string()))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::LowPower,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        })).ok_or(RhiError::NoAdapter)?;

        info!("GPU adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nativis"),
                ..Default::default()
            },
            None,
        )).map_err(|e| RhiError::DeviceCreation(e.to_string()))?;

        let device = Arc::new(device);
        let queue  = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied()
            .find(|f| f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or(RhiError::SurfaceFormat)?;

        let config = wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode:                  wgpu::PresentMode::Fifo,
            alpha_mode:                    caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            format,
            current_frame: None,
            current_view: None,
            current_handle: None,
            surface_w: width,
            surface_h: height,
        })
    }
}

impl IRhiBackend for WgpuBackend {
    fn backend_type(&self) -> BackendType {
        // wgpu selects the best available backend per platform.
        BackendType::WebGpu
    }

    fn begin_frame(&mut self) -> Result<(), RhiError> {
        let frame = self.surface
            .get_current_texture()
            .map_err(|e| RhiError::Backend(e.to_string()))?;

        let view = Arc::new(frame.texture.create_view(&wgpu::TextureViewDescriptor::default()));

        // Build a TextureHandle that points at the surface texture view.
        // The surface texture itself is owned by `current_frame`; the Arc<Texture>
        // wraps a dummy — the real resource is the SurfaceTexture.
        // We keep a separate Arc for the view so the handle stays valid.
        let _tex_arc = Arc::new(frame.texture.create_view(&wgpu::TextureViewDescriptor::default()));
        // The actual wgpu::Texture is inside SurfaceTexture and cannot be moved
        // independently. We expose the view through the handle; the texture
        // lives in current_frame until present().
        //
        // For the surface handle we construct it differently — passing a
        // fake Arc<wgpu::Texture> is not needed since we never call
        // raw_refs() on the surface handle from media code.
        // We use a sentinel texture by re-using the already-created view.
        let handle = TextureHandle::from_arc_view(
            view.clone(),
            // We need an Arc<wgpu::Texture> — create a 1×1 dummy to satisfy
            // the type. The real surface texture is managed via current_frame.
            Arc::new(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("surface_sentinel"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })),
            self.surface_w,
            self.surface_h,
        );

        self.current_view   = Some(view);
        self.current_handle = Some(handle);
        self.current_frame  = Some(frame);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), RhiError> {
        if width == 0 || height == 0 { return Ok(()); }
        self.config.width  = width;
        self.config.height = height;
        self.surface_w = width;
        self.surface_h = height;
        self.surface.configure(&self.device, &self.config);
        Ok(())
    }

    fn present(&mut self) -> Result<(), RhiError> {
        self.current_handle = None;
        self.current_view   = None;
        if let Some(frame) = self.current_frame.take() {
            frame.present();
        }
        Ok(())
    }

    fn surface_size(&self) -> (u32, u32) {
        (self.surface_w, self.surface_h)
    }

    fn surface_format(&self) -> TextureFormat {
        match self.format {
            wgpu::TextureFormat::Bgra8Unorm     => TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb => TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm     => TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            _                                   => TextureFormat::Rgba8Unorm,
        }
    }

    fn surface_texture(&self) -> &TextureHandle {
        self.current_handle.as_ref().expect("begin_frame() not called before surface_texture()")
    }

    fn rhi_context(&self) -> RhiContext {
        RhiContext::new(Arc::clone(&self.device), Arc::clone(&self.queue))
    }
}
