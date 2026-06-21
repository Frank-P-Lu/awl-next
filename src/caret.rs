//! The animated text caret: spring physics + a motion-driven shape morph, plus
//! the wgpu pipeline that draws the caret as a single GPU quad.
//!
//! The caret has TWO states that the spring morphs between, both driven by the
//! same settle/velocity factor:
//!   * AT REST — a "roundish square": a rounded rectangle sitting ON the current
//!     character, full glyph-advance wide (full-width for CJK) and most of the
//!     line's glyph height tall, with clearly soft corners. Amber, no glow; the
//!     glyph renders on top so the letter stays legible.
//!   * IN MOTION — a "trailing underline": as the caret leaves a character it
//!     DROPS DOWN to the baseline/underline level and morphs into a horizontal
//!     streak that TRAILS behind the leading edge in the direction of travel. The
//!     faster it moves, the LONGER the streak; as it decelerates onto the target
//!     it shortens, rises back up off the line, and re-forms into the rounded
//!     square on the destination glyph.
//!
//! So during a move there are TWO simultaneous morphs — vertical (char-cell level
//! ⇄ baseline level) and shape (rounded square ⇄ stretched trailing underline) —
//! both keyed off `settle_factor()` (≈1 = rounded square on the char; ≈0 / high
//! speed = long trailing underline on the line). The streak length additionally
//! scales with the spring's horizontal velocity.
//!
//! The module is split in two:
//!   * [`CaretAnim`] — pure logic (spring integration + a settle factor derived
//!     from distance-to-target and speed). No GPU, no winit, no clock; the caller
//!     supplies `dt`. This makes the overshoot/settle behaviour unit-testable.
//!   * [`CaretPipeline`] — the wgpu render pipeline + instance buffer. It emits a
//!     SINGLE rounded-rect quad whose size + corner radius carry the morphed
//!     shape; the renderer computes that geometry from the settle factor + the
//!     spring velocity + the glyph advance.

// ---------------------------------------------------------------------------
// Tunable constants (documented in the return summary).
// ---------------------------------------------------------------------------

/// Spring stiffness `k` in `accel = k*(target-pos) - c*vel`. With DAMPING below
/// this gives ωn = √k ≈ 37.4 rad/s and damping ratio ζ ≈ 0.735 — lightly
/// underdamped: a small overshoot, settling to rest in ~140-160 ms.
pub const STIFFNESS: f32 = 1400.0;
/// Spring damping `c` for a LONG jump — the springy end of the distance-aware
/// band. See STIFFNESS for the resulting ζ ≈ 0.735 (the overshoot that reads as
/// life on a big cross-screen move). Short hops use a higher, near-critical
/// damping (see [`SMALL_MOVE_DAMPING`]); the actual `c` used each move is
/// interpolated between the two by [`CaretAnim::move_damping`].
pub const DAMPING: f32 = 55.0;

/// Spring damping `c` for a TINY hop (≤ [`SMALL_MOVE_ADV`] glyph-advances). At
/// k = STIFFNESS this is ζ = c/(2√k) ≈ 1.07 — just past critical, so a single
/// keystroke settles with ZERO overshoot and rapid typing never strobes. Big
/// jumps ease back down to the springy [`DAMPING`].
pub const SMALL_MOVE_DAMPING: f32 = 80.0;

/// Move distance (in glyph-advances) at/below which a move is "tiny" and uses
/// the fully-damped [`SMALL_MOVE_DAMPING`] (no overshoot).
const SMALL_MOVE_ADV: f32 = 1.5;
/// Move distance (in glyph-advances) at/above which a move is "big" and uses the
/// springy [`DAMPING`] (keeps its overshoot). Between the two the damping eases
/// (smoothstep) from one to the other.
const LARGE_MOVE_ADV: f32 = 8.0;

/// Settle thresholds: once the caret is within this many pixels of target AND
/// moving slower than this many px/s, we snap and stop animating (idle = 0% CPU).
pub const POS_EPSILON: f32 = 0.35;
pub const VEL_EPSILON: f32 = 6.0;

/// Max physics sub-step (s). Long frames (e.g. after a stall) are split so the
/// explicit Euler integration stays stable and deterministic-ish.
const MAX_SUBSTEP: f32 = 1.0 / 240.0;

