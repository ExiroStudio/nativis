//! Screen-space blit pass: renders a media texture onto a full-screen quad.



/// WGSL blit shader — full-screen triangle trick, no vertex buffer needed.
const BLIT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
};

// Full-screen triangle: vertices generated entirely in the shader.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var out: VertexOutput;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv  = uv;
    return out;
}

@group(0) @binding(0) var media_texture: texture_2d<f32>;
@group(0) @binding(1) var media_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(media_texture, media_sampler, in.uv);
}
"#;

/// A compiled screen-space blit pipeline.
pub struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    sampler:  wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
}

impl BlitPipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("blit_shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_bind_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("blit_pipeline_layout"),
            bind_group_layouts:   &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("blit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: "vs_main",

                buffers:     &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: "fs_main",

                targets:     &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:          wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face:        wgpu::FrontFace::Ccw,
                cull_mode:         None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
            cache:         None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:     Some("blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self { pipeline, sampler, bind_layout }
    }

    /// Execute the blit pass: draw `media_view` into `target_view`.
    pub fn execute(
        &self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        media_view:  &wgpu::TextureView,
        target_view: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:  Some("blit_bind_group"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: wgpu::BindingResource::TextureView(media_view),
                },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blit_encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1); // full-screen triangle
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}
