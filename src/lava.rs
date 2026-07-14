//! src/lava.rs — the LAVA-LAMP GROUND machinery: awl's first TIME-VARYING
//! background. ONE continuous viewport-space metaball field ("lava lamp"
//! register) sits behind the centered page; the page mask merely reveals it in
//! the margins. Page-width changes never resize or recompose the lamp — the
//! mirror of Wagtail (the one world whose one warm thing is the GROUND itself).
//! This module owns:
//!
//! * [`LavaPipeline`] — the wgpu pipeline (`shaders/lava.wgsl`), a single
//!   fullscreen triangle drawn AFTER the margin-gradient background pass and
//!   BEFORE every foreground layer. Inactive (draws NOTHING) unless the active
//!   world's [`crate::theme::Background`] is [`Background::Lava`] — so every one
//!   of the fifteen non-lava worlds stays byte-identical.
//! * The PURE field + column-mask math ([`metaball_field`], [`column_mask`],
//!   [`animated_center`]) — the Rust mirror the shader must stay in lockstep
//!   with, unit-tested here without a GPU (the dither.rs / Bayer precedent).
//! * The ANIMATION CADENCE helpers: the gate ([`lava_should_tick`]) the live
//!   App reads before arming its slow ~10 fps `WaitUntil` tick, the bounded
//!   phase advance ([`advance_phase`]), and the effective-phase resolver
//!   ([`lava_phase_for`]) — env override > Reduce-Motion freeze > App-driven.
//! * The dev-only [`env_override`] gallery knob (`AWL_LAVA=...`), mirroring the
//!   `AWL_CJK_FORCE` / probe `AWL_LAVA_PROBE` precedent: a total no-op unless
//!   set, so normal + headless determinism is untouched when absent.
//!
//! NEGOTIATED LAWS (logged on `THEMES.md` / the queue's lava entry): 0%-idle
//! (the tick arms ONLY for a lava world with ambient motion on, focused, not
//! reduced — a non-lava world schedules zero frames); Reduce Motion freezes to a
//! fixed phase; a headless capture renders the fixed t=0 phase (deterministic).
//!
//! Firetail and Mangrove ship [`Background::Lava`]; every other world leaves the
//! pipeline dormant.

use crate::theme::{Background, LavaEdge, Srgb};
use std::sync::OnceLock;

// --- CADENCE / PHASE constants ------------------------------------------------

/// The ambient tick period — a SLOW ~10 fps cadence (never the hot per-frame
/// `advance()` loop). The live App's `about_to_wait` arms a single `WaitUntil`
/// this far out, advances the phase, requests one redraw, and re-arms — so a lava
/// world costs ~10 sparse frames/sec, and a non-lava world costs zero. TASTE
/// TUNABLE — flagged for live review (the lava's speed is a feel call), named
/// like `THEME_FONT_DEBOUNCE`.
pub const LAVA_TICK_MS: u64 = 100;

/// Phase advance rate in CYCLES PER SECOND. The composed field loops over
/// [`LAVA_LOOP_CYCLES`] (two cycles, because horizontal sway runs at half the
/// vertical frequency), so one seamless lamp loop lasts ~67 s at 0.03. TASTE
/// TUNABLE.
pub const LAVA_SPEED: f32 = 0.03;

/// The WHOLE field's period in phase cycles. Vertical bob repeats after one
/// cycle, but horizontal sway uses half-frequency and repeats only after two;
/// wrapping at two is therefore the first phase where every blob center meets
/// its own starting point.
pub const LAVA_LOOP_CYCLES: f32 = 2.0;

/// One fixed ambient advancement step. A delayed event-loop wake (notably while
/// macOS is dragging the window) may report much more wall time than this, but
/// the lamp advances by at most one sparse-tick step: it drifts instead of
/// catching up in one visible jump.
pub const LAVA_TICK_SECONDS: f32 = LAVA_TICK_MS as f32 / 1000.0;