/// Shape-morph tuning. The caret's width is `lerp(dot, underline, settle)` where
/// `settle` ∈ [0,1] is computed from how far the caret is from its target and how
/// fast it is moving. These two scales set how quickly the shape re-forms: the
/// underline is fully re-formed once the caret is within ~`SETTLE_DIST_SCALE` px
/// of the target and slower than ~`SETTLE_VEL_SCALE` px/s.
///
/// `SETTLE_VEL_SCALE` dominates mid-glide (the spring is fast there), so the
/// caret reads as a dot for most of the travel and only blooms back to the
/// underline as it decelerates onto the destination glyph.
pub const SETTLE_DIST_SCALE: f32 = 26.0;
pub const SETTLE_VEL_SCALE: f32 = 520.0;

/// Corner radius (px, at zoom 1.0) of the RESTING rounded square. Large enough
/// that the block reads as a friendly "roundish square" (soft corners), not a
/// hard terminal block. The radius is passed PER-INSTANCE (it morphs down toward
/// the streak's thin-bar radius in motion), but this is the at-rest reference and
/// the value the GPU clamps against the rect half-extent.
pub const CORNER_RADIUS: f32 = 7.0;

/// Corner radius (px, at zoom 1.0) of the MOTION trailing-underline streak. Small
/// so the streak reads as a clean amber bar lying on the baseline (its short edge
/// is rounded into a comet-like cap, its long body stays a straight underline so
/// it never reads as a wavy spell squiggle).
pub const STREAK_RADIUS: f32 = 1.4;

/// One animated caret sample (a position the caret occupied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
}

/// Pure spring state for the caret. `pos` is the rendered (animated) pixel
/// position of the caret's LEFT-edge / baseline anchor; `target` is the true
/// cursor pixel position. Motion is conveyed by the rounded-square ⇄ trailing-
/// underline shape morph driven by [`CaretAnim::settle_factor`], plus a streak
/// whose length scales with `vel` (read by the renderer in `caret_geometry`).
pub struct CaretAnim {
    pub pos: Sample,
    pub vel: Sample,
    pub target: Sample,
    /// The caret position at the START of the most recent `step()`. With `pos`
    /// (the end-of-step position) this gives `frame_dx()` — how far the caret
    /// travelled this frame — which the renderer uses to bridge the trailing
    /// streak across fast glides so it never strobes into ___ ___ gaps.
    prev_pos: Sample,
    /// True while the spring has not yet settled at `target`.
    animating: bool,
    /// True once a target has been set at least once (so the first set snaps
    /// rather than gliding in from (0,0)).
    primed: bool,
    /// Per-move damping `c`, recomputed by `set_target` from the move distance
    /// (in glyph-advances) so short hops settle without overshoot while big
    /// jumps stay springy. See [`CaretAnim::move_damping`].
    damping: f32,
    /// One glyph advance in (zoomed) pixels — the yardstick `move_damping` uses
    /// to judge a move's size in glyphs rather than raw pixels, keeping the
    /// distance-aware damping zoom-invariant. Defaults to the unzoomed
    /// `render::CHAR_WIDTH`; the renderer keeps it in sync via `set_glyph_advance`.
    glyph_advance: f32,
    /// True when the current move is typing-sized (≤ [`SMALL_MOVE_ADV`] glyph-
    /// advances) OR caused by a text edit, so the underline morph is suppressed:
    /// `settle_factor()` stays pinned at 1.0 and the caret just slides as the
    /// rounded square. Set per move by `set_target`; cleared for the
    /// deterministic motion-demo path.
    streak_suppressed: bool,
    /// Set by the renderer before each `set_target`: true when this move was
    /// caused by a text EDIT (typing, delete, paste, newline) rather than
    /// navigation. An edit is ALWAYS a plain slide (no underline) however far it
    /// moves — a wide/CJK glyph, Enter, or a paste shouldn't streak — whereas a
    /// navigation move only streaks when it's a real jump (distance-gated).
    edit_move: bool,
}

impl CaretAnim {
    pub fn new() -> Self {
        Self {
            pos: Sample { x: 0.0, y: 0.0 },
            vel: Sample { x: 0.0, y: 0.0 },
            target: Sample { x: 0.0, y: 0.0 },
            prev_pos: Sample { x: 0.0, y: 0.0 },
            animating: false,
            primed: false,
            damping: DAMPING,
            glyph_advance: crate::render::CHAR_WIDTH,
            streak_suppressed: false,
            edit_move: false,
        }
    }

