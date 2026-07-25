use nativis_rhi::TextureHandle;
use std::collections::HashMap;

/// Holds the GPU state for rendering a surface: shader, uniforms, textures.
/// A `Material` is not a `Texture` — it is the complete rendering description.
#[derive(Debug, Clone)]
pub struct Material {
    pub id:          u32,
    pub shader_wgsl: Option<String>,
    pub textures:    Vec<(u32, TextureHandle)>,  // (slot, handle)
    pub uniforms_f32: HashMap<String, f32>,
    pub uniforms_vec4: HashMap<String, [f32; 4]>,
}

impl Material {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            shader_wgsl:   None,
            textures:      Vec::new(),
            uniforms_f32:  HashMap::new(),
            uniforms_vec4: HashMap::new(),
        }
    }

    pub fn with_texture(mut self, slot: u32, handle: TextureHandle) -> Self {
        self.textures.push((slot, handle));
        self
    }

    pub fn set_float(&mut self, name: &str, v: f32) {
        self.uniforms_f32.insert(name.to_string(), v);
    }

    pub fn set_vec4(&mut self, name: &str, v: [f32; 4]) {
        self.uniforms_vec4.insert(name.to_string(), v);
    }
}

// ── Built-in BlitPass (blit a texture to the swapchain) ───────────────────────
use nativis_render_graph::{IRenderPass, PassBuilder, PassExecuteContext, RenderGraphResources};
use nativis_rhi::IRhiBackend;
use std::cell::RefCell;

struct BlitPipelineState {
    pipeline:   wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

/// The simplest possible render pass: blit a source texture onto the
/// swapchain surface using a full-screen triangle and a wgpu render pass.
/// `pipeline_state` is lazily built on first execute using `RefCell`.
pub struct BlitPass {
    pub source_texture: TextureHandle,
    pipeline_state: RefCell<Option<BlitPipelineState>>,
}

// SAFETY: BlitPass is only used from the single render thread. RefCell is not
// Sync, but execute() is never called concurrently.
unsafe impl Send for BlitPass {}
unsafe impl Sync for BlitPass {}

impl BlitPass {
    pub fn new(source_texture: TextureHandle) -> Self {
        Self {
            source_texture,
            pipeline_state: RefCell::new(None),
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        source_view: &wgpu::TextureView,
    ) -> BlitPipelineState {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("blit-bg"),
            layout:  &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("blit-layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("blit-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         "vs_main",
                buffers:             &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         "fs_main",
                targets:             &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:     wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
            cache:         None,
        });

        BlitPipelineState { pipeline, bind_group }
    }
}

impl IRenderPass for BlitPass {
    fn name(&self) -> &'static str { "BlitPass" }

    fn declare(&self, builder: &mut PassBuilder) {
        builder.write_surface();
    }

    fn execute(&self, ctx: &mut PassExecuteContext, _resources: &RenderGraphResources) {
        // Downcast IRhiBackend to WgpuBackend to access raw wgpu objects.
        // SAFETY: Only WgpuBackend is constructed in Phase 1. The pointer
        // is valid and exclusively owned by the calling Engine for this frame.
        let rhi = unsafe {
            &mut *(ctx.rhi as *mut dyn IRhiBackend as *mut nativis_rhi::WgpuBackend)
        };

        // Lazy pipeline init — safe because execute() is single-threaded.
        {
            let mut state = self.pipeline_state.borrow_mut();
            if state.is_none() {
                if let Some(source_view) = rhi.texture_view(self.source_texture) {
                    let fmt = rhi.surface_texture_format();
                    *state = Some(Self::build_pipeline(rhi.device(), fmt, source_view));
                }
            }
        }

        let state_borrow = self.pipeline_state.borrow();
        let state = match state_borrow.as_ref() {
            Some(s) => s,
            None    => return,
        };

        let surface_view = match rhi.texture_view(rhi.current_surface_texture()) {
            Some(v) => v,
            None    => return,
        };

        let mut encoder = rhi.device().create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("blit-encoder") }
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           surface_view,
                    resolve_target: None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set:      None,
                timestamp_writes:         None,
            });
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.bind_group, &[]);
            pass.draw(0..3, 0..1); // full-screen triangle
        }

        rhi.queue().submit(std::iter::once(encoder.finish()));
    }
}

// ── Built-in WGSL shader: full-screen blit ────────────────────────────────────
const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    // Full-screen triangle: 3 vertices cover the entire NDC viewport.
    var x = f32((vi << 1u) & 2u);
    var y = f32(vi & 2u);
    var out: VertexOutput;
    out.position = vec4<f32>(x * 2.0 - 1.0, -(y * 2.0 - 1.0), 0.0, 1.0);
    out.uv       = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_source, s_source, in.uv);
}
"#;