/// The FROZEN phase: what the lamp settles to under Reduce Motion, and the fixed
/// phase a headless capture always renders (t=0, deterministic). The base blob
/// layout ([`BACKDROP_BLOBS`]) is authored so this phase reads as a settled mid
/// composition, so the one frozen frame serves BOTH the accessibility freeze
/// and the capture — matching the caret-demo `settle()` precedent (`render.rs`).
pub const LAVA_FROZEN_PHASE: f32 = 0.0;

/// MARGINS-ONLY mask feather WIDTH (px): how far into the margin, starting from
/// the column edge, the field ramps 0 → full strength. Comfortably inside a
/// modest margin. TASTE TUNABLE — flagged for live review.
pub const MARGIN_GAP_PX: f32 = 28.0;

/// Maximum blobs the shader's uniform carries (`array<vec4<f32>, 8>`); the
/// backdrop currently uses the full budget, and `blob_count` names how many are
/// live.
pub const MAX_BLOBS: usize = 8;

/// ONE continuous backdrop field, authored in viewport UV and wholly independent
/// of the page column. Each row is `[cx, cy, r, w]`: center in viewport UV,
/// radius as a fraction of viewport height, and field weight. Several blobs sit
/// behind the ordinary page footprint on purpose; widening/narrowing the page
/// only occludes/reveals this same composition instead of manufacturing two
/// separately-sized side lamps.
pub const BACKDROP_BLOBS: [[f32; 4]; 8] = [
    [0.08, 0.18, 0.14, 0.90],
    [0.16, 0.50, 0.18, 1.05],
    [0.12, 0.82, 0.16, 0.95],
    [0.38, 0.68, 0.21, 1.10],
    [0.58, 0.30, 0.20, 1.00],
    [0.86, 0.18, 0.14, 0.90],
    [0.82, 0.50, 0.18, 1.05],
    [0.88, 0.82, 0.16, 0.95],
];

#[allow(dead_code)] // shader-mirror constant (see the pure-math note below).
const TAU: f32 = std::f32::consts::TAU;

// --- PURE math (the shader mirror, unit-tested) -------------------------------
//
// `#[allow(dead_code)]` on the four functions below (+ `TAU`): the REAL runtime
// math happens in `shaders/lava.wgsl`'s own copy of this exact field + mask; these
// Rust functions exist ONLY as the pure mirror `lava::tests` exercises — the
// established `render::dither`/`SelectionPipeline::instance_count` idiom for a
// test-only shader mirror. They MUST stay in lockstep with the WGSL.

/// WGSL-matching `smoothstep(edge0, edge1, x)`: 0 below `edge0`, 1 above `edge1`,
/// a Hermite ease between. Pure — the Rust mirror of the shader's own builtin.
#[allow(dead_code)]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 == edge1 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The MARGINS-ONLY column mask at pixel x (px): 0 inside the writing column
/// `[col_left, col_right)` and at its edge, ramping to 1 a `gap` px into the
/// margin. The lava is drawn at this coverage, so the field fades entirely
/// OUTSIDE the column and the page stays a clean flat ground. MUST match
/// `shaders/lava.wgsl`'s `mask`. `gap` is floored at 1.0 (matching the shader).
#[allow(dead_code)]
pub fn column_mask(x: f32, col_left: f32, col_right: f32, gap: f32) -> f32 {
    let dist_outside = (col_left - x).max(x - col_right);
    smoothstep(0.0, gap.max(1.0), dist_outside)
}

/// The ANIMATED center (UV space) of base blob `i` at `phase` (in cycles) — the
/// slow lava bob, a per-blob sine keyed off the index so the lamps never move in
/// unison. MUST match `shaders/lava.wgsl`'s `blob_center`. Pure.
#[allow(dead_code)]
pub fn animated_center(
    i: usize,
    base_cx: f32,
    base_cy: f32,
    base_r: f32,
    viewport: (f32, f32),
    phase: f32,
) -> (f32, f32) {
    let fi = i as f32;
    let amp_y = 0.055 + 0.020 * (fi * 0.37).fract();
    // Horizontal sway follows the authored viewport-relative radius, so the
    // whole backdrop scales coherently with the window, never with page width.
    let aspect = viewport.1.max(1.0) / viewport.0.max(1.0);
    let amp_x = base_r * aspect * (0.18 + 0.08 * (fi * 0.61).fract());
    let off = fi * 1.7;
    let cy = base_cy + amp_y * (phase * TAU + off).sin();
    let cx = base_cx + amp_x * (phase * TAU * 0.5 + off * 1.3).sin();
    (cx, cy)
}

