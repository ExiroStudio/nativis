use std::ptr::NonNull;
use anyhow::{Context, Result};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};

pub struct WaylandSurfaceHandle {
    pub display_ptr: *mut std::ffi::c_void,
    pub surface_ptr: *mut std::ffi::c_void,
}

unsafe impl Send for WaylandSurfaceHandle {}
unsafe impl Sync for WaylandSurfaceHandle {}

impl HasDisplayHandle for WaylandSurfaceHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let handle = WaylandDisplayHandle::new(
            NonNull::new(self.display_ptr).ok_or(HandleError::Unavailable)?,
        );
        unsafe { Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle))) }
    }
}

impl HasWindowHandle for WaylandSurfaceHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = WaylandWindowHandle::new(
            NonNull::new(self.surface_ptr).ok_or(HandleError::Unavailable)?,
        );
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle))) }
    }
}

const NV12_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var out: VertexOutput;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y = textureSample(y_tex, samp, in.uv).r;
    let uv = textureSample(uv_tex, samp, in.uv).rg - vec2<f32>(0.5, 0.5);
    
    // BT.601 color matrix
    let r = y + 1.402 * uv.y;
    let g = y - 0.344136 * uv.x - 0.714136 * uv.y;
    let b = y + 1.772 * uv.x;
    return vec4<f32>(r, g, b, 1.0);
}
"#;

const RGBA_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var out: VertexOutput;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var rgba_tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(rgba_tex, samp, in.uv);
}
"#;

pub struct WaylandWgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    sampler: wgpu::Sampler,

    // NV12 pipeline & resources
    nv12_pipeline: wgpu::RenderPipeline,
    nv12_layout: wgpu::BindGroupLayout,
    nv12_resources: Option<Nv12GpuState>,

    // RGBA pipeline & resources
    rgba_pipeline: wgpu::RenderPipeline,
    rgba_layout: wgpu::BindGroupLayout,
    rgba_resources: Option<RgbaGpuState>,
}

struct Nv12GpuState {
    width: u32,
    height: u32,
    y_tex: wgpu::Texture,
    uv_tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct RgbaGpuState {
    width: u32,
    height: u32,
    tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

impl WaylandWgpuRenderer {
    pub fn new(handle: WaylandSurfaceHandle, width: u32, height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: handle.display_handle()?.as_raw(),
                raw_window_handle: handle.window_handle()?.as_raw(),
            })
        }.context("Failed to create WGPU surface for Wayland")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })).context("No suitable GPU adapter found")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Nativis Wayland Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        )).context("Failed to create WGPU device")?;

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps.formats.iter().copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Wayland Bilinear Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // 1. Compile NV12 Pipeline
        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NV12 Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(NV12_SHADER.into()),
        });

        let nv12_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NV12 Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let nv12_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NV12 Pipeline Layout"),
            bind_group_layouts: &[&nv12_layout],
            push_constant_ranges: &[],
        });

        let nv12_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("NV12 Render Pipeline"),
            layout: Some(&nv12_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &nv12_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &nv12_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 2. Compile RGBA Pipeline
        let rgba_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RGBA Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(RGBA_SHADER.into()),
        });

        let rgba_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RGBA Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let rgba_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RGBA Pipeline Layout"),
            bind_group_layouts: &[&rgba_layout],
            push_constant_ranges: &[],
        });

        let rgba_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RGBA Render Pipeline"),
            layout: Some(&rgba_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rgba_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &rgba_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            surface,
            config,
            sampler,
            nv12_pipeline,
            nv12_layout,
            nv12_resources: None,
            rgba_pipeline,
            rgba_layout,
            rgba_resources: None,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 && (self.config.width != width || self.config.height != height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn draw_nv12(
        &mut self,
        y_data: &[u8],
        y_stride: u32,
        y_w: u32,
        y_h: u32,
        uv_data: &[u8],
        uv_stride: u32,
        uv_w: u32,
        uv_h: u32,
    ) -> Result<()> {
        let needs_realloc = match &self.nv12_resources {
            Some(res) => res.width != y_w || res.height != y_h,
            None => true,
        };

        if needs_realloc {
            let y_tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("NV12 Y Texture"),
                size: wgpu::Extent3d {
                    width: y_w,
                    height: y_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let uv_tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("NV12 UV Texture"),
                size: wgpu::Extent3d {
                    width: uv_w,
                    height: uv_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let y_view = y_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let uv_view = uv_tex.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("NV12 Bind Group"),
                layout: &self.nv12_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&y_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&uv_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.nv12_resources = Some(Nv12GpuState {
                width: y_w,
                height: y_h,
                y_tex,
                uv_tex,
                bind_group,
            });
        }

        let res = self.nv12_resources.as_ref().unwrap();

        // Upload Y plane
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &res.y_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            y_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(y_stride),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: y_w,
                height: y_h,
                depth_or_array_layers: 1,
            },
        );

        // Upload UV plane
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &res.uv_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            uv_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(uv_stride),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: uv_w,
                height: uv_h,
                depth_or_array_layers: 1,
            },
        );

        // Render pass to surface
        let output = self.surface.get_current_texture()
            .context("Failed to acquire next swapchain texture")?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Wayland Render Encoder"),
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wayland NV12 Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.nv12_pipeline);
            rpass.set_bind_group(0, &res.bind_group, &[]);
            rpass.draw(0..3, 0..1); // Full-screen triangle
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();

        Ok(())
    }

    pub fn draw_rgba(&mut self, data: &[u8], width: u32, height: u32) -> Result<()> {
        let needs_realloc = match &self.rgba_resources {
            Some(res) => res.width != width || res.height != height,
            None => true,
        };

        if needs_realloc {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("RGBA Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RGBA Bind Group"),
                layout: &self.rgba_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.rgba_resources = Some(RgbaGpuState {
                width,
                height,
                tex,
                bind_group,
            });
        }

        let res = self.rgba_resources.as_ref().unwrap();

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &res.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let output = self.surface.get_current_texture()
            .context("Failed to acquire next swapchain texture")?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Wayland RGBA Render Encoder"),
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wayland RGBA Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.rgba_pipeline);
            rpass.set_bind_group(0, &res.bind_group, &[]);
            rpass.draw(0..3, 0..1); // Full-screen triangle
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();

        Ok(())
    }
}
