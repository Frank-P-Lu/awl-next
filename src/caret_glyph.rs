//! The glyph-silhouette ("Morph") caret pipeline: a parallel pipeline to
//! [`crate::caret::CaretPipeline`] that draws the caret as the cursor GLYPH'S
//! SHAPE filled SOLID in the accent, cross-fading between the previous and current
//! glyph as the caret glides. NO glow, NO halo, NO soft falloff — the caret is
//! eye-catching by COLOUR alone. The silhouette IS expanded by a small HARD,
//! uniform dilation (a morphological max over a ring of taps, see the shader) so
//! it reads a touch bolder than the letter, but it stays SOLID in the one accent
//! colour — a fatter version of the same letter, not a tapered glow.
//!
//! Where [`CaretPipeline`](crate::caret::CaretPipeline) rasterizes a rounded rect
//! in the shader, this pipeline samples TWO small per-glyph coverage MASKS (R8
//! alpha textures, CPU-rasterized from the same swash cache glyphon uses) and
//! paints the accent through their cross-faded union. Unlike the block caret it
//! draws OVER the document text (after the glyph pass), so the accent silhouette
//! lands exactly on the real letter and RECOLOURS it — the cursor's letter reads
//! as the accent hue rather than a black letter with a coloured ring around it.
//!
//! The renderer owns the mask rasterization (it has `font_system` + `swash_cache`)
//! and hands this pipeline the two textures + the per-instance geometry each frame.
//! Block mode is left completely untouched; this is a clean parallel pipeline.

/// A single CPU-rasterized glyph coverage mask uploaded to a small R8 texture,
/// cached by the cosmic-text [`glyphon::CacheKey`] that produced it (glyph id +
/// font id + font size + subpixel bin, so a zoom / font / world change makes a new
/// key and re-rasterizes automatically). `placement_*` are the swash placement box
/// offsets relative to the glyph PEN ORIGIN (left edge, baseline): `left` is +x
/// from the pen, `top` is the pixels ABOVE the baseline (so the box top sits at
/// baseline_y - top).
pub struct GlyphMask {
    pub key: glyphon::CacheKey,
    /// Owns the GPU texture; the `view` below is what the bind group samples, but
    /// the texture must outlive it, so this field is kept as an RAII guard.
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Placement box in pixels (swash placement): left/top offsets + size.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

impl GlyphMask {
    /// Build a mask texture from an R8 coverage bitmap (`width*height` bytes).
    pub fn from_coverage(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: glyphon::CacheKey,
        left: i32,
        top: i32,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("caret glyph mask"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if width > 0 && height > 0 && !data.is_empty() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // R8 = 1 byte/px; rows are tightly packed in the swash image.
                    bytes_per_row: Some(width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            key,
            texture,
            view,
            left,
            top,
            width,
            height,
        }
    }
}

/// Per-quad instance data. MUST match the `Instance` struct layout in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct GlyphInstance {
    /// Union pixel rect (min corner + size) the quad covers.
    rect_min: [f32; 2],
    rect_size: [f32; 2],
    /// FROM-glyph placement box (min + size) in pixels.
    from_min: [f32; 2],
    from_size: [f32; 2],
    /// TO-glyph placement box (min + size) in pixels.
    to_min: [f32; 2],
    to_size: [f32; 2],
    /// morph_t (0=from,1=to), overall alpha.
    morph_t: f32,
    alpha: f32,
    /// Linear accent color.
    color: [f32; 3],
    /// Hard same-color dilation radius in PIXELS (already zoom-scaled on the CPU).
    /// The fragment shader takes a `max` over a small ring of taps at this radius
    /// so the silhouette is a touch fatter than the letter, filled SOLID — no glow.
    dilate_px: f32,
}

/// Uniform globals. MUST match `Globals` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// The glyph-silhouette caret pipeline: a single instanced quad sampling two glyph
/// masks, drawn UNDER the text (same slot as the block caret).
pub struct CaretGlyphPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    /// Rebuilt each `prepare` (the bound mask textures change with the cursor glyph).
    bind_group: Option<wgpu::BindGroup>,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// 1x1 transparent fallback texture for an unbound "from"/"to" mask slot, so
    /// the bind group is always complete even on the first frame. Owns the texture
    /// backing `blank_view` (kept as an RAII guard; the view is what gets bound).
    #[allow(dead_code)]
    blank: wgpu::Texture,
    blank_view: wgpu::TextureView,
    instance_count: u32,
    /// Linear accent color for the silhouette.
    color: [f32; 3],
}