/// The summed metaball FIELD at pixel `px` (physical px), the Gaussian-falloff
/// sum over the animated blobs — MUST match `shaders/lava.wgsl`'s
/// `metaball_field`. `blobs` are `[cx, cy, r, w]` base positions; `viewport` is
/// `(width, height)` px. Pure (a function of position + phase, never a clock).
#[allow(dead_code)]
pub fn metaball_field(px: (f32, f32), viewport: (f32, f32), blobs: &[[f32; 4]], phase: f32) -> f32 {
    const FIELD_K: f32 = 1.2;
    let mut total = 0.0;
    for (i, b) in blobs.iter().enumerate() {
        let (cx, cy) = animated_center(i, b[0], b[1], b[2], viewport, phase);
        let center = (cx * viewport.0, cy * viewport.1);
        let r_px = (b[2] * viewport.1).max(1.0);
        let dx = px.0 - center.0;
        let dy = px.1 - center.1;
        let dist_sq = dx * dx + dy * dy;
        total += b[3] * (-FIELD_K * dist_sq / (r_px * r_px)).exp();
    }
    total
}

// --- CADENCE / PHASE resolution (pure, unit-tested) ---------------------------

/// THE CADENCE GATE: may the live App arm its slow ambient lava tick THIS frame?
/// True ONLY when a lava world is active AND ambient motion is on AND motion is
/// NOT reduced AND the window is focused (pause on blur). A non-lava world
/// (`active == false`) is always false, so it schedules ZERO extra frames —
/// preserving 0% idle CPU. Pure, so the whole gate is unit-testable.
pub fn lava_should_tick(active: bool, ambient_on: bool, reduced: bool, focused: bool) -> bool {
    active && ambient_on && !reduced && focused
}

/// Bound an ambient wake's elapsed wall time to ONE fixed sparse tick. Normal
/// due wakes therefore advance by exactly [`LAVA_TICK_SECONDS`]; delayed wakes
/// never accumulate and replay the missing wall time as a visible catch-up jump.
/// Pure, so the macOS event-loop-stall behavior is law-testable without a window.
pub fn ambient_tick_dt(elapsed: f32) -> f32 {
    elapsed.max(0.0).min(LAVA_TICK_SECONDS)
}

/// Advance the phase by one bounded ambient step at [`LAVA_SPEED`], wrapping to
/// `[0, LAVA_LOOP_CYCLES)` so a long-running session never loses `sin` precision
/// AND the half-frequency horizontal term meets its own endpoint. Pure.
pub fn advance_phase(phase: f32, dt: f32) -> f32 {
    let p = phase + ambient_tick_dt(dt) * LAVA_SPEED;
    p.rem_euclid(LAVA_LOOP_CYCLES)
}

/// The EFFECTIVE render phase: the dev gallery `env` override wins outright
/// (frozen gallery captures); else Reduce Motion pins [`LAVA_FROZEN_PHASE`]
/// (mirroring the caret-demo `settle()` precedent); else the App-driven `stored`
/// phase (which is [`LAVA_FROZEN_PHASE`] = 0.0 in a headless capture, since the
/// capture never ticks). Pure — the whole determinism story reads off this one
/// resolver. See `TextPipeline::lava_render_phase`.
pub fn lava_phase_for(stored: f32, reduced: bool, env: Option<f32>) -> f32 {
    match env {
        Some(e) => e,
        None if reduced => LAVA_FROZEN_PHASE,
        None => stored,
    }
}

