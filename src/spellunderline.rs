//! The spell-check squiggle pipeline: a wavy red underline drawn beneath each
//! misspelled word. It is MODELED ON [`crate::selection::SelectionPipeline`]
//! (instanced quad draw, per-quad instance buffer, std140-friendly globals,
//! local bytemuck shim) but the fragment shader evaluates a sine-wave SDF
//! instead of a rounded rect, so the underline reads as a real wavy squiggle.
//!
//! Each underline is given as a band rectangle `[x, y, w, h]` in PIXELS
//! (top-left origin) PLUS the wave parameters (amplitude / period / thickness),
//! all in pixels so they scale with zoom. The renderer computes the band from
//! the span's advance-aware x-range + the line baseline, mirroring the selection
//! rect builder, so the squiggle lands exactly under the misspelled glyphs.

/// Per-quad instance: the band center + half-size in pixels, the wave params,
/// and the shared RGBA color. MUST match `Instance` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct SquiggleInstance {
    center: [f32; 2],
    half: [f32; 2],
    /// x pixel of the band's LEFT edge, so the wave phase is anchored to the
    /// document (a span keeps the same crests regardless of its width).
    x0: f32,
    /// Sine amplitude (px), period (px), and stroke thickness (px).
    amp: f32,
    period: f32,
    thickness: f32,
    color: [f32; 4],
}

/// Uniform globals. MUST match `Globals` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// The squiggle render pipeline: an instanced wavy-line draw with alpha
/// blending, recorded UNDER the text (so glyphs stay crisp on top).
pub struct SpellUnderlinePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    instance_count: u32,
    /// Linear-space RGBA matching the requested sRGB squiggle color.
    color: [f32; 4],
}

/// One squiggle's geometry, in pixels. `x`,`y`,`w`,`h` is the band the wave is
/// drawn into; `amp`/`period`/`thickness` are the wave params (already zoom-
/// scaled by the caller).
#[derive(Clone, Copy, Debug)]
pub struct Squiggle {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub amp: f32,
    pub period: f32,
    pub thickness: f32,
}

impl SpellUnderlinePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, srgba: [u8; 4]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spell underline shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/spellunderline.wgsl").into(),
            ),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("spell underline globals layout"),
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
            label: Some("spell underline globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spell underline globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spell underline pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SquiggleInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
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
                // x0: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 16,
                    shader_location: 2,
                },
                // amp: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 20,
                    shader_location: 3,
                },
                // period: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 24,
                    shader_location: 4,
                },
                // thickness: f32
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 28,
                    shader_location: 5,
                },
                // color: vec4
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 6,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spell underline pipeline"),
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
                    // Straight-alpha over-blend so the soft-red squiggle
                    // composites onto the dark background and under the text.
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

        let instance_cap = 64;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spell underline instances"),
            size: (instance_cap * std::mem::size_of::<SquiggleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group,
            globals_buf,
            instance_buf,
            instance_cap,
            instance_count: 0,
            color: srgba_u8_to_linear(srgba),
        }
    }

    /// Re-tint to a new sRGBA color (for a live theme switch). The next
    /// `prepare` uploads it into the instance buffer.
    pub fn set_color(&mut self, srgba: [u8; 4]) {
        self.color = srgba_u8_to_linear(srgba);
    }

    /// How many squiggle instances the last `prepare` uploaded (0 = nothing drawn).
    /// A cheap headless assertion hook, mirroring
    /// [`crate::selection::SelectionPipeline::instance_count`] (used by the grow
    /// regression test below; no non-test caller in the shipping binary).
    #[allow(dead_code)]
    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// Build instances from per-span squiggle bands and upload them + globals.
    /// An empty slice draws nothing.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        squiggles: &[Squiggle],
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let mut instances: Vec<SquiggleInstance> = Vec::with_capacity(squiggles.len());
        for s in squiggles {
            if s.w <= 0.0 || s.h <= 0.0 {
                continue;
            }
            instances.push(SquiggleInstance {
                center: [s.x + s.w * 0.5, s.y + s.h * 0.5],
                half: [s.w * 0.5, s.h * 0.5],
                x0: s.x,
                amp: s.amp,
                period: s.period.max(1.0),
                thickness: s.thickness.max(0.5),
                color: self.color,
            });
        }

        self.upload_instances(device, queue, &instances);
        self.instance_count = instances.len() as u32;
    }

    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[SquiggleInstance],
    ) {
        if instances.len() > self.instance_cap {
            self.instance_cap = instances.len().next_power_of_two();
            // Size the new buffer to the FULL capacity (see selection.rs): a later
            // frame with count ≤ instance_cap but > the grow-time count must not
            // overrun the write_buffer below. Fixes the wgpu write_buffer overrun panic.
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spell underline instances"),
                size: (self.instance_cap * std::mem::size_of::<SquiggleInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck_lite::cast_slice(instances));
        }
    }

    /// Record the squiggle draw into an already-open render pass. Called UNDER
    /// the text (after the selection + caret, before the glyphs) so the wavy
    /// underline is visible but the text stays crisp on top.
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

