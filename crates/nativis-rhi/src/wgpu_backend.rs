use crate::{backend::IRhiBackend, types::*};
use nativis_core::Handle;
use tracing::{debug, info};


// ── Internal storage for wgpu resources ──────────────────────────────────────

struct ManagedTexture {
    texture: wgpu::Texture,
    view:    wgpu::TextureView,
    desc:    TextureDescriptor,
}

struct ManagedBuffer {
    buffer: wgpu::Buffer,
}

// ── WgpuBackend ───────────────────────────────────────────────────────────────

/// Concrete RHI backend using `wgpu` (Vulkan on Linux, Metal on macOS,
/// DirectX 12 on Windows). Wraps all wgpu objects inside the handle-pool
/// pattern so higher-level code never touches raw GPU objects.
pub struct WgpuBackend {
    // wgpu core objects — instance must outlive the surface
    #[allow(dead_code)]
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter:  wgpu::Adapter,
    pub(crate) device:   wgpu::Device,
    pub(crate) queue:    wgpu::Queue,

    // Surface / swapchain
    pub(crate) surface:       wgpu::Surface<'static>,
    pub(crate) surface_config: wgpu::SurfaceConfiguration,
    pub(crate) surface_format: wgpu::TextureFormat,

    // Resource pools (generational index → resource)
    textures:  Vec<Option<ManagedTexture>>,
    tex_gens:  Vec<u32>,
    buffers:   Vec<Option<ManagedBuffer>>,
    buf_gens:  Vec<u32>,

    // Current frame swapchain texture slot (reserved slot 0)
    surface_handle: TextureHandle,

    // Current swapchain frame state
    current_surface_tex:  Option<wgpu::SurfaceTexture>,
    current_surface_view: Option<wgpu::TextureView>,
}

impl WgpuBackend {
    /// Create a backend from a raw window/display handle pair.
    ///
    /// `width` / `height` are the *physical* pixel dimensions of the surface.
    pub fn new<W>(window: &W, width: u32, height: u32) -> Result<Self, RhiError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends:  wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // SAFETY: The surface must not outlive the window. The caller (Engine)
        // is responsible for keeping the window alive as long as the backend.
        let surface = unsafe {
            instance.create_surface_unsafe(
                wgpu::SurfaceTargetUnsafe::from_window(window)
                    .map_err(|e| RhiError::SurfaceCreation(e.to_string()))?,
            )
        }.map_err(|e| RhiError::SurfaceCreation(e.to_string()))?;

        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::LowPower,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
        ).ok_or(RhiError::NoAdapter)?;

        info!("GPU adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);

        let (device, queue) = pollster::block_on(
            adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("nativis-device"),
                    ..Default::default()
                },
                None,
            )
        ).map_err(|e| RhiError::DeviceCreation(e.to_string()))?;

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps.formats.iter().copied()
            .find(|f| f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or(RhiError::SurfaceFormat)?;

        let surface_config = wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                        surface_format,
            width,
            height,
            present_mode:                  wgpu::PresentMode::Fifo,
            alpha_mode:                    caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Slot 0 is permanently reserved for the surface texture handle.
        let surface_handle = Handle::new(0, 0);
        let textures = vec![None]; // slot 0 = surface (managed separately)
        let tex_gens = vec![0u32];

        Ok(Self {
            instance, adapter, device, queue,
            surface, surface_config, surface_format,
            textures, tex_gens,
            buffers: Vec::new(), buf_gens: Vec::new(),
            surface_handle,
            current_surface_tex:  None,
            current_surface_view: None,
        })
    }

    // ── Internal helper: allocate a free texture slot ─────────────────────────
    fn alloc_texture_slot(&mut self, managed: ManagedTexture) -> TextureHandle {
        for (i, slot) in self.textures.iter_mut().enumerate().skip(1) {
            if slot.is_none() {
                let gen = self.tex_gens[i];
                *slot = Some(managed);
                return Handle::new(i as u32, gen);
            }
        }
        let index = self.textures.len() as u32;
        self.textures.push(Some(managed));
        self.tex_gens.push(0);
        Handle::new(index, 0)
    }

    fn alloc_buffer_slot(&mut self, managed: ManagedBuffer) -> BufferHandle {
        for (i, slot) in self.buffers.iter_mut().enumerate() {
            if slot.is_none() {
                let gen = self.buf_gens[i];
                *slot = Some(managed);
                return Handle::new(i as u32, gen);
            }
        }
        let index = self.buffers.len() as u32;
        self.buffers.push(Some(managed));
        self.buf_gens.push(0);
        Handle::new(index, 0)
    }

    /// Get a `wgpu::TextureView` for an engine `TextureHandle`.
    /// Returns `None` for the surface handle (slot 0) or invalid handles.
    pub fn texture_view(&self, handle: TextureHandle) -> Option<&wgpu::TextureView> {
        if handle == self.surface_handle {
            return self.current_surface_view.as_ref();
        }
        let idx = handle.index() as usize;
        self.textures.get(idx)?.as_ref().map(|m| &m.view)
    }

    pub fn device(&self) -> &wgpu::Device  { &self.device }
    pub fn queue(&self)  -> &wgpu::Queue   { &self.queue  }
    pub fn surface_texture_format(&self) -> wgpu::TextureFormat { self.surface_format }
}

// ── IRhiBackend impl ──────────────────────────────────────────────────────────