// --- The dev-only gallery knob (AWL_LAVA=...) ---------------------------------
//
// Mirrors `AWL_CJK_FORCE` / the probe's `AWL_LAVA_PROBE` exactly: read ONCE at
// startup, memoized, a total no-op unless set. Since NO world ships a lava
// background yet, this is the only way to render the lamp (it forces a
// `Background::Lava` over whatever world is active, at a FIXED phase), so a
// gallery capture can be produced for the human eyeball step. Format:
//   AWL_LAVA=<palette>:<phase>[:<edge>][:<dither>]
//   <palette> = warm | deepsea            (the probe's tuned, legibility-checked palettes)
//   <phase>   = a float (the frozen composition, e.g. 0.0 / 0.35)
//   <edge>    = hard | glow               (optional; default glow — the probe's agent pick)
//   <dither>  = dither                    (optional; the coarse Bayer print-grain)
// e.g. AWL_LAVA=deepsea:0.35:glow:dither

fn parse_spec(raw: &str) -> Option<(Background, f32)> {
    let mut parts = raw.split(':');
    let palette = parts.next()?;
    let phase: f32 = parts.next()?.parse().ok()?;
    let mut edge = LavaEdge::Glow;
    let mut dithered = false;
    for tok in parts {
        match tok {
            "hard" => edge = LavaEdge::Hard,
            "glow" => edge = LavaEdge::Glow,
            "dither" | "dithered" => dithered = true,
            "" => {}
            _ => return None,
        }
    }
    // Reuse the SHIPPED worlds' authored colors rather than carrying a second
    // probe-only copy that can drift after a palette retune. The env spec still
    // owns its requested edge/dither treatment below.
    let source = match palette {
        "warm" => crate::theme::FIRETAIL.background,
        "deepsea" => crate::theme::MANGROVE.background,
        _ => return None,
    };
    let (ground, blob_lo, blob_hi, _, _) = source.lava_params()?;
    Some((
        Background::Lava {
            ground,
            blob_lo,
            blob_hi,
            edge,
            dithered,
        },
        phase,
    ))
}

fn spec() -> &'static Option<(Background, f32)> {
    static ONCE: OnceLock<Option<(Background, f32)>> = OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::var("AWL_LAVA")
            .ok()
            .as_deref()
            .and_then(parse_spec)
    })
}

/// The dev gallery override [`Background::Lava`], if `AWL_LAVA` was set at startup
/// and parses. `None` (every normal + headless run) means: no override, the
/// active world's real background stands — byte-identical to before this feature.
pub fn env_override() -> Option<Background> {
    spec().as_ref().map(|(bg, _)| *bg)
}

/// The dev gallery override's FIXED phase, if `AWL_LAVA` is set. Consumed by
/// [`lava_phase_for`] (env wins outright), so a gallery capture renders exactly
/// the requested frozen composition.
pub fn env_phase() -> Option<f32> {
    spec().as_ref().map(|(_, phase)| *phase)
}

// --- The wgpu pipeline --------------------------------------------------------

/// Uniform globals. MUST match `Globals` in `shaders/lava.wgsl`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    blob_count: u32,
    dither: u32,
    /// `[col_left_px, col_right_px, gap_px, mask_mode]` — `mask_mode` from
    /// [`LavaEdge::mask_mode`] (1.0 hard, 2.0 glow).
    margin: [f32; 4],
    /// `[phase, 0, 0, 0]` — phase in cycles.
    anim: [f32; 4],
    ground: [f32; 4],
    blob_lo: [f32; 4],
    blob_hi: [f32; 4],
    blobs: [[f32; 4]; MAX_BLOBS],
}

/// The LAVA-LAMP metaball ground pipeline: one fullscreen triangle, drawn right
/// after the margin-gradient background and before every foreground layer.
/// Mirrors [`crate::background::BackgroundPipeline`]'s structure (std140-friendly
/// globals, a tiny local bytemuck shim, vertex-free draw, straight-alpha
/// over-blend). `active` is set each [`Self::prepare`]; [`Self::draw`] is a total
/// no-op while `false`, so a non-lava world draws NOTHING (byte-identical).
pub struct LavaPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    active: bool,
}