    /// Set the cursor's true target. The first call snaps (no glide-in); later
    /// calls to a NEW target start a glide.
    pub fn set_target(&mut self, x: f32, y: f32) {
        let new = Sample { x, y };
        if !self.primed {
            self.pos = new;
            self.vel = Sample { x: 0.0, y: 0.0 };
            self.target = new;
            self.prev_pos = self.pos;
            self.primed = true;
            self.animating = false;
            return;
        }
        if (new.x - self.target.x).abs() > f32::EPSILON
            || (new.y - self.target.y).abs() > f32::EPSILON
        {
            // Judge the move by its REAL remaining distance from where the caret
            // is RIGHT NOW (not the old target), so a new target arriving
            // mid-glide is damped for the distance actually left to travel.
            // Damping is judged by the REAL remaining distance from where the
            // caret is RIGHT NOW (not the old target), so a new target arriving
            // mid-glide is damped for the distance actually left to travel.
            let dx = new.x - self.pos.x;
            let dy = new.y - self.pos.y;
            let dist = (dx * dx + dy * dy).sqrt();
            self.damping = self.move_damping(dist);
            // Streak suppression. An EDIT (typing/delete/paste/newline) is always
            // a plain slide, however far it moves. Otherwise — navigation — it is
            // judged by the size of THIS logical move (new target vs the PREVIOUS
            // target, NOT the gap to the lagging animated pos), so a burst of
            // one-char steps (held arrow, even key-mashing where the spring falls
            // advances behind) stays plain while only a real jump (word/line/
            // Ctrl-A, or a click) exceeds the threshold and streaks.
            let step = ((new.x - self.target.x).powi(2) + (new.y - self.target.y).powi(2)).sqrt();
            self.streak_suppressed =
                self.edit_move || step <= SMALL_MOVE_ADV * self.glyph_advance;
            self.target = new;
            self.prev_pos = self.pos;
            self.animating = true;
        }
    }

    /// True while the glide means we should keep redrawing.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Horizontal distance the caret travelled during the most recent `step()`
    /// (end-of-step `pos` minus start-of-step `prev_pos`). The renderer floors
    /// the trailing-streak length with this so a fast full-line glide that moves
    /// farther than the aesthetic streak clamp still draws a streak long enough
    /// to reach back to the previous frame's leading edge — no strobing gaps.
    /// Deterministic screenshot paths leave it at 0 (they set `prev_pos = pos`).
    pub fn frame_dx(&self) -> f32 {
        self.pos.x - self.prev_pos.x
    }

    /// Set the glyph advance (px, zoom-scaled) used to measure move distance in
    /// glyphs. Keeping the yardstick zoomed makes the distance-aware damping
    /// zoom-invariant: a one-glyph hop is "one glyph" at any zoom.
    pub fn set_glyph_advance(&mut self, advance: f32) {
        self.glyph_advance = advance;
    }

    /// Mark the NEXT `set_target` as an edit move (typing/delete/paste/newline)
    /// vs. navigation. The renderer sets this from the editor's edit-vs-motion
    /// signal before every target update; an edit move always suppresses the
    /// underline regardless of distance.
    pub fn set_edit_move(&mut self, is_edit: bool) {
        self.edit_move = is_edit;
    }

    /// Damping coefficient `c` for a move of `dist` pixels. Measured in
    /// glyph-advances, it eases (smoothstep) from the near-critical
    /// [`SMALL_MOVE_DAMPING`] for hops ≤ [`SMALL_MOVE_ADV`] advances (zero
    /// overshoot — calm rapid typing) down to the springy [`DAMPING`] for jumps
    /// ≥ [`LARGE_MOVE_ADV`] advances (overshoot preserved on big moves). Pure
    /// function of `dist` + the glyph advance, so it is unit-testable and
    /// zoom-invariant.
    fn move_damping(&self, dist: f32) -> f32 {
        let advances = dist / self.glyph_advance;
        let t = ((advances - SMALL_MOVE_ADV) / (LARGE_MOVE_ADV - SMALL_MOVE_ADV)).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        SMALL_MOVE_DAMPING + (DAMPING - SMALL_MOVE_DAMPING) * smooth
    }

