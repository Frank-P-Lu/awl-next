//! The selection highlight pipeline: a set of translucent GPU quads drawn UNDER
//! the caret and text, one per visible line of the active region. It mirrors the
//! structure of [`crate::caret::CaretPipeline`] (instanced quad draw, per-quad
//! instance buffer, std140-friendly globals) but is intentionally simpler: a
//! flat, soft-cornered, single-color translucent rectangle — no glow, no trail.
//!
//! Each rectangle is given as `[x, y, w, h]` in PIXELS (top-left origin); the
//! renderer computes these from the selection endpoints + scroll + zoom so the
//! highlight lands exactly behind the selected glyphs.

/// Rounded-corner radius (px) of a selection rectangle. A small radius softens
/// the block so it reads as a highlight rather than a hard inverse-video bar.
const CORNER_RADIUS: f32 = 2.5;

/// Per-quad instance: a rectangle center + half-size in pixels, plus the shared
/// RGBA color. MUST match `Instance` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct SelInstance {
    center: [f32; 2],
    half: [f32; 2],
    color: [f32; 4],
}

/// Uniform globals. MUST match `Globals` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    corner: f32,
    _pad: f32,
}

/// The selection render pipeline: an instanced translucent quad draw with alpha
/// blending over the cleared background, BEFORE the caret + text are drawn.
pub struct SelectionPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    instance_count: u32,
    /// Linear-space RGBA matching the requested sRGB selection color.
    color: [f32; 4],
}

impl SelectionPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, srgba: [u8; 4]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("selection shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/selection.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("selection globals layout"),
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
            label: Some("selection globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("selection globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("selection pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SelInstance>() as u64,
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
                // color: vec4
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("selection pipeline"),
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
                    // Straight-alpha over-blend so the translucent highlight
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
        });

        let instance_cap = 64;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selection instances"),
            size: (instance_cap * std::mem::size_of::<SelInstance>()) as u64,
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

    /// How many quad instances the last `prepare` uploaded (0 = nothing drawn). A cheap
    /// headless assertion hook for "is this summoned rect present this frame?" (used by
    /// the render tests; no non-test caller in the shipping binary).
    #[allow(dead_code)]
    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// Build instances from per-line rectangles (`[x, y, w, h]` top-left, px)
    /// and upload them + globals. An empty slice draws nothing.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rects: &[[f32; 4]],
    ) {
        self.prepare_with_color(device, queue, width, height, rects, self.color);
    }

    /// COPY PULSE: build instances exactly like [`Self::prepare`], but blend the
    /// STORED base `color` toward `peak_srgba` (a brighter tint in the SAME hue
    /// family — see `render::copy_pulse_peak_srgba`) by `(1.0 - settle)`. `settle`
    /// in `[0, 1]`: `1.0` draws EXACTLY the base color — byte-identical to
    /// `prepare` (the short-circuit below skips the blend arithmetic entirely, so
    /// there is no floating-point drift at rest either) — `0.0` draws fully
    /// `peak_srgba`. Never mutates the stored base `color`: a live theme switch's
    /// [`Self::set_color`] stays the single source of truth, and the very next
    /// settled frame (`settle >= 1.0`) reverts automatically with no extra
    /// bookkeeping on either side.
    pub fn prepare_pulsed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rects: &[[f32; 4]],
        peak_srgba: [u8; 4],
        settle: f32,
    ) {
        let settle = settle.clamp(0.0, 1.0);
        if settle >= 1.0 {
            self.prepare(device, queue, width, height, rects);
            return;
        }
        let peak = srgba_u8_to_linear(peak_srgba);
        let color = lerp4(peak, self.color, settle);
        self.prepare_with_color(device, queue, width, height, rects, color);
    }

    /// Build instances from per-quad `([x, y, w, h], srgba)` pairs — each rectangle
    /// in its OWN sRGBA colour, unlike [`Self::prepare`] which paints every rect the
    /// one stored `color`. The theme-picker SWATCHES use this: every world row's chip
    /// is a different world's palette (its ground band + accent dot), so one draw
    /// carries many colours. An empty slice draws nothing. `self.color` is untouched.
    pub fn prepare_colored(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        quads: &[([f32; 4], [u8; 4])],
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            corner: CORNER_RADIUS,
            _pad: 0.0,
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let mut instances: Vec<SelInstance> = Vec::with_capacity(quads.len());
        for (r, srgba) in quads {
            let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            instances.push(SelInstance {
                center: [x + w * 0.5, y + h * 0.5],
                half: [w * 0.5, h * 0.5],
                color: srgba_u8_to_linear(*srgba),
            });
        }

        self.upload_instances(device, queue, &instances);
        self.instance_count = instances.len() as u32;
    }

    /// The shared body of [`Self::prepare`] / [`Self::prepare_pulsed`]: build +
    /// upload instances from `rects`, tinted with the given (already-linear)
    /// `color` — NOT necessarily the stored `self.color`, so the copy-pulse blend
    /// never has to mutate persistent state to draw an ephemeral frame.
    fn prepare_with_color(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rects: &[[f32; 4]],
        color: [f32; 4],
    ) {
        let globals = Globals {
            viewport: [width as f32, height as f32],
            corner: CORNER_RADIUS,
            _pad: 0.0,
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let mut instances: Vec<SelInstance> = Vec::with_capacity(rects.len());
        for r in rects {
            let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            instances.push(SelInstance {
                center: [x + w * 0.5, y + h * 0.5],
                half: [w * 0.5, h * 0.5],
                color,
            });
        }

        self.upload_instances(device, queue, &instances);
        self.instance_count = instances.len() as u32;
    }

    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[SelInstance],
    ) {
        if instances.len() > self.instance_cap {
            self.instance_cap = instances.len().next_power_of_two();
            // Size the new buffer to the FULL capacity — NOT just the current
            // contents. A later frame whose count is ≤ instance_cap but > the
            // count at grow-time would otherwise overrun this buffer (the
            // write_buffer path below never resizes). This is the fix for the
            // wgpu "Copy … would overrun the Destination buffer" validation panic.
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("selection instances"),
                size: (self.instance_cap * std::mem::size_of::<SelInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck_lite::cast_slice(instances));
        }
    }

    /// Record the selection draw into an already-open render pass (after clear,
    /// before the caret + text).
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