impl CaretGlyphPipeline {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, caret_srgb: [u8; 3]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("caret glyph shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/caret_glyph.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("caret glyph layout"),
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
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret glyph globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("caret glyph sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // 1x1 transparent fallback so an unbound mask slot still satisfies the
        // bind group (its placement box size is 0, so the shader reads 0 coverage).
        let blank = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("caret glyph blank mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &blank,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let blank_view = blank.create_view(&wgpu::TextureViewDescriptor::default());

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("caret glyph pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GlyphInstance>() as u64,
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
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 16,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 24,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 32,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 40,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 48,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 52,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 56,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 68,
                    shader_location: 9,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("caret glyph pipeline"),
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
                    // Straight-alpha over-blend, matching the block caret so the
                    // anti-aliased silhouette composites softly onto the canvas.
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

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("caret glyph instances"),
            size: std::mem::size_of::<GlyphInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group: None,
            globals_buf,
            instance_buf,
            sampler,
            blank,
            blank_view,
            instance_count: 0,
            color: crate::caret::srgb_u8_to_linear(caret_srgb),
        }
    }

    /// Re-tint the silhouette to a new sRGB accent (live theme switch).
    pub fn set_color(&mut self, caret_srgb: [u8; 3]) {
        self.color = crate::caret::srgb_u8_to_linear(caret_srgb);
    }

    /// Mark this pipeline as drawing nothing this frame (e.g. block mode active, or
    /// the cursor is on a glyphless cell where we fall back to the block caret).
    pub fn clear(&mut self) {
        self.instance_count = 0;
    }

    /// Build the single silhouette instance for this frame and upload it. The
    /// `from`/`to` masks are the leaving + arriving glyphs (either may be `None`,
    /// in which case its placement size is 0 and it contributes no coverage), and
    /// `morph_t` cross-fades between them (driven by the spring settle factor). The
    /// caret BODY position rides the spring: `from`/`to` boxes are already offset
    /// to the animated pen origin by the renderer.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        from: Option<&GlyphMask>,
        from_box: [f32; 4],
        to: Option<&GlyphMask>,
        to_box: [f32; 4],
        morph_t: f32,
        alpha: f32,
        dilate_px: f32,
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, crate::caret::bytes_of_pod(&globals));

        // Bind the actual masks (or the blank fallback). The bind group is rebuilt
        // each prepare because the bound textures change as the cursor glyph does;
        // this is one caret quad per frame, so the cost is negligible.
        let from_view = from.map(|m| &m.view).unwrap_or(&self.blank_view);
        let to_view = to.map(|m| &m.view).unwrap_or(&self.blank_view);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("caret glyph bind"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(from_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(to_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.bind_group = Some(bind_group);

        // Union rect = bounding box of both placement boxes, expanded by a small
        // margin so the silhouette's anti-aliased edge has room. The hard dilation
        // pushes coverage up to `dilate_px` OUTWARD, so the quad must grow by that
        // much (plus 1px of AA slack) or the fattened edge would be clipped.
        let margin = dilate_px.max(0.0) + 1.0;
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (present, b) in [(from.is_some(), from_box), (to.is_some(), to_box)] {
            if !present || b[2] <= 0.0 || b[3] <= 0.0 {
                continue;
            }
            min_x = min_x.min(b[0]);
            min_y = min_y.min(b[1]);
            max_x = max_x.max(b[0] + b[2]);
            max_y = max_y.max(b[1] + b[3]);
        }
        if !min_x.is_finite() {
            // No glyph at all: nothing to draw.
            self.instance_count = 0;
            return;
        }
        let rect_min = [min_x - margin, min_y - margin];
        let rect_size = [(max_x - min_x) + margin * 2.0, (max_y - min_y) + margin * 2.0];

        let inst = GlyphInstance {
            rect_min,
            rect_size,
            from_min: [from_box[0], from_box[1]],
            from_size: if from.is_some() {
                [from_box[2], from_box[3]]
            } else {
                [0.0, 0.0]
            },
            to_min: [to_box[0], to_box[1]],
            to_size: if to.is_some() {
                [to_box[2], to_box[3]]
            } else {
                [0.0, 0.0]
            },
            morph_t: morph_t.clamp(0.0, 1.0),
            alpha,
            color: self.color,
            dilate_px: dilate_px.max(0.0),
        };
        queue.write_buffer(&self.instance_buf, 0, crate::caret::bytes_of_pod(&inst));
        self.instance_count = 1;
    }

    /// Record the silhouette caret draw (after clear/selection/spell, before text).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        let Some(bg) = self.bind_group.as_ref() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..self.instance_count);
    }
}
