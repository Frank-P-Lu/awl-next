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
    _pad: [f32; 2],
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
}

impl BackgroundPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        from: [u8; 4],
        to: [u8; 4],
        dir: (f32, f32),
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
            from: srgba_u8_to_linear(from),
            to: srgba_u8_to_linear(to),
            dir: [dir.0, dir.1],
        }
    }

    /// Re-tint the gradient to a new world (a live theme switch). The next
    /// `prepare` uploads it.
    pub fn set_gradient(&mut self, from: [u8; 4], to: [u8; 4], dir: (f32, f32)) {
        self.from = srgba_u8_to_linear(from);
        self.to = srgba_u8_to_linear(to);
        self.dir = [dir.0, dir.1];
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
            _pad: [0.0, 0.0],
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