/// Convert an 8-bit sRGB RGBA quad to linear-light floats for the shader (the
/// render target is sRGB, so the GPU expects linear color it re-encodes on
/// write). Alpha is linear already. Identical to selection.rs.
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
// Minimal local Pod/bytemuck shim (same approach as selection.rs / caret.rs).
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

    pub fn cast_slice<T: Pod>(s: &[T]) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(s.as_ptr() as *const u8, core::mem::size_of_val(s))
        }
    }
}

unsafe impl bytemuck_lite::Pod for SquiggleInstance {}
unsafe impl bytemuck_lite::Pod for Globals {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgba_linear_alpha_passthrough() {
        let c = srgba_u8_to_linear([0xE0, 0x52, 0x52, 0xE0]);
        // Alpha is linear (0xE0/255 ~= 0.878).
        assert!((c[3] - 0.8784314).abs() < 1e-4);
        for k in 0..3 {
            assert!(c[k] >= 0.0 && c[k] <= 1.0);
        }
    }

    /// Regression (mirrors `selection::tests::grow_sizes_buffer_to_capacity_not_contents`):
    /// growing the instance buffer must size it to the FULL power-of-two capacity,
    /// not the current contents. Otherwise a later frame whose count sits between
    /// the grow-time count and the cap overruns the buffer — the wgpu "Copy …
    /// would overrun the Destination buffer" write_buffer validation panic, on the
    /// squiggle pipeline the exact spell-heavy long-file freeze. This is the
    /// pipeline that actually crashed; until now only the color helper was tested.
    #[test]
    fn grow_sizes_buffer_to_capacity_not_contents() {
        let dq = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl spell-underline grow-test device"),
                    ..Default::default()
                })
                .await
                .ok()
        });
        let Some((device, queue)) = dq else {
            return; // no GPU adapter available — skip
        };
        let mut pipe = SpellUnderlinePipeline::new(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            [0xE0, 0x52, 0x52, 0xE0],
        );
        let squiggles = |n: usize| -> Vec<Squiggle> {
            (0..n)
                .map(|i| Squiggle {
                    x: i as f32 * 12.0,
                    y: 10.0,
                    w: 10.0,
                    h: 8.0,
                    amp: 2.0,
                    period: 10.0,
                    thickness: 1.5,
                })
                .collect()
        };
        // Grow past the initial cap (64) at 65 → cap becomes 128. With the old bug
        // the buffer was sized to 65 instances; the next frame at 100 (≤ 128 ⇒ NO
        // regrow) wrote 100 instances into a 65-slot buffer and tripped the wgpu
        // validation panic. Both prepares must upload cleanly.
        pipe.prepare(&device, &queue, 800, 600, &squiggles(65));
        pipe.prepare(&device, &queue, 800, 600, &squiggles(100));
        assert_eq!(pipe.instance_count(), 100);
    }
}