impl LavaPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lava shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/lava.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lava globals layout"),
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
            label: Some("lava globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lava globals bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lava pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lava pipeline"),
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
                // Straight-alpha over-blend (same as the background pipeline): the
                // margins composite onto the base-ground pass, the transparent
                // column (alpha 0) leaves the base_100 page clear untouched.
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            active: false,
        }
    }

    /// Upload this frame's globals from the resolved lava `params` (`None` for
    /// every non-lava world → the pipeline goes INACTIVE and draws nothing), the
    /// live column bounds (`col_left`/`col_w` from `TextPipeline::column_left`/
    /// `column_width`, the one geometry owner), and the effective `phase`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        col_left: f32,
        col_w: f32,
        params: Option<(Srgb, Srgb, Srgb, LavaEdge, bool)>,
        phase: f32,
    ) {
        let (ground, blob_lo, blob_hi, edge, dithered) = match params {
            Some(p) => p,
            None => {
                self.active = false;
                return;
            }
        };
        self.active = true;
        let mut blobs = [[0.0f32; 4]; MAX_BLOBS];
        for (dst, src) in blobs.iter_mut().zip(BACKDROP_BLOBS.iter()) {
            *dst = *src;
        }
        let globals = Globals {
            viewport: [width as f32, height as f32],
            blob_count: BACKDROP_BLOBS.len() as u32,
            dither: dithered as u32,
            margin: [col_left, col_left + col_w, MARGIN_GAP_PX, edge.mask_mode()],
            anim: [phase, 0.0, 0.0, 0.0],
            ground: srgb_u8_to_linear(ground),
            blob_lo: srgb_u8_to_linear(blob_lo),
            blob_hi: srgb_u8_to_linear(blob_hi),
            blobs,
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck_lite::bytes_of(&globals));
    }

    /// Record the fullscreen-triangle draw — a TOTAL NO-OP while inactive (no
    /// lava world / the last `prepare` saw `None`), so a non-lava frame is
    /// byte-identical to before this feature existed.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if !self.active {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Convert an opaque sRGB u8 color to linear-light rgba for the shader (the
/// render target is sRGB). Same converter as the background pipeline's.
fn srgb_u8_to_linear(c: Srgb) -> [f32; 4] {
    fn ch(u: u8) -> f32 {
        let s = u as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [ch(c.r), ch(c.g), ch(c.b), 1.0]
}

mod bytemuck_lite {
    /// # Safety
    /// Implementors must be `#[repr(C)]`, contain no padding, and consist only of
    /// plain-old-data fields.
    pub unsafe trait Pod: Copy + 'static {}

    pub fn bytes_of<T: Pod>(t: &T) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((t as *const T) as *const u8, core::mem::size_of::<T>())
        }
    }
}

unsafe impl bytemuck_lite::Pod for Globals {}

#[cfg(test)]
mod tests {
    use super::*;

    // --- The pure metaball FIELD ------------------------------------------------

    #[test]
    fn field_is_strongest_at_a_blob_center_and_decays_with_distance() {
        // ONE blob at UV (0.5, 0.5), r=0.1 of height, weight 1.0, phase 0 (no
        // animation offset for a single blob at index 0 with amp*sin(0)=0... amp_y
        // adds sin(0)=0, so index-0 center is exactly its base at phase 0).
        let blobs = [[0.5f32, 0.5, 0.1, 1.0]];
        let vp = (1000.0, 800.0);
        let center = animated_center(0, 0.5, 0.5, 0.1, vp, 0.0);
        let center_px = (center.0 * vp.0, center.1 * vp.1);
        let at_center = metaball_field(center_px, vp, &blobs, 0.0);
        let near = metaball_field((center_px.0 + 40.0, center_px.1), vp, &blobs, 0.0);
        let far = metaball_field((center_px.0 + 400.0, center_px.1), vp, &blobs, 0.0);
        assert!(
            at_center > near,
            "field peaks at the center: {at_center} > {near}"
        );
        assert!(near > far, "field decays with distance: {near} > {far}");
        assert!(
            at_center <= 1.0 + 1e-4,
            "peak field ~= weight 1.0: {at_center}"
        );
        assert!(far < 0.01, "far field is negligible: {far}");
    }