    /// A smooth [0,1] factor: 1.0 when the caret is at rest on its target (so the
    /// shape is the resting rounded square ON the glyph), → 0 while it is far from
    /// target and/or moving fast (so the shape drops to the baseline and stretches
    /// into the trailing underline). Driven by BOTH distance and speed so the
    /// square only re-forms once the caret has actually arrived and decelerated —
    /// mid-glide (fast spring) it reads as a streak on the line.
    ///
    /// Pure function of the current spring state, so the morph is unit-testable.
    pub fn settle_factor(&self) -> f32 {
        // Typing-sized hops never drop to the underline: the caret stays the
        // rounded square and just slides to the next cell.
        if self.streak_suppressed {
            return 1.0;
        }
        let dx = self.target.x - self.pos.x;
        let dy = self.target.y - self.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        // Each term is 1.0 when the corresponding quantity is ~0 and decays toward
        // 0 as it grows. We take the MIN so either "still far" OR "still fast"
        // keeps the caret collapsed; both must be small for the underline to form.
        let by_dist = 1.0 - (dist / SETTLE_DIST_SCALE).clamp(0.0, 1.0);
        let by_vel = 1.0 - (speed / SETTLE_VEL_SCALE).clamp(0.0, 1.0);
        let raw = by_dist.min(by_vel);
        // Smoothstep so the re-form eases in (no linear kink as it lands).
        raw * raw * (3.0 - 2.0 * raw)
    }

    /// Advance the spring by `dt` seconds. Snaps + stops when settled.
    pub fn step(&mut self, dt: f32) {
        if !self.animating {
            return;
        }
        // Record where this frame started so `frame_dx()` reports how far the
        // caret moves this step (used by the renderer to bridge the streak).
        self.prev_pos = self.pos;

        // Integrate the spring in small sub-steps for stability on long frames.
        let mut remaining = dt.clamp(0.0, 0.1);
        while remaining > 0.0 {
            let h = remaining.min(MAX_SUBSTEP);
            self.integrate(h);
            remaining -= h;
        }

        // Settle test: close enough and slow enough -> snap and stop.
        let dx = self.target.x - self.pos.x;
        let dy = self.target.y - self.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        if dist < POS_EPSILON && speed < VEL_EPSILON {
            self.pos = self.target;
            self.vel = Sample { x: 0.0, y: 0.0 };
            self.animating = false;
        }
    }

    /// One explicit-Euler spring sub-step.
    fn integrate(&mut self, h: f32) {
        let ax = STIFFNESS * (self.target.x - self.pos.x) - self.damping * self.vel.x;
        let ay = STIFFNESS * (self.target.y - self.pos.y) - self.damping * self.vel.y;
        self.vel.x += ax * h;
        self.vel.y += ay * h;
        self.pos.x += self.vel.x * h;
        self.pos.y += self.vel.y * h;
    }

    /// Snap immediately to target with no velocity (used by the at-rest
    /// deterministic screenshot path). settle_factor() is then 1.0 (the resting
    /// rounded square sitting on the glyph).
    pub fn snap_to_target(&mut self) {
        self.pos = self.target;
        self.vel = Sample { x: 0.0, y: 0.0 };
        self.prev_pos = self.pos;
        self.animating = false;
        self.primed = true;
    }

    /// Inject a fully synthetic, deterministic mid-glide state (used by the
    /// `--screenshot-motion` path): a caret part-way through a glide with a high
    /// velocity, so `settle_factor()` is near 0 and the caret renders as a long
    /// trailing underline streak on the baseline partway along its path. No clock
    /// is consulted, so the frame is reproducible.
    pub fn inject_motion(&mut self, target: Sample, pos: Sample, vel: Sample) {
        self.target = target;
        self.pos = pos;
        self.vel = vel;
        self.prev_pos = pos;
        self.animating = true;
        self.primed = true;
        // The motion demo is explicitly a long fast glide: show the streak.
        self.streak_suppressed = false;
    }
}

impl Default for CaretAnim {
    fn default() -> Self {
        Self::new()
    }
}

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

