//! The MARGIN-gradient background pipeline (PAGE MODE). Draws ONE fullscreen
//! triangle BEFORE the selection / text passes: the calm page column is left
//! untouched (the fragment outputs alpha 0 there, so the base_100 clear shows)
//! and the surrounding MARGINS carry a per-world gradient, so the page reads as
//! a clean shape floating on a styled ground (N++ figure/ground).
//!
//! It mirrors [`crate::selection::SelectionPipeline`]'s structure (std140-friendly
//! globals, a tiny local bytemuck shim, the same straight-alpha over-blend) but is
//! vertex-free: the triangle is generated from `vertex_index`, so there is no
//! instance buffer. Colors arrive as sRGB theme bytes and are converted to linear
//! here (the render target is sRGB). Static: no time uniform, so the headless
//! capture stays byte-deterministic.

/// Uniform globals. MUST match `Globals` in `shaders/background.wgsl`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    col_left: f32,
    col_w: f32,
    from: [f32; 4],
    to: [f32; 4],
    dir: [f32; 2],
    /// Procedural ground discriminant: 0=gradient, 1=dots, 2=starfield,
    /// 3=pinstripe, 4=stripes, 5=bands, 6=waves (see `Background::shader_id`).
    shader: u32,
    _pad: u32,
    /// Mark/band tint (linear rgb) + its max coverage in `a`.
    pat: [f32; 4],
    /// Extra per-ground params: `params.x` = edge/proximity flag (0/1, Dots),
    /// `params.y` = stripe/band angle in radians (Stripes, Bands), `.zw`
    /// reserved. Bands/Waves read `from`/`to`/`tint` above as their three
    /// authored TONES (not a gradient) — no new uniform slots needed.
    params: [f32; 4],
}

/// A flat, host-side descriptor of a world's [`crate::theme::Background`] — the
/// sRGB bytes + shader discriminant + the per-ground params the pipeline needs.
/// Built from `theme::background()` in render.rs (the linear conversion happens
/// here, in [`BackgroundPipeline`]).
#[derive(Clone, Copy)]
pub struct BgDesc {
    /// Gradient START endpoint (sRGB rgba bytes).
    pub from: [u8; 4],
    /// Gradient END endpoint (sRGB rgba bytes).
    pub to: [u8; 4],
    /// Gradient direction in UV space (for Stripes: derived from the angle).
    pub dir: (f32, f32),
    /// Ground discriminant (`Background::shader_id`).
    pub shader: u32,
    /// Mark/band tint (sRGB rgb bytes; inert for a plain gradient).
    pub tint: [u8; 3],
    /// Proximity-scaling flag (Dots only).
    pub edge: bool,
    /// Stripe angle in radians (Stripes only; 0 otherwise).
    pub angle: f32,
}

/// The margin-gradient render pipeline: a single fullscreen triangle alpha-blended
/// over the cleared background, before selection + text.
pub struct BackgroundPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    /// Linear-space gradient endpoints + direction, re-tinted on a theme switch.
    from: [f32; 4],
    to: [f32; 4],
    dir: [f32; 2],
    /// Procedural margin ground + its linear mark/band tint (re-set on a theme
    /// switch), plus the per-ground params (edge flag / stripe angle).
    shader: u32,
    pat: [f32; 4],
    params: [f32; 4],
}

/// Max coverage the margin pattern's marks reach (the shader multiplies the
/// per-pixel coverage by this). Kept low so the dots / stars / stripes whisper
/// and the page column stays the clear figure.
const PATTERN_MAX_COVERAGE: f32 = 0.55;

impl BackgroundPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        desc: BgDesc,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("background shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/background.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("background globals layout"),
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
            label: Some("background globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("background globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("background pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Straight-alpha over-blend: the margins composite onto the
                    // base_100 clear, the page (alpha 0) leaves it untouched.
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
            bind_group,
            globals_buf,
            from: srgba_u8_to_linear(desc.from),
            to: srgba_u8_to_linear(desc.to),
            dir: [desc.dir.0, desc.dir.1],
            shader: desc.shader,
            pat: pattern_tint(desc.tint),
            params: ground_params(desc.edge, desc.angle),
        }
    }

    /// Re-tint the gradient + ground to a new world (a live theme switch). The
    /// next `prepare` uploads it.
    pub fn set_gradient(&mut self, desc: BgDesc) {
        self.from = srgba_u8_to_linear(desc.from);
        self.to = srgba_u8_to_linear(desc.to);
        self.dir = [desc.dir.0, desc.dir.1];
        self.shader = desc.shader;
        self.pat = pattern_tint(desc.tint);
        self.params = ground_params(desc.edge, desc.angle);
    }

    /// Upload the per-frame globals: the viewport + the page column rect (in
    /// physical pixels). When page mode is OFF the caller passes `col_w == width`
    /// so the column covers the whole canvas and the margins vanish.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        col_left: f32,
        col_w: f32,
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            col_left,
            col_w,
            from: self.from,
            to: self.to,
            dir: self.dir,
            shader: self.shader,
            _pad: 0,
            pat: self.pat,
            params: self.params,
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));
    }

    /// Record the fullscreen-triangle draw into an open render pass, FIRST (right
    /// after the clear, before selection + text).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Convert an 8-bit sRGB RGBA quad to linear-light floats for the shader (the
/// render target is sRGB, so the GPU expects linear color it re-encodes on
/// write). Alpha is linear already. Same as the selection pipeline's converter.
fn srgba_u8_to_linear(c: [u8; 4]) -> [f32; 4] {
    fn ch(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [ch(c[0]), ch(c[1]), ch(c[2]), c[3] as f32 / 255.0]
}

/// Convert an opaque 8-bit sRGB pattern tint to linear rgb + bake the max
/// coverage into `a` (the shader multiplies its per-pixel coverage by this).
fn pattern_tint(c: [u8; 3]) -> [f32; 4] {
    let lin = srgba_u8_to_linear([c[0], c[1], c[2], 0xFF]);
    [lin[0], lin[1], lin[2], PATTERN_MAX_COVERAGE]
}

/// Pack the per-ground params the shader reads: `x` = the Dots proximity flag
/// (0/1), `y` = the Stripes angle in radians, `zw` reserved. For every UNCHANGED
/// ground both are 0, so the shader takes its exact original code path (a
/// byte-identical render).
fn ground_params(edge: bool, angle: f32) -> [f32; 4] {
    [if edge { 1.0 } else { 0.0 }, angle, 0.0, 0.0]
}

// ---------------------------------------------------------------------------
// Minimal local Pod/bytemuck shim (same approach as selection.rs, no extra crate).
// ---------------------------------------------------------------------------
mod bytemuck_lite {
    /// Marker for types safe to reinterpret as bytes.
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

unsafe impl bytemuck_lite::Pod for Globals {}