    #[test]
    fn two_near_blobs_sum_higher_than_one_between_them() {
        // The metaball "merge": the field between two nearby blobs exceeds either
        // blob's own field there (why they neck + split).
        let one = [[0.40f32, 0.5, 0.1, 1.0]];
        let two = [[0.40f32, 0.5, 0.1, 1.0], [0.46, 0.5, 0.1, 1.0]];
        let vp = (1000.0, 800.0);
        let mid_px = (0.43 * vp.0, 0.5 * vp.1);
        let f_one = metaball_field(mid_px, vp, &one, 0.0);
        let f_two = metaball_field(mid_px, vp, &two, 0.0);
        assert!(
            f_two > f_one,
            "summed field is higher between two blobs: {f_two} > {f_one}"
        );
    }

    #[test]
    fn animation_moves_a_blob_between_distinct_phases_but_is_bounded() {
        // A blob at a non-zero index actually bobs across phases, and stays within
        // its authored amplitude (never wandering into the column).
        let base_cy = 0.5;
        let vp = (1000.0, 800.0);
        let a = animated_center(2, 0.05, base_cy, 0.05, vp, 0.0);
        let b = animated_center(2, 0.05, base_cy, 0.05, vp, 0.25);
        assert!(
            (a.1 - b.1).abs() > 1e-3,
            "phase 0 vs 0.25 move the blob: {a:?} {b:?}"
        );
        for phase in [0.0, 0.1, 0.37, 0.5, 0.83, 0.99, 1.25, 1.99] {
            let (_, cy) = animated_center(2, 0.05, base_cy, 0.05, vp, phase);
            assert!((cy - base_cy).abs() < 0.09, "bob stays bounded: {cy}");
        }
    }

    // --- ONE viewport-space backdrop, page-width invariant --------------------

    #[test]
    fn backdrop_layout_has_no_page_geometry_input() {
        let vp = (1200.0, 800.0);
        // BACKDROP_BLOBS has no column argument at all: page geometry can only
        // reach `column_mask`, never the underlying centers/radii/field.
        assert_eq!(BACKDROP_BLOBS.len(), MAX_BLOBS);
        for b in BACKDROP_BLOBS {
            assert!((0.0..=1.0).contains(&b[0]));
            assert!((0.0..=1.0).contains(&b[1]));
            assert!(
                b[2] * vp.1 >= 100.0,
                "backdrop blob is substantial at 1200×800"
            );
        }
    }

    #[test]
    fn page_width_only_occludes_or_reveals_the_same_backdrop_field() {
        let vp = (1200.0, 800.0);
        let px = (250.0, 400.0);
        let field = metaball_field(px, vp, &BACKDROP_BLOBS, 0.0);
        assert!(
            field > 0.5,
            "the immutable backdrop has visible lava at the probe: {field}"
        );
        assert!(column_mask(px.0, 300.0, 900.0, MARGIN_GAP_PX) > 0.0);
        assert_eq!(column_mask(px.0, 200.0, 1000.0, MARGIN_GAP_PX), 0.0);
        // The raw field is deliberately not recomputed from either column: the
        // wider page hides this pixel; the narrower page reveals the SAME value.
        assert_eq!(field, metaball_field(px, vp, &BACKDROP_BLOBS, 0.0));
    }

    #[test]
    fn backdrop_continues_behind_the_page_while_the_page_stays_flat() {
        let vp = (1200.0, 800.0);
        let b = BACKDROP_BLOBS[3]; // authored under the ordinary page footprint
        let center = animated_center(3, b[0], b[1], b[2], vp, 0.0);
        let px = (center.0 * vp.0, center.1 * vp.1);
        assert!(metaball_field(px, vp, &BACKDROP_BLOBS, 0.0) >= b[3]);
        assert_eq!(column_mask(px.0, 300.0, 900.0, MARGIN_GAP_PX), 0.0);
    }

    // --- The MARGINS-ONLY column mask ------------------------------------------