impl CaretPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, caret_srgb: [u8; 3]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("caret shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/caret.wgsl").into()),
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
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("caret pipeline"),
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
        });

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
        let globals = Globals {
            viewport: [width as f32, height as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));

        let inst = CaretInstance {
            center: [center_x, center_y],
            half_size: [rect_w * 0.5, rect_h * 0.5],
            corner,
            alpha: 1.0,
            color: self.color,
        };
        queue.write_buffer(&self.instance_buf, 0, bytemuck_lite::bytes_of(&inst));
        self.instance_count = 1;
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
fn srgb_u8_to_linear(c: [u8; 3]) -> [f32; 3] {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: run the spring to rest from a downward jump and report frames +
    /// whether it overshot the target.
    fn settle(target: Sample, start: Sample, dt: f32) -> (usize, bool, f32) {
        let mut a = CaretAnim::new();
        // Prime at start so the next set_target glides.
        a.set_target(start.x, start.y);
        a.set_target(target.x, target.y);
        let mut frames = 0;
        let mut overshot = false;
        // The caret starts at `start` and glides UP to `target` (target.y < start.y).
        while a.is_animating() && frames < 2000 {
            a.step(dt);
            frames += 1;
            // Overshoot = pos goes past target in the direction of travel.
            if start.y > target.y && a.pos.y < target.y - 0.5 {
                overshot = true;
            }
        }
        (frames, overshot, a.pos.y)
    }

    #[test]
    fn first_target_snaps_no_glide() {
        let mut a = CaretAnim::new();
        a.set_target(100.0, 200.0);
        assert!(!a.is_animating(), "first target must snap, not animate");
        assert_eq!(a.pos, Sample { x: 100.0, y: 200.0 });
    }

    #[test]
    fn spring_settles_and_stops() {
        // Glide from y=300 up to y=20 at 60 fps.
        let (frames, _overshot, final_y) = settle(
            Sample { x: 16.0, y: 20.0 },
            Sample { x: 16.0, y: 300.0 },
            1.0 / 60.0,
        );
        // Must come to rest exactly on target and stop animating.
        assert!((final_y - 20.0).abs() < 1.0, "did not settle on target: {final_y}");
        // ~140-160 ms at 60 fps is ~9-11 frames; allow slack but bound it so a
        // runaway/never-settling spring fails the test.
        assert!(frames > 3 && frames < 60, "settle frames out of range: {frames}");
    }

    #[test]
    fn spring_is_underdamped_overshoots() {
        // A lightly underdamped spring should overshoot the target slightly.
        let (_frames, overshot, _final_y) = settle(
            Sample { x: 16.0, y: 20.0 },
            Sample { x: 16.0, y: 400.0 },
            1.0 / 120.0,
        );
        assert!(overshot, "expected a small overshoot (underdamped feel)");
    }

    #[test]
    fn settles_within_epsilon() {
        let mut a = CaretAnim::new();
        a.set_target(0.0, 0.0);
        a.set_target(50.0, 50.0);
        while a.is_animating() {
            a.step(1.0 / 60.0);
        }
        let dx = (a.pos.x - a.target.x).abs();
        let dy = (a.pos.y - a.target.y).abs();
        assert!(dx <= POS_EPSILON && dy <= POS_EPSILON);
        assert_eq!(a.vel.x, 0.0);
        assert_eq!(a.vel.y, 0.0);
    }

    // --- Shape-morph settle factor (dot <-> underline) --------------------

    #[test]
    fn settle_factor_is_one_at_rest() {
        // At rest exactly on target: settle_factor == 1.0 (full underline).
        let mut a = CaretAnim::new();
        a.set_target(100.0, 200.0); // snaps; pos == target, vel == 0
        assert!(!a.is_animating());
        assert!((a.settle_factor() - 1.0).abs() < 1e-6, "rest must be full underline");
    }

    #[test]
    fn settle_factor_collapses_when_moving_fast() {
        // A caret far from target AND moving fast must collapse toward the dot
        // (settle_factor near 0).
        let mut a = CaretAnim::new();
        a.inject_motion(
            Sample { x: 0.0, y: 0.0 },
            Sample { x: 0.0, y: 300.0 },
            Sample { x: 0.0, y: -1500.0 },
        );
        let s = a.settle_factor();
        assert!(s < 0.05, "fast mid-glide must collapse to a dot, got {s}");
    }

    #[test]
    fn settle_factor_monotone_reforms_as_it_arrives() {
        // As the caret nears the target and decelerates, the settle factor must
        // rise monotonically toward 1.0 over the final stretch of a glide. We
        // sample it at the very end of a glide and assert it is climbing.
        let mut a = CaretAnim::new();
        a.set_target(16.0, 300.0);
        a.set_target(16.0, 20.0);
        let mut last = a.settle_factor();
        let mut climbed_to_full = false;
        let mut min_seen = 1.0f32;
        while a.is_animating() {
            a.step(1.0 / 120.0);
            let s = a.settle_factor();
            min_seen = min_seen.min(s);
            last = s;
        }
        // Mid-glide it dipped low (was a dot)...
        assert!(min_seen < 0.2, "should have collapsed mid-glide, min={min_seen}");
        // ...and by the time it settled it is the full underline.
        if (last - 1.0).abs() < 1e-3 {
            climbed_to_full = true;
        }
        assert!(climbed_to_full, "must re-form to full underline at rest, last={last}");
    }

    #[test]
    fn settle_factor_in_unit_range() {
        // For arbitrary injected states the factor stays within [0,1].
        for (px, py, vx, vy) in [
            (0.0, 0.0, 0.0, 0.0),
            (5.0, 5.0, 100.0, 100.0),
            (200.0, 0.0, -3000.0, 0.0),
            (1.0, 1.0, 10.0, -10.0),
        ] {
            let mut a = CaretAnim::new();
            a.inject_motion(
                Sample { x: 0.0, y: 0.0 },
                Sample { x: px, y: py },
                Sample { x: vx, y: vy },
            );
            let s = a.settle_factor();
            assert!((0.0..=1.0).contains(&s), "settle factor out of [0,1]: {s}");
        }
    }

    #[test]
    fn injected_motion_animates() {
        let mut a = CaretAnim::new();
        a.inject_motion(
            Sample { x: 16.0, y: 16.0 },
            Sample { x: 16.0, y: 120.0 },
            Sample { x: 0.0, y: -300.0 },
        );
        assert!(a.is_animating());
    }

    // --- Distance-aware damping + frame bridging (the two refinements) -----

    #[test]
    fn one_glyph_hop_never_overshoots() {
        // A single-character hop (~1 glyph-advance) is near-critically damped, so
        // it must settle WITHOUT overshooting — rapid typing reads as calm.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(adv, 0.0); // one-glyph hop to the right
        let mut overshot = false;
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            frames += 1;
            if a.pos.x > adv + 0.5 {
                overshot = true;
            }
        }
        assert!(!overshot, "a one-glyph hop must not overshoot, x={}", a.pos.x);
    }

    #[test]
    fn large_jump_still_overshoots() {
        // A big jump (~42 advances) stays springy and keeps its overshoot.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(0.0, 600.0); // 600px jump down
        let mut overshot = false;
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            frames += 1;
            if a.pos.y > 600.0 + 0.5 {
                overshot = true;
            }
        }
        assert!(overshot, "a 600px jump must keep its springy overshoot");
    }

    #[test]
    fn move_damping_monotonic_in_distance() {
        // Damping must be monotonically NON-INCREASING in distance: tiny hops are
        // the most damped, big jumps the springiest.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        let mut prev = a.move_damping(0.0);
        let mut i = 1;
        while i <= 200 {
            let dist = i as f32 * 2.0;
            let d = a.move_damping(dist);
            assert!(
                d <= prev + 1e-4,
                "damping increased with distance: {d} > {prev} at dist={dist}"
            );
            prev = d;
            i += 1;
        }
        // Endpoints land on the documented band.
        assert!(
            (a.move_damping(0.0) - SMALL_MOVE_DAMPING).abs() < 1e-3,
            "tiny move must use SMALL_MOVE_DAMPING"
        );
        let far = crate::render::CHAR_WIDTH * (LARGE_MOVE_ADV + 4.0);
        assert!(
            (a.move_damping(far) - DAMPING).abs() < 1e-3,
            "far move must use springy DAMPING"
        );
    }

    #[test]
    fn damping_zoom_invariant_for_one_glyph_move() {
        // A one-glyph move must yield the SAME damping at any zoom: the glyph
        // advance scales with zoom and so does the pixel distance, so the move
        // measured in advances (and thus the damping) is unchanged.
        let adv1 = crate::render::CHAR_WIDTH;
        let adv2 = crate::render::CHAR_WIDTH * 2.0;
        let mut a1 = CaretAnim::new();
        a1.set_glyph_advance(adv1);
        let mut a2 = CaretAnim::new();
        a2.set_glyph_advance(adv2);
        let d1 = a1.move_damping(adv1); // one glyph at zoom 1
        let d2 = a2.move_damping(adv2); // one glyph at zoom 2
        assert!(
            (d1 - d2).abs() < 1e-4,
            "one-glyph damping must be zoom-invariant: {d1} vs {d2}"
        );
    }

    #[test]
    fn typing_hop_shows_no_underline() {
        // A single-character advance (typing) must NOT drop to the underline:
        // settle_factor stays pinned at 1.0 for the whole slide, so the caret
        // renders as the rounded square the entire time.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(100.0, 50.0); // prime / snap
        a.set_target(100.0 + adv, 50.0); // type one char
        let mut min_s = a.settle_factor();
        let mut frames = 0;
        while a.is_animating() && frames < 2000 {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
            frames += 1;
        }
        assert!(
            min_s > 0.999,
            "a typing hop must not show the underline, min settle={min_s}"
        );
    }

    #[test]
    fn mashing_keys_shows_no_underline() {
        // Type so fast (one char EVERY frame) the spring can't catch up and falls
        // several advances behind — the case where the old gap-from-pos check
        // wrongly flipped the underline on. Suppression is keyed to the per-
        // keystroke target delta, so it must stay off the whole burst.
        let adv = crate::render::CHAR_WIDTH;
        let mut a = CaretAnim::new();
        a.set_glyph_advance(adv);
        a.set_target(100.0, 50.0); // prime
        let mut tx = 100.0_f32;
        let mut min_s = a.settle_factor();
        let mut max_lag = 0.0_f32;
        for _ in 0..30 {
            tx += adv; // one-char advance per frame
            a.set_target(tx, 50.0);
            a.step(1.0 / 60.0);
            min_s = min_s.min(a.settle_factor());
            max_lag = max_lag.max((a.target.x - a.pos.x).abs());
        }
        while a.is_animating() {
            a.step(1.0 / 60.0);
            min_s = min_s.min(a.settle_factor());
        }
        // The burst really did outrun the spring (else the test proves nothing):
        // the old pos-based check would have streaked here.
        assert!(
            max_lag > 1.5 * adv,
            "test must drive the spring past the threshold, lag={} adv",
            max_lag / adv
        );
        // ...yet no underline ever appeared.
        assert!(min_s > 0.999, "mashing keys must not show the underline, min settle={min_s}");
    }

    #[test]
    fn edit_move_suppresses_underline_even_when_large() {
        // An edit can move the caret a long way in one step (Enter to a far
        // column, a wide/CJK glyph, a paste), but it's still typing — no
        // underline, however large the jump.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(16.0, 40.0); // prime
        a.set_edit_move(true);
        a.set_target(200.0, 90.0); // big move, but flagged as an edit
        let mut min_s = a.settle_factor();
        while a.is_animating() {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
        }
        assert!(min_s > 0.999, "an edit move must not streak even when large, min={min_s}");
    }

    #[test]
    fn navigation_jump_still_shows_underline() {
        // A real jump (here a full-line Ctrl-E style glide) must still collapse
        // to the streak mid-flight — suppression is only for typing-sized hops.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(16.0, 40.0); // prime / snap
        a.set_target(600.0, 40.0); // long horizontal jump
        let mut min_s = a.settle_factor();
        while a.is_animating() {
            a.step(1.0 / 120.0);
            min_s = min_s.min(a.settle_factor());
        }
        assert!(min_s < 0.2, "a navigation jump must still show the underline, min={min_s}");
    }

    #[test]
    fn frame_dx_reports_large_per_frame_advance_mid_glide() {
        // A fast full-line glide moves farther than the streak clamp in a single
        // 60fps frame; frame_dx() must report that large advance so the renderer
        // can bridge the streak across it.
        let mut a = CaretAnim::new();
        a.set_glyph_advance(crate::render::CHAR_WIDTH);
        a.set_target(0.0, 0.0); // prime / snap
        a.set_target(1200.0, 0.0); // fast cross-screen jump
        a.step(1.0 / 60.0);
        assert!(
            a.frame_dx().abs() > 64.0,
            "fast glide must move more than the streak clamp in one frame, got {}",
            a.frame_dx()
        );

        // The deterministic injected-motion screenshot path leaves frame_dx at 0.
        let mut b = CaretAnim::new();
        b.inject_motion(
            Sample { x: 1000.0, y: 0.0 },
            Sample { x: 200.0, y: 0.0 },
            Sample { x: 1900.0, y: 0.0 },
        );
        assert_eq!(b.frame_dx(), 0.0, "injected motion must keep frame_dx == 0");
    }
}
