//! The inline-image textured-quad pipeline: draws each decoded markdown image as
//! one textured, soft-cornered GPU quad, fit to the writing column and centered in
//! its reserved tall row. A DIRECT template of [`crate::caret_glyph`] (itself the
//! codebase's complete textured-quad pipeline) — the whole delta is the texture
//! format: an `Rgba8UnormSrgb` COLOR texture sampled straight through (rather than
//! an `R8Unorm` coverage mask painted a flat accent), plus a gentle rounded-corner
//! SDF borrowed from `selection.wgsl`.
//!
//! Unlike the single-quad caret pipelines, a document may show SEVERAL images at
//! once, each with its OWN texture (and therefore its own bind group). So this
//! pipeline holds a per-image `(bind group, instance)` list, rebuilt each
//! `prepare` (mirroring the caret-glyph "rebuild the bind group each prepare"
//! pattern — the bound texture views change with what's visible), and issues one
//! draw call per image. The image TEXTURES themselves live in the decode cache
//! ([`crate::render::image_cache`], owned by the pipeline aggregator); this
//! pipeline only borrows their views to build bind groups. Drawn right AFTER the
//! syntax/wysiwyg washes and BEFORE selection, so selection / caret / a revealed
//! source line all composite OVER the image.

/// Per-quad instance data. MUST match the `Instance` struct layout in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct ImageInstance {
    /// Destination rect top-left (px).
    dst_min: [f32; 2],
    /// Destination rect size (px).
    dst_size: [f32; 2],
    /// Overall opacity.
    alpha: f32,
    /// Rounded-corner radius (px).
    corner: f32,
}

/// Uniform globals. MUST match `Globals` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// One image ready to draw this frame: its own single-instance vertex buffer and
/// the bind group binding this image's texture view.
struct ImageDraw {
    bind_group: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
}

/// One placed image the caller wants drawn this frame: the destination rect
/// (top-left + size, px), the opacity, the rounded-corner radius, and the decoded
/// texture VIEW (borrowed from the decode cache).
pub struct PlacedImage<'a> {
    pub dst: [f32; 4],
    pub alpha: f32,
    pub corner: f32,
    pub view: &'a wgpu::TextureView,
}

/// The inline-image quad pipeline: one instanced textured quad per visible image.
pub struct ImageQuadPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    globals_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Rebuilt each `prepare` — one entry per visible image this frame.
    draws: Vec<ImageDraw>,
}

impl ImageQuadPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("inline image shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/image.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("inline image layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
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

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("inline image globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("inline image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("inline image pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 16,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 20,
                    shader_location: 3,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("inline image pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Straight-alpha over-blend, matching the caret/selection quads,
                    // so a rounded edge composites softly onto the page ground.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            globals_buf,
            sampler,
            draws: Vec::new(),
        }
    }

    /// Mark this pipeline as drawing nothing this frame (feature off / no visible
    /// images / a non-markdown buffer), so nothing stale lingers. Used on the wasm
    /// park path; on native, `prepare` with an empty slice is the park.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.draws.clear();
    }

    /// How many image quads the last `prepare` uploaded (0 = nothing drawn). A cheap
    /// headless assertion hook — "is an image drawn this frame?".
    #[allow(dead_code)]
    pub fn instance_count(&self) -> u32 {
        self.draws.len() as u32
    }

    /// Build one bind group + instance per placed image and upload globals. An empty
    /// slice draws nothing (byte-identical to the feature being off).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        images: &[PlacedImage<'_>],
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, crate::caret::bytes_of_pod(&globals));

        self.draws.clear();
        for img in images {
            if img.dst[2] <= 0.0 || img.dst[3] <= 0.0 {
                continue;
            }
            let inst = ImageInstance {
                dst_min: [img.dst[0], img.dst[1]],
                dst_size: [img.dst[2], img.dst[3]],
                alpha: img.alpha,
                corner: img.corner.max(0.0),
            };
            let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("inline image instance"),
                size: std::mem::size_of::<ImageInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&instance_buf, 0, crate::caret::bytes_of_pod(&inst));
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("inline image bind"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.globals_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(img.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.draws.push(ImageDraw {
                bind_group,
                instance_buf,
            });
        }
    }

    /// Record every image quad draw (after the washes, before selection).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.draws.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        for d in &self.draws {
            pass.set_bind_group(0, &d.bind_group, &[]);
            pass.set_vertex_buffer(0, d.instance_buf.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}