    #[test]
    fn column_mask_is_zero_inside_the_column_and_full_in_the_margin() {
        let (col_left, col_right, gap) = (300.0, 900.0, 28.0);
        // Deep inside the column: masked out entirely (transparent → page clear).
        assert_eq!(column_mask(600.0, col_left, col_right, gap), 0.0);
        assert_eq!(
            column_mask(col_left, col_left, col_right, gap),
            0.0,
            "0 AT the edge"
        );
        assert_eq!(
            column_mask(col_right, col_left, col_right, gap),
            0.0,
            "0 AT the far edge"
        );
        // A full gap into the left margin: full strength.
        assert!((column_mask(col_left - gap, col_left, col_right, gap) - 1.0).abs() < 1e-4);
        assert!((column_mask(col_right + gap, col_left, col_right, gap) - 1.0).abs() < 1e-4);
        // Deep in the margin: full strength.
        assert_eq!(column_mask(20.0, col_left, col_right, gap), 1.0);
    }

    #[test]
    fn column_mask_ramps_monotonically_across_the_feather() {
        let (col_left, col_right, gap) = (300.0, 900.0, 40.0);
        let mut prev = column_mask(col_left, col_left, col_right, gap);
        for k in 1..=40 {
            let x = col_left - k as f32; // stepping out into the left margin
            let m = column_mask(x, col_left, col_right, gap);
            assert!(
                m >= prev - 1e-6,
                "mask ramps monotonically at x={x}: {m} >= {prev}"
            );
            prev = m;
        }
        assert!(
            (prev - 1.0).abs() < 1e-4,
            "settled at full strength: {prev}"
        );
    }

    // --- The CADENCE gate -------------------------------------------------------

    #[test]
    fn lava_ticks_only_when_active_ambient_on_not_reduced_and_focused() {
        assert!(
            lava_should_tick(true, true, false, true),
            "all conditions met → tick"
        );
        // Each single negation kills the tick (0% idle preserved).
        assert!(
            !lava_should_tick(false, true, false, true),
            "non-lava world never ticks"
        );
        assert!(
            !lava_should_tick(true, false, false, true),
            "ambient_motion off → no tick"
        );
        assert!(
            !lava_should_tick(true, true, true, true),
            "reduce motion → no tick"
        );
        assert!(
            !lava_should_tick(true, true, false, false),
            "blurred → paused, no tick"
        );
    }

    // --- Phase resolution / determinism ----------------------------------------

    #[test]
    fn env_override_wins_then_reduced_freeze_then_stored() {
        // Env override wins outright (the gallery knob), regardless of reduced.
        assert_eq!(lava_phase_for(0.7, false, Some(0.35)), 0.35);
        assert_eq!(lava_phase_for(0.7, true, Some(0.35)), 0.35);
        // No env, reduced → frozen (the accessibility freeze).
        assert_eq!(lava_phase_for(0.7, true, None), LAVA_FROZEN_PHASE);
        // No env, not reduced → the App-driven stored phase.
        assert_eq!(lava_phase_for(0.7, false, None), 0.7);
    }

    #[test]
    fn capture_default_phase_is_frozen_t0() {
        // A headless capture never ticks (stored stays the construction default
        // 0.0) and never sets reduced() and never sets the env knob, so the
        // resolved phase is the fixed t=0 — deterministic across machines.
        assert_eq!(lava_phase_for(LAVA_FROZEN_PHASE, false, None), 0.0);
        assert_eq!(LAVA_FROZEN_PHASE, 0.0);
    }

    #[test]
    fn advance_phase_moves_forward_and_wraps_over_the_full_field_period() {
        let p = advance_phase(0.0, 1.0);
        assert!(
            p > 0.0 && p < LAVA_LOOP_CYCLES,
            "one second advances within a cycle: {p}"
        );
        // Wrapping: a phase already near the two-cycle endpoint wraps cleanly.
        let w = advance_phase(1.999, 1.0);
        assert!(
            (0.0..LAVA_LOOP_CYCLES).contains(&w),
            "wrapped into the two-cycle interval: {w}"
        );
        // Monotone within a cycle.
        assert!(advance_phase(0.1, 0.5) > 0.1);
    }

