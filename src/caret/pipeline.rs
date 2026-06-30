//! CARET GPU PIPELINE — the wgpu render pipeline + instance buffer that emits the
//! single rounded-rect caret quad (its size + corner radius carrying the morphed
//! shape, rotated onto the travel axis). Plus the sRGB→linear tint helper and the
//! tiny inline Pod/bytemuck shim, both shared with the glyph-silhouette caret
//! pipeline. Lifted out of `caret.rs` VERBATIM and re-exported from `caret`, so
//! `caret::CaretPipeline` / `caret::srgb_u8_to_linear` / `caret::bytes_of_pod` keep
//! resolving. (The `include_str!` shader path gains one `../` for the new depth.)

// ---------------------------------------------------------------------------
// GPU pipeline
// ---------------------------------------------------------------------------

/// Per-quad instance data. MUST match the `Instance` struct layout in the WGSL.
/// `Pod` is implemented manually below (no bytemuck dependency).
#[repr(C)]
#[derive(Clone, Copy)]
struct CaretInstance {
    /// Center of the caret rect, in pixels.
    center: [f32; 2],
    /// Half-size (w/2, h/2) of the rounded rect, in pixels. This carries the
    /// morphed shape: a tall, advance-wide half-extent is the resting roundish
    /// square; a long, very-short half-extent is the moving trailing streak.
    /// (Named `half_size` to mirror the WGSL field, which cannot be `half` — a
    /// reserved Metal type name.)
    half_size: [f32; 2],
    /// Per-instance rounded-rect corner radius (px). Carries the corner morph:
    /// large at rest (soft roundish square), small in motion (clean bar streak).
    corner: f32,
    /// Overall alpha multiplier.
    alpha: f32,
    /// Linear amber color.
    color: [f32; 3],
    /// Unit travel AXIS (cos, sin) the quad is rotated onto, so the in-motion
    /// streak is a DIRECT line along the real travel vector (diagonal included),
    /// not axis-snapped. `(1, 0)` = upright/unrotated (the resting block, the
    /// horizontal underline, the space bar, the I-beam) — byte-identical to before.
    axis: [f32; 2],
    /// Pad to keep the struct 16-byte friendly for the vertex buffer stride.
    _pad: [f32; 2],
}

/// Uniform globals. MUST match `Globals` in the WGSL. Only the viewport is needed
/// now (the corner radius is per-instance so rest vs. motion can differ).
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// The caret render pipeline: a single instanced quad with alpha blending, drawn
/// UNDER the text (the underline sits below the glyphs).
pub struct CaretPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_count: u32,
    /// Linear-space amber matching the glyphon CARET color, for the shader.
    color: [f32; 3],
}

/// The per-instance vertex attribute table — MUST match `CaretInstance`'s field
/// order + the WGSL `Instance` struct. Pulled out as a named `'static` const so
/// [`CaretPipeline::new`] reads as a short orchestrator and the attribute layout
/// is one auditable unit; the bodies are the inline attributes lifted VERBATIM.
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 6] = [
    // center: vec2
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    // half: vec2
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    // corner: f32
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 16,
        shader_location: 2,
    },
    // alpha: f32
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 20,
        shader_location: 3,
    },
    // color: vec3
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 24,
        shader_location: 4,
    },
    // axis: vec2 (travel direction the quad rotates onto)
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 36,
        shader_location: 5,
    },
];

/// Build the caret render pipeline — a single instanced quad with straight-alpha
/// over-blending, drawn UNDER the text. The descriptor (vertex/fragment/blend/
/// primitive state) is lifted VERBATIM out of [`CaretPipeline::new`] as a named
/// seam; it produces the identical pipeline object, so the drawn caret is
/// byte-identical.
fn build_render_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    instance_layout: wgpu::VertexBufferLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("caret pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[instance_layout],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // Standard straight-alpha over-blend so the anti-aliased edge
                // composites softly onto the dark background.
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
    })
}