impl IRhiBackend for WgpuBackend {
    fn backend_type(&self) -> BackendType {
        match self.adapter.get_info().backend {
            wgpu::Backend::Vulkan  => BackendType::Vulkan,
            wgpu::Backend::Metal   => BackendType::Metal,
            wgpu::Backend::Dx12    => BackendType::DirectX12,
            wgpu::Backend::Gl     => BackendType::OpenGL,
            _                     => BackendType::WebGpu,
        }
    }

    fn begin_frame(&mut self) -> Result<(), RhiError> {
        let frame = self.surface.get_current_texture()
            .map_err(|e| RhiError::Backend(e.to_string()))?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.current_surface_view = Some(view);
        self.current_surface_tex  = Some(frame);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), RhiError> {
        if width == 0 || height == 0 { return Ok(()); }
        self.surface_config.width  = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        debug!("Surface resized to {}x{}", width, height);
        Ok(())
    }

    fn present(&mut self) -> Result<(), RhiError> {
        self.current_surface_view = None;
        if let Some(frame) = self.current_surface_tex.take() {
            frame.present();
        }
        Ok(())
    }

    fn create_texture(&mut self, desc: &TextureDescriptor) -> Result<TextureHandle, RhiError> {
        let wgpu_format = desc.format.to_wgpu()
            .ok_or_else(|| RhiError::Backend("TextureFormat not directly supported by wgpu".into()))?;

        let mut usage = wgpu::TextureUsages::empty();
        if desc.usage.contains(TextureUsage::COPY_SRC)      { usage |= wgpu::TextureUsages::COPY_SRC; }
        if desc.usage.contains(TextureUsage::COPY_DST)      { usage |= wgpu::TextureUsages::COPY_DST; }
        if desc.usage.contains(TextureUsage::SAMPLED)       { usage |= wgpu::TextureUsages::TEXTURE_BINDING; }
        if desc.usage.contains(TextureUsage::RENDER_TARGET) { usage |= wgpu::TextureUsages::RENDER_ATTACHMENT; }
        if desc.usage.contains(TextureUsage::STORAGE)       { usage |= wgpu::TextureUsages::STORAGE_BINDING; }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label:           desc.label.as_deref(),
            size:            wgpu::Extent3d { width: desc.width, height: desc.height, depth_or_array_layers: 1 },
            mip_level_count: desc.mip_levels,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu_format,
            usage,
            view_formats:    &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(self.alloc_texture_slot(ManagedTexture { texture, view, desc: desc.clone() }))
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        let idx = handle.index() as usize;
        if idx == 0 { return; } // never destroy surface slot
        if let Some(slot) = self.textures.get_mut(idx) {
            if slot.as_ref().map(|_| true).unwrap_or(false) {
                *slot = None;
                self.tex_gens[idx] += 1;
            }
        }
    }

    fn create_buffer(&mut self, desc: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        let usage = match desc.usage {
            BufferUsage::Vertex  => wgpu::BufferUsages::VERTEX  | wgpu::BufferUsages::COPY_DST,
            BufferUsage::Index   => wgpu::BufferUsages::INDEX   | wgpu::BufferUsages::COPY_DST,
            BufferUsage::Uniform => wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            BufferUsage::Storage => wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            BufferUsage::Staging => wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label:              desc.label.as_deref(),
            size:               desc.size,
            usage,
            mapped_at_creation: false,
        });
        Ok(self.alloc_buffer_slot(ManagedBuffer { buffer }))
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) {
        let idx = handle.index() as usize;
        if let Some(slot) = self.buffers.get_mut(idx) {
            *slot = None;
            self.buf_gens[idx] += 1;
        }
    }

    fn create_shader(&mut self, _desc: ShaderDescriptor) -> Result<ShaderHandle, RhiError> {
        // Shaders are compiled lazily by the pipeline; for now return a dummy handle.
        // In the full pipeline builder, the WGSL source is stored and compiled
        // during create_pipeline. This stub satisfies the type contract.
        Ok(Handle::new(u32::MAX - 1, 0))
    }

    fn create_pipeline(&mut self, _desc: &PipelineDescriptor) -> Result<PipelineHandle, RhiError> {
        // Full pipeline PSO creation is done inside RenderPass nodes that have
        // direct access to the WgpuBackend. This stub satisfies the type contract.
        Ok(Handle::new(u32::MAX - 1, 0))
    }

    fn upload_texture_data(&mut self, upload: TextureUpload<'_>) -> Result<(), RhiError> {
        let idx = upload.handle.index() as usize;
        let managed = self.textures.get(idx)
            .and_then(|s| s.as_ref())
            .ok_or(RhiError::InvalidHandle)?;

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture:   &managed.texture,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            upload.data,
            wgpu::ImageDataLayout {
                offset:         0,
                bytes_per_row:  Some(upload.bytes_per_row),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width:                  managed.desc.width,
                height:                 managed.desc.height,
                depth_or_array_layers:  1,
            },
        );
        Ok(())
    }

    fn write_buffer(&mut self, handle: BufferHandle, offset: u64, data: &[u8]) {
        let idx = handle.index() as usize;
        if let Some(Some(m)) = self.buffers.get(idx) {
            self.queue.write_buffer(&m.buffer, offset, data);
        }
    }

    fn surface_format(&self) -> TextureFormat {
        match self.surface_format {
            wgpu::TextureFormat::Bgra8Unorm     => TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb => TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm     => TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            _                                   => TextureFormat::Rgba8Unorm,
        }
    }

    fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    fn current_surface_texture(&self) -> TextureHandle {
        self.surface_handle
    }
}