    #[test]
    fn two_cycle_endpoint_is_seamless_for_every_blob_center() {
        let vp = (1200.0, 800.0);
        for (i, b) in BACKDROP_BLOBS.iter().enumerate() {
            for start in [0.0, 0.17, 0.63, 1.21] {
                let a = animated_center(i, b[0], b[1], b[2], vp, start);
                let z = animated_center(
                    i,
                    b[0],
                    b[1],
                    b[2],
                    vp,
                    start + LAVA_LOOP_CYCLES,
                );
                assert!(
                    (a.0 - z.0).abs() < 1e-6 && (a.1 - z.1).abs() < 1e-6,
                    "blob {i} does not meet its two-cycle endpoint from {start}: {a:?} vs {z:?}"
                );
            }
        }
        // One cycle is deliberately NOT the full loop: horizontal sway is at
        // half-frequency, so at least one blob must still be elsewhere there.
        let b = BACKDROP_BLOBS[1];
        let at_zero = animated_center(1, b[0], b[1], b[2], vp, 0.0);
        let at_one = animated_center(1, b[0], b[1], b[2], vp, 1.0);
        assert!((at_zero.0 - at_one.0).abs() > 1e-4);

        // Centers are the field's only phase-varying input, but prove the
        // composed metaball result too so the law names the visible outcome.
        for px in [(24.0, 40.0), (160.0, 400.0), (600.0, 300.0), (1140.0, 720.0)] {
            let a = metaball_field(px, vp, &BACKDROP_BLOBS, 0.0);
            let z = metaball_field(px, vp, &BACKDROP_BLOBS, LAVA_LOOP_CYCLES);
            assert!(
                (a - z).abs() < 1e-6,
                "metaball field does not meet its two-cycle endpoint at {px:?}: {a} vs {z}"
            );
        }
    }

    #[test]
    fn delayed_ambient_ticks_advance_at_most_one_fixed_step() {
        assert_eq!(ambient_tick_dt(LAVA_TICK_SECONDS), LAVA_TICK_SECONDS);
        assert_eq!(ambient_tick_dt(8.0), LAVA_TICK_SECONDS);
        assert_eq!(ambient_tick_dt(-1.0), 0.0);

        let ordinary = advance_phase(0.4, LAVA_TICK_SECONDS);
        let delayed = advance_phase(0.4, 8.0);
        assert_eq!(
            delayed, ordinary,
            "an eight-second event-loop stall must advance exactly one ambient tick, never catch up"
        );
        assert!((ordinary - 0.4 - LAVA_TICK_SECONDS * LAVA_SPEED).abs() < 1e-6);
    }

    // --- The dev gallery knob ---------------------------------------------------

    #[test]
    fn parse_spec_reads_palette_phase_edge_and_dither() {
        let (bg, phase) = parse_spec("deepsea:0.35:glow:dither").unwrap();
        assert_eq!(phase, 0.35);
        match bg {
            Background::Lava { edge, dithered, .. } => {
                assert_eq!(edge, LavaEdge::Glow);
                assert!(dithered);
            }
            _ => panic!("expected a Lava background"),
        }
        // Defaults: no edge/dither tokens → glow, undithered.
        let (bg2, _) = parse_spec("warm:0.0").unwrap();
        match bg2 {
            Background::Lava { edge, dithered, .. } => {
                assert_eq!(edge, LavaEdge::Glow);
                assert!(!dithered);
            }
            _ => panic!("expected a Lava background"),
        }
        // Hard edge.
        let (bg3, _) = parse_spec("warm:0.5:hard").unwrap();
        assert!(matches!(
            bg3,
            Background::Lava {
                edge: LavaEdge::Hard,
                ..
            }
        ));
        // Garbage → None (leniently ignored; no lava forced).
        assert!(parse_spec("nope:0.0").is_none());
        assert!(parse_spec("warm:notanumber").is_none());
        assert!(parse_spec("warm:0.0:bogus").is_none());
    }
}