impl CaretPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, caret_srgb: [u8; 3]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("caret shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/caret.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("caret globals layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("caret globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("caret pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CaretInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRS,
        };

        let pipeline =
            build_render_pipeline(device, format, &shader, &pipeline_layout, instance_layout);

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret instances"),
            size: std::mem::size_of::<CaretInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group,
            globals_buf,
            instance_buf,
            instance_count: 0,
            color: srgb_u8_to_linear(caret_srgb),
        }
    }

    /// Re-tint the caret to a new sRGB color (for a live theme switch). The next
    /// `prepare` uploads it into the instance buffer.
    pub fn set_color(&mut self, caret_srgb: [u8; 3]) {
        self.color = srgb_u8_to_linear(caret_srgb);
    }

    /// Build the single caret instance and upload globals + instance.
    ///
    /// `center_x`/`center_y` are the caret rect CENTER in pixels (the renderer
    /// computes this from the glyph cell + the morphed width). `rect_w`/`rect_h`
    /// are the already-morphed rect dimensions (advance-wide roundish square when
    /// settled, long thin streak when moving) and `corner` the already-morphed
    /// rounded-rect corner radius (large at rest, small in motion). The whole
    /// morph is done by the renderer (it knows the advance, the settle factor and
    /// the spring velocity); this stage just draws what it's handed.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
    ) {
        // Fully-opaque, UPRIGHT caret (resting block / space bar / panel): axis
        // (1,0) leaves the quad unrotated, byte-identical to the pre-axis path.
        self.prepare_axis(
            queue, width, height, center_x, center_y, rect_w, rect_h, corner, 1.0, 1.0, 0.0,
        );
    }

    /// Like [`Self::prepare`] but with an explicit unit travel `axis` `(ax, ay)`
    /// the quad rotates onto, so the in-motion streak is a direct line along the
    /// real travel vector (diagonal included). `(1, 0)` is upright/unrotated.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_directed(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
        ax: f32,
        ay: f32,
    ) {
        self.prepare_axis(
            queue, width, height, center_x, center_y, rect_w, rect_h, corner, 1.0, ax, ay,
        );
    }

    /// The single instance upload, with both an `alpha` multiplier and a unit
    /// travel `axis`. All the other `prepare*` helpers funnel here.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_axis(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        center_x: f32,
        center_y: f32,
        rect_w: f32,
        rect_h: f32,
        corner: f32,
        alpha: f32,
        ax: f32,
        ay: f32,
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let inst = CaretInstance {
            center: [center_x, center_y],
            half_size: [rect_w * 0.5, rect_h * 0.5],
            corner,
            alpha,
            color: self.color,
            axis: [ax, ay],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.instance_buf, 0, bytemuck_lite::bytes_of(&inst));
        self.instance_count = 1;
    }

    /// Suppress the block caret for this frame (no instances), so when MORPH mode
    /// draws the glyph-silhouette caret instead the block quad never also paints.
    pub fn prepare_empty(&mut self) {
        self.instance_count = 0;
    }

    /// Record the caret draw into an already-open render pass (after clear,
    /// before text).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..self.instance_count);
    }
}

/// Convert an 8-bit sRGB channel triple to linear-light floats for the shader.
/// The render target is sRGB, so the GPU expects linear color which it encodes
/// back to sRGB on write — this keeps the amber hue matching the glyphon caret.
/// Shared with the glyph-silhouette caret pipeline so both carets tint identically.
pub fn srgb_u8_to_linear(c: [u8; 3]) -> [f32; 3] {
    fn ch(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [ch(c[0]), ch(c[1]), ch(c[2])]
}

// ---------------------------------------------------------------------------
// Minimal local Pod/bytemuck shim (no extra crate dependency).
// ---------------------------------------------------------------------------

/// A tiny inline replacement for the parts of `bytemuck` we use, so we don't add
/// a dependency. SAFETY: only implemented for the `#[repr(C)]` plain-old-data
/// structs above, which contain only f32 fields and no padding-sensitive layout.
mod bytemuck_lite {
    /// Marker for types that are safe to reinterpret as bytes.
    ///
    /// # Safety
    /// Implementors must be `#[repr(C)]`, contain no padding, and consist only
    /// of plain-old-data fields (here: f32 arrays/scalars).
    pub unsafe trait Pod: Copy + 'static {}

    pub fn bytes_of<T: Pod>(t: &T) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((t as *const T) as *const u8, core::mem::size_of::<T>())
        }
    }
}

unsafe impl bytemuck_lite::Pod for CaretInstance {}
unsafe impl bytemuck_lite::Pod for Globals {}

/// Reinterpret a `#[repr(C)]` plain-old-data value as bytes, for uploading to a
/// GPU buffer. Shared with the glyph-silhouette caret pipeline.
///
/// # Safety
/// `T` must be `#[repr(C)]`, contain no padding-sensitive layout, and consist only
/// of plain-old-data fields (f32 arrays/scalars). The caret pipelines' instance /
/// globals structs satisfy this.
pub fn bytes_of_pod<T: Copy + 'static>(t: &T) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts((t as *const T) as *const u8, core::mem::size_of::<T>())
    }
}