/// Linear-interpolate two linear-space RGBA colors by `t` ∈ `[0, 1]` (`0` = `a`,
/// `1` = `b`) — the copy-pulse's per-channel blend. Pure; no clamping (callers
/// already clamp `t`).
fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Convert an 8-bit sRGB RGBA quad to linear-light floats for the shader (the
/// render target is sRGB, so the GPU expects linear color it re-encodes on
/// write). Alpha is linear already.
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
// Minimal local Pod/bytemuck shim (same approach as caret.rs, no extra crate).
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

unsafe impl bytemuck_lite::Pod for SelInstance {}
unsafe impl bytemuck_lite::Pod for Globals {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgba_linear_alpha_passthrough() {
        let c = srgba_u8_to_linear([0x3A, 0x6F, 0xD8, 0x52]);
        // Alpha is linear (0x52/255 ~= 0.32).
        assert!((c[3] - 0.32156864).abs() < 1e-4);
        // Channels are in [0,1].
        for k in 0..3 {
            assert!(c[k] >= 0.0 && c[k] <= 1.0);
        }
    }

    /// COPY PULSE pure decay math: `lerp4` at `t=0` is exactly `a` (the pulse's
    /// peak), at `t=1` exactly `b` (the settled base), and linear in between —
    /// the arithmetic `prepare_pulsed` blends the base color toward the peak with.
    #[test]
    fn lerp4_interpolates_linearly_between_endpoints() {
        let a = [0.0, 0.2, 1.0, 0.5];
        let b = [1.0, 0.8, 0.0, 0.1];
        let at0 = lerp4(a, b, 0.0);
        let at1 = lerp4(a, b, 1.0);
        for k in 0..4 {
            assert!((at0[k] - a[k]).abs() < 1e-6, "t=0 must be the first color");
            assert!((at1[k] - b[k]).abs() < 1e-6, "t=1 must be the second color");
        }
        let mid = lerp4(a, b, 0.5);
        for k in 0..4 {
            assert!(
                (mid[k] - (a[k] + b[k]) / 2.0).abs() < 1e-6,
                "channel {k} must be the midpoint"
            );
        }
    }

    /// Regression: growing the instance buffer must size it to the FULL
    /// power-of-two capacity, not the current contents. Otherwise a later frame
    /// whose count sits between the grow-time count and the cap overruns the
    /// buffer — the wgpu "Copy … would overrun the Destination buffer" write_buffer
    /// validation panic that froze awl on a spell-heavy long file.
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
                    label: Some("awl selection grow-test device"),
                    ..Default::default()
                })
                .await
                .ok()
        });
        let Some((device, queue)) = dq else {
            return; // no GPU adapter available — skip
        };
        let mut pipe = SelectionPipeline::new(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            [255, 255, 255, 255],
        );
        let rects = |n: usize| -> Vec<[f32; 4]> {
            (0..n).map(|i| [i as f32, 0.0, 10.0, 10.0]).collect()
        };
        // Grow past the initial cap (64) at 65 → cap becomes 128. With the old bug
        // the buffer was sized to 65; the next frame at 100 (≤ 128 ⇒ NO regrow)
        // wrote 100 instances into a 65-slot buffer and panicked.
        pipe.prepare(&device, &queue, 800, 600, &rects(65));
        pipe.prepare(&device, &queue, 800, 600, &rects(100));
        assert_eq!(pipe.instance_count(), 100);
    }
}
