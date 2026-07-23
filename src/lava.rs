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
    // COMPOSITION-C2: radii shrunk ~15% (user — "starting lava lamp spots… kinda
    // massive"). Centers/weights and the whole authored COMPOSITION are unchanged,
    // so more calm ground shows between/around the lamps while the layout still
    // reads as one continuous field. SHARED by both lava worlds (Firetail +
    // Mangrove read the same field — a per-world fork for the identical "a tad
    // smaller" ask would be machinery the design doesn't need; the shrink IS the
    // data). Every lava LAW (figure/ground at worst phase, rail-flat, frost,
    // seamless loop) re-verified green after the shrink.
    [0.08, 0.18, 0.120, 0.90],
    [0.16, 0.50, 0.153, 1.05],
    [0.12, 0.82, 0.136, 0.95],
    [0.38, 0.68, 0.178, 1.10],
    [0.58, 0.30, 0.170, 1.00],
    [0.86, 0.18, 0.120, 0.90],
    [0.82, 0.50, 0.153, 1.05],
    [0.88, 0.82, 0.136, 0.95],
];

// --- FROST constants (the shipped headed-doc treatment) -----------------------
//
// The user's design: on a lava world a HEADED doc keeps BOTH margins alive (the
// lamp animates in the left margin too), and the drawn margin ink (the outline
// entries + the bottom-left gutter) sits over a FROSTED FIELD — the SMOOTH
// metaball field softened + a value dim — so the dim ink keeps its contrast while
// the lamp stays fully alive around it. This REPLACES the old whole-left-margin
// CARVE (which flattened the entire rail to ground). All TASTE TUNABLE — flagged
// for live review, named like `THEME_FONT_DEBOUNCE`.
//
// ORGANIC GLYPH-SEEDED FIELD (item 32): the frost is NOT a union of rectangular
// pills. Every visible margin glyph SEEDS a small close halo; the halos are
// summed into ONE continuous scalar field and thresholded ([`frost_coverage`]),
// so neighbouring seeds MERGE in ANY direction wherever their softened halos
// overlap — nearby words / rows naturally become a larger organic island, a run
// of consecutive headings joins while a group-gapped one separates, with NO
// artificial per-row separation and NO zoom breakpoint (the topology falls out of
// the continuous field, never an `if zoom >= X` mode switch). Mangrove and
// Firetail share the SAME shape recipe; only the palette-derived colour differs.

/// THE SHIPPED HEADED-DOC TREATMENT. `true` (the user's pick — "both margins
/// alive on every doc") = the glyph-seeded FROST field, the lamp animating in both
/// margins. Flip this ONE const to `false` to revert to the OLD whole-left-margin
/// CARVE (the `rail` global + [`rail_dist_outside`] + `lava_rail_carved` stay
/// wired for exactly this one-line data revert): frost turns off and a headed
/// doc's whole left margin flattens back to the flat page ground.
pub const FROST_RAIL_DEFAULT: bool = true;

/// The VALUE DIM inside the frost field: how far the softened field is mixed back
/// toward the flat page `ground` (0 = the raw softened lamp, 1 = pure flat
/// ground). Sized so the dim margin ink clears its ink-ladder contrast floor
/// against the field at EVERY animation phase (law
/// `outline_frost_pills_keep_ink_contrast_on_every_lava_world`), while a whisper
/// of the softened lamp still reads behind the text.
pub const FROST_DIM: f32 = 0.65;

/// The frost BLUR kernel spacing (logical px, zoom/DPI-scaled by the caller): the
/// per-tap offset of the 3×3 cross [`frost_field`] averages the SMOOTH field over.
/// Averaging the raw undithered field (never the posterized color) is the
/// Mangrove REQUIREMENT — blurring the Bayer grid makes cross moiré (the
/// documented palette-blur lesson), so the frost samples the field, not the
/// dither.
pub const FROST_BLUR_PX: f32 = 5.0;

/// The frost SEED HALO SKIRT (logical px, zoom/DPI-scaled): a soft px pad added to
/// each seed's glyph-derived halo radius, so the field blends into the live lamp
/// instead of drawing a hard boundary. (Formerly the rectangular pill's edge
/// feather; now the halo skirt of the organic field.)
pub const FROST_FEATHER_PX: f32 = 7.0;

/// The horizontal padding (logical px, zoom/DPI-scaled) the seeded run extends past
/// each end of its shaped text extent — the "comfortable padding" that hugs the
/// text without clipping its antialiased edge.
pub const FROST_PILL_PAD_X: f32 = 6.0;

/// The vertical inset of a frost seed row from its full line box, as a fraction of
/// the row height (top AND bottom) — so seed y-centres hug the text band. (Kept
/// for the coarse outline-ink exclusion rects the STARS layer reads.)
pub const FROST_PILL_INSET_Y_FRAC: f32 = 0.1;

/// THE SEED HALO RADIUS as a fraction of the LABEL row height — the "small close
/// halo" each glyph seeds, DERIVED FROM THE ACTUAL ZOOMED GLYPH GEOMETRY (the row
/// height IS the zoomed line box). Tuned with [`FROST_ISO`] so glyph seeds within
/// a run always merge, consecutive rows (pitch one row height) merge into a larger
/// island, and a group-gapped row (pitch 1.5 rows) tends to separate — the natural
/// island structure, with no explicit per-row rule.
pub const FROST_SEED_RADIUS_FRAC: f32 = 0.62;

/// THE FIELD ISO LEVEL: the summed seed halo strength at which the organic frost
/// coverage crosses 0.5. A lone seed peaks at 1.0 at its core, so ISO < 1 gives a
/// halo out past the seed; two seeds each contributing ~ISO/2 in the gap between
/// them bridge into one island (the metaball neck). MUST match `shaders/lava.wgsl`.
pub const FROST_ISO: f32 = 0.55;

/// THE ISO SOFT BAND: the summed-field half-width over which coverage ramps
/// 0 → 1 around [`FROST_ISO`] — the organic edge softness (never a hard contour).
/// MUST match `shaders/lava.wgsl`.
pub const FROST_ISO_SOFT: f32 = 0.42;

/// SEED GRANULARITY (item 32 perf arm). `true` = PER-GLYPH seeds (one halo per
/// visible glyph — the ideal bumpy hug). `false` = the NAMED DEGRADATION ARM:
/// one WORD-RUN seed per whitespace-delimited run, still merging organically in
/// any direction, far fewer per-pixel seeds. Flipped to the degradation arm only
/// if the per-glyph steady frame cost regresses > 5% (item 32 STEP 3).
pub const FROST_SEED_PER_GLYPH: bool = false;

/// Convert an authored Frost dimension from logical to physical pixels. The lava
/// shader consumes physical pixels, so its blur, skirt, and pad must use the same
/// user-zoom × device-DPI scale as [`crate::render::Metrics::with_dpi`].
pub fn frost_px(logical_px: f32, zoom: f32, dpi: f32) -> f32 {
    logical_px * zoom * dpi
}

/// The MAX frost SEEDS the shader's uniform carries (`array<vec4<f32>,
/// MAX_FROST_SEEDS>`). Per-glyph seeding over a full followed outline plus the
/// gutter can reach into the low hundreds; the field clamps here (a generous cap,
/// far above any realistic margin-ink glyph budget).
pub const MAX_FROST_SEEDS: usize = 256;

/// The MAX outline-ink exclusion RECTS the STARS layer reads (one per drawn
/// outline row) — unchanged from the pre-organic-field cap, kept for that coarse
/// keep-out geometry (see [`crate::render::TextPipeline::lava_frost_pill_rects`]).
pub const MAX_FROST_PILLS: usize = 48;

#[allow(dead_code)] // shader-mirror constant (see the pure-math note below).
const TAU: f32 = std::f32::consts::TAU;

// Frost blend constants — MUST match `shaders/lava.wgsl`'s `THRESHOLD` /
// `EDGE_WIDTH` / `CORE_WIDTH` (the metaball edge/core smoothstep bands the frost
// pixel maps the softened field through).
const FROST_THRESHOLD: f32 = 0.5;
const FROST_EDGE_WIDTH: f32 = 0.12;
const FROST_CORE_WIDTH: f32 = 0.35;

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

/// The signed "distance outside the no-lava zones" at pixel x (px): positive out
/// in a lava-bearing margin, <= 0 inside a zone the field must vanish from.
/// Ordinarily the one zone is the writing column `[col_left, col_right]` (both
/// edges via `max()`). With the LEFT-MARGIN RAIL carved (`rail_carved` — the
/// margin OUTLINE is actually DRAWN this frame, a HEADED doc, see
/// `TextPipeline::lava_rail_carved`) the whole LEFT margin joins the no-lava
/// zone: only the RIGHT margin's distance counts, so the outline's dim entries
/// sit on the flat ground at every phase and the lamp keeps the right margin
/// (the outline hiding reclaims the full margin). The bottom-left GUTTER no
/// longer flattens the whole margin — it drives the bounded
/// [`gutter_corner_dist_outside`] carve instead, so an ordinary (gutter-only)
/// doc keeps BOTH margins. The carve also feeds the Glow treatment's
/// `could_glow` through this same distance, so no unexplained edge-bleed tints
/// the page next to a flat rail. MUST match `shaders/lava.wgsl`'s `dist_outside`.
#[allow(dead_code)]
pub fn rail_dist_outside(x: f32, col_left: f32, col_right: f32, rail_carved: bool) -> f32 {
    if rail_carved {
        x - col_right
    } else {
        (col_left - x).max(x - col_right)
    }
}

/// The MARGINS-ONLY lava coverage mask at pixel x (px): 0 inside every no-lava
/// zone ([`rail_dist_outside`] — the writing column, plus the whole left margin
/// while the outline rail is carved) and at its edge, ramping to 1 a `gap` px
/// further out. The lava is drawn at this coverage, so the field fades entirely
/// outside the zones and the page (and a carved rail) stays a clean flat
/// ground. MUST match `shaders/lava.wgsl`'s `mask`. `gap` is floored at 1.0
/// (matching the shader).
#[allow(dead_code)]
pub fn lava_mask(x: f32, col_left: f32, col_right: f32, gap: f32, rail_carved: bool) -> f32 {
    smoothstep(
        0.0,
        gap.max(1.0),
        rail_dist_outside(x, col_left, col_right, rail_carved),
    )
}

/// The plain (un-carved) column mask — [`lava_mask`] with no outline rail, kept
/// as the named identity the page-width-invariance tests read.
#[allow(dead_code)]
pub fn column_mask(x: f32, col_left: f32, col_right: f32, gap: f32) -> f32 {
    lava_mask(x, col_left, col_right, gap, false)
}

/// The signed "distance outside the GUTTER's local corner rect" at pixel
/// `(x, y)` (px): <= 0 inside the bounded bottom-left region the gutter owns,
/// positive out beyond it. `rect` is `[left, top, right, bottom]`. The per-axis
/// outside distances (negative inside the span) are combined by `max`, so the
/// result is negative iff BOTH axes are inside — the box interior — and its
/// magnitude just outside a face is the perpendicular distance to that face.
/// Unlike [`rail_dist_outside`] (which flattens the WHOLE left margin for a
/// headed doc), this carves only a bounded corner, leaving the rest of both
/// margins their lamp. MUST match `shaders/lava.wgsl`'s `gutter_dist_outside`.
#[allow(dead_code)]
pub fn gutter_corner_dist_outside(x: f32, y: f32, rect: [f32; 4]) -> f32 {
    let gx = (rect[0] - x).max(x - rect[2]);
    let gy = (rect[1] - y).max(y - rect[3]);
    gx.max(gy)
}

/// The full 2-D lava coverage mask at pixel `(x, y)`: the 1-D margin mask
/// ([`lava_mask`] — the writing column plus, when `rail_carved`, the whole left
/// margin) further multiplied by the GUTTER's local corner carve when
/// `gutter_rect` is `Some` (the field vanishes over the bounded bottom-left
/// region, feathered over `gap` px at its top/right faces). This is the exact
/// SHIP-path mirror of `shaders/lava.wgsl`'s `fs_main` mask (probe mode 0): with
/// `gutter_rect = None` it is byte-for-byte [`lava_mask`]. MUST stay in lockstep
/// with the shader.
#[allow(dead_code)]
pub fn lava_mask_2d(
    x: f32,
    y: f32,
    col_left: f32,
    col_right: f32,
    gap: f32,
    rail_carved: bool,
    gutter_rect: Option<[f32; 4]>,
) -> f32 {
    let base = lava_mask(x, col_left, col_right, gap, rail_carved);
    match gutter_rect {
        Some(r) => base * smoothstep(0.0, gap.max(1.0), gutter_corner_dist_outside(x, y, r)),
        None => base,
    }
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

// --- FROST pure math (the shader mirror, unit-tested) -------------------------
//
// The FROST treatment (behind the margin ink): a SOFTENED sample of the SMOOTH
// metaball field ([`frost_field`], a 3×3 tap average — never the dithered color,
// the Mangrove palette-blur lesson) mapped through the same edge/core blend the
// lamp uses, then value-dimmed toward the flat ground ([`frost_pixel`]). Blended
// into the live lamp by the ORGANIC glyph-seeded coverage — every seed contributes
// a soft halo ([`frost_seed_bump`]), the halos SUM into one continuous field and
// threshold ([`frost_coverage`]) so neighbours merge in any direction. All four
// MUST stay in lockstep with `shaders/lava.wgsl`.

/// The SOFTENED (blurred) metaball field at pixel `px`: [`metaball_field`]
/// averaged over a 3×3 tap cross at `blur` px spacing. Averaging the RAW field
/// (undithered) widens each blob's apparent edge without ever sampling the Bayer
/// grid — the Mangrove REQUIREMENT (blurring the ordered-dither grid makes cross
/// moiré). MUST match `shaders/lava.wgsl`'s `frost_field`. Pure.
#[allow(dead_code)]
pub fn frost_field(
    px: (f32, f32),
    viewport: (f32, f32),
    blobs: &[[f32; 4]],
    phase: f32,
    blur: f32,
) -> f32 {
    let mut acc = 0.0;
    for oy in [-blur, 0.0, blur] {
        for ox in [-blur, 0.0, blur] {
            acc += metaball_field((px.0 + ox, px.1 + oy), viewport, blobs, phase);
        }
    }
    acc / 9.0
}

/// The FROST PILL PIXEL (sRGB): the softened `field` mapped through the lamp's
/// own edge/core blend (`ground → blob_lo → blob_hi`), then VALUE-DIMMED toward
/// the flat `ground` by `dim` so the dim outline ink keeps its contrast. The
/// blend is computed in sRGB (the documented approximation the sibling lava
/// figure/ground law uses — the shader mixes in linear, but the tones are dark +
/// close so the perceptual gap is negligible; the law asserts the contrast floor
/// over these values directly). MUST match `shaders/lava.wgsl`'s frost color path.
#[allow(dead_code)]
pub fn frost_pixel(field: f32, ground: Srgb, blob_lo: Srgb, blob_hi: Srgb, dim: f32) -> Srgb {
    let edge_t = smoothstep(FROST_THRESHOLD - FROST_EDGE_WIDTH, FROST_THRESHOLD + FROST_EDGE_WIDTH, field);
    let core_t = smoothstep(FROST_THRESHOLD, FROST_THRESHOLD + FROST_CORE_WIDTH, field);
    let lerp = |a: u8, b: u8, t: f32| -> u8 { (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8 };
    let ch = |gc: u8, lo: u8, hi: u8| -> u8 {
        let blob = lerp(lo, hi, core_t); // blob_lo → blob_hi by core_t
        let smooth = lerp(gc, blob, edge_t); // ground → blob by edge_t
        lerp(smooth, gc, dim) // value dim back toward the flat ground
    };
    Srgb {
        r: ch(ground.r, blob_lo.r, blob_hi.r),
        g: ch(ground.g, blob_lo.g, blob_hi.g),
        b: ch(ground.b, blob_lo.b, blob_hi.b),
        a: 0xFF,
    }
}

/// THE SEED HALO BUMP at pixel `(x, y)` for one glyph/word SEED `[x0, x1, yc, r]`
/// (a horizontal run `[x0, x1]` at row centre `yc`, halo radius `r` px): a compact
/// support soft bump — `(1 - (d/r)^2)^2` where `d` is the distance to the run
/// segment (0 within the run's x-span), so it is `1` at the ink and `0` past a
/// radius, C1-smooth. Summed across seeds it is the metaball field that MERGES
/// neighbours. MUST match `shaders/lava.wgsl`'s `seed_bump`. Pure.
#[allow(dead_code)]
pub fn frost_seed_bump(x: f32, y: f32, seed: [f32; 4]) -> f32 {
    let dx = (seed[0] - x).max(x - seed[1]).max(0.0);
    let dy = y - seed[2];
    let r = seed[3].max(1.0);
    let t = (1.0 - (dx * dx + dy * dy) / (r * r)).clamp(0.0, 1.0);
    t * t
}

/// THE ORGANIC FROST COVERAGE at pixel `(x, y)`: the seed halos SUMMED into one
/// continuous field, then thresholded at [`FROST_ISO`] over the [`FROST_ISO_SOFT`]
/// band. Because the halos SUM (never a max/union), two nearby seeds bridge into
/// one island wherever their combined field clears the iso — so glyphs within a
/// run, and nearby rows, merge in ANY direction with no per-row rule. An EMPTY
/// seed list is a total no-op (0 everywhere) — the inert non-frost frame. MUST
/// match `shaders/lava.wgsl`'s seed loop. Pure.
#[allow(dead_code)]
pub fn frost_coverage(x: f32, y: f32, seeds: &[[f32; 4]]) -> f32 {
    let mut field = 0.0f32;
    for s in seeds {
        field += frost_seed_bump(x, y, *s);
    }
    smoothstep(FROST_ISO - FROST_ISO_SOFT, FROST_ISO + FROST_ISO_SOFT, field)
}

// --- CADENCE / PHASE resolution (pure, unit-tested) ---------------------------

/// THE CADENCE GATE: may the live App arm its slow ambient lava tick THIS frame?
/// True ONLY when a lava world is active AND ambient motion is on AND motion is
/// NOT reduced AND the window is focused and unobstructed (pause on frost,
/// resize, and move). A non-lava world
/// (`active == false`) is always false, so it schedules ZERO extra frames —
/// preserving 0% idle CPU. Pure, so the whole gate is unit-testable.
pub fn lava_should_tick(
    active: bool,
    ambient_on: bool,
    reduced: bool,
    focused: bool,
    paused: bool,
) -> bool {
    active && ambient_on && !reduced && focused && !paused
}

/// THE PAUSE COMPOSITION the cadence gate's `paused` term is fed from — ONE
/// owner of "which transient live interactions hold the lamp": an active
/// RESIZE stream, an active MOVE stream, or a blur-eligible overlay (frost).
/// Any of the three holds the phase (and, since [`lava_should_tick`] is the
/// only door to `advance_lava`, the field with it) without resetting it. Pure,
/// so the OR-composition itself is law-testable — the live App reads its three
/// inputs off `resize_settle_at` / `move_settle_at` / `lava_blur_active()`.
pub fn lava_paused(resizing: bool, moving: bool, blurred: bool) -> bool {
    resizing || moving || blurred
}

// THE PREVIEW-CROSSING CLASSIFICATION IS RETIRED (2026-07-18). It once decided
// whether a theme-preview step got the present-transaction bracket by testing the
// OUTGOING/INCOMING pair against a HEAVYWEIGHT-PIPELINE boundary (ambient cadence
// or one-bit pipeline). A live probe of the reported Mangrove→Magpie gesture
// proved the classification structurally wrong: the actual LANDING step
// (`Galah→Magpie`) is same-side on both boundaries → it read `Steady` and armed
// NO bracket, so the landing frame presented unbracketed while the bracket that
// did arm (on the transient `Mangrove→Galah` boundary earlier in the nav) had
// already torn down. Three successive widenings (is_lava → ambient → one-bit)
// chased the boundary and never covered the landing. The fix is the simpler
// truth: `App::retint_theme_preview` arms the bracket on EVERY preview step
// unconditionally, and the teardown is event-ordered (it waits for the reshaped
// frame's in-bracket present) rather than a per-crossing timer. There is no
// per-pair decision left to make, so the pure fn + its `CrossingAction` enum are
// gone; `has_ambient_motion` / `is_one_bit` survive for their other callers.

/// Choose the viewport used to lay out the metaball field. During a live resize
/// the last-settled dimensions are held while the live viewport and column mask
/// continue to follow the window; the new dimensions become authoritative only
/// on settle.
pub fn field_viewport(live: [f32; 2], settled: [f32; 2]) -> [f32; 2] {
    if settled[0] > 0.0 && settled[1] > 0.0 {
        settled
    } else {
        live
    }
}

/// The blur capture consumes a smooth lava source. Ordered posterization is an
/// authored live-world treatment, but its axis-aligned grid aliases with the
/// downsampled separable frost and produces crosses; outside capture it remains
/// exactly as the world requested.
pub fn dither_for_blur(authored: bool, backdrop_blur: bool) -> bool {
    authored && !backdrop_blur
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

// --- The dev-only FROST-OFF gallery knob (AWL_LAVA_FROST=off) ------------------
//
// Mirrors the `AWL_LAVA` / `AWL_CJK_FORCE` precedent: read ONCE at startup,
// memoized, a TOTAL no-op unless set, so ship + headless determinism is untouched
// when absent. The ONLY knob kept — the vetoed plate/band/bleed both-sides
// auditions were deleted (the user picked FROST). `AWL_LAVA_FROST=off` turns the
// frost pills OFF so the A/B "before" (the outline sitting on the raw, unfrosted
// lamp — why frost earns its place) stays producible for a gallery.

/// Whether the dev-only `AWL_LAVA_FROST` env knob was set to `off` at startup —
/// the A/B "before" (frost pills suppressed). Read once, memoized. A no-op
/// (returns `false`) unless set, so every normal + headless run frosts by default.
fn frost_env_off() -> bool {
    static ONCE: OnceLock<bool> = OnceLock::new();
    *ONCE.get_or_init(|| {
        std::env::var("AWL_LAVA_FROST")
            .ok()
            .as_deref()
            .map(|v| v.trim().eq_ignore_ascii_case("off"))
            .unwrap_or(false)
    })
}

/// Whether per-entry FROST is active this run: the shipped default
/// ([`FROST_RAIL_DEFAULT`]) UNLESS the dev-only `AWL_LAVA_FROST=off` gallery knob
/// suppressed it. When off, a headed lava doc's outline sits on the raw lamp (the
/// A/B "before"); when the const is flipped to `false`, frost is off AND the old
/// whole-margin carve returns (see [`FROST_RAIL_DEFAULT`]).
pub fn frost_on() -> bool {
    FROST_RAIL_DEFAULT && !frost_env_off()
}

// --- The wgpu pipeline --------------------------------------------------------

/// Uniform globals. MUST match `Globals` in `shaders/lava.wgsl`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Globals {
    viewport: [f32; 2],
    field_viewport: [f32; 2],
    blob_count: u32,
    dither: u32,
    /// 1 = the margin OUTLINE is drawn this frame (a HEADED doc), so the whole
    /// LEFT margin is its rail — carved out of the field mask (the conservative
    /// FULL carve, see [`rail_dist_outside`]). The bottom-left GUTTER no longer
    /// gates this full carve; it drives the LOCAL corner carve below instead, so
    /// an ordinary (gutter-only) doc keeps BOTH margins their lamp.
    rail: u32,
    /// 1 = the bottom-left GUTTER is drawn this frame, so a bounded LOCAL corner
    /// region around it ([`Globals::gutter_rect`]) is carved out of the field
    /// mask while the rest of both margins keep the lamp. MUST match
    /// `shaders/lava.wgsl`'s `gutter` gate + [`gutter_corner_dist_outside`].
    gutter: u32,
    /// `[col_left_px, col_right_px, gap_px, mask_mode]` — `mask_mode` from
    /// [`LavaEdge::mask_mode`] (1.0 hard, 2.0 glow).
    margin: [f32; 4],
    /// `[phase, 0, 0, 0]` — phase in cycles.
    anim: [f32; 4],
    ground: [f32; 4],
    blob_lo: [f32; 4],
    blob_hi: [f32; 4],
    blobs: [[f32; 4]; MAX_BLOBS],
    /// The GUTTER's local corner carve rect `[left, top, right, bottom]` (px) —
    /// the bounded bottom-left region the field vanishes from when `gutter == 1`.
    /// All-zero when there is no gutter carve. See [`gutter_corner_dist_outside`].
    gutter_rect: [f32; 4],
    /// FROST params `[dim, blur_px, iso_unused, seed_count]`: the organic
    /// glyph-seeded field treatment (the shipped headed-doc default). `seed_count`
    /// (the trailing float) is how many of [`Globals::seeds`] are live — `0` in
    /// every non-frost frame (non-lava world, no margin ink, or `AWL_LAVA_FROST=off`),
    /// so the whole frost path is inert. See [`frost_pixel`] / [`frost_coverage`].
    frost: [f32; 4],
    /// The FROST SEEDS `[x0, x1, yc, r]` (px), one per visible margin glyph (or
    /// per word-run under the degradation arm) — the halos whose SUMMED field the
    /// lava renders FROSTED behind. Only the first `frost.w` are live (all-zero
    /// otherwise). See [`MAX_FROST_SEEDS`] / [`frost_seed_bump`].
    seeds: [[f32; 4]; MAX_FROST_SEEDS],
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
    /// `column_width`, the one geometry owner), whether the whole LEFT margin is
    /// carved out of the mask this frame (`rail_carved`, from
    /// `TextPipeline::lava_rail_carved` — the margin OUTLINE's own draw gate, so
    /// the full carve can never disagree with what the frame draws), the GUTTER's
    /// bounded LOCAL corner carve rect (`gutter_rect`, `Some` iff the gutter
    /// draws — from `TextPipeline::lava_gutter_carve_rect`), the effective
    /// `phase`, the organic FROST `frost_seeds` (the visible margin glyphs' halo
    /// seeds `[x0, x1, yc, r]` — empty in every non-frost frame, so the frost path
    /// is inert) plus their `[dim, blur_px, iso]` params, and (for the one-line
    /// carve revert) whether the whole LEFT margin is carved (`rail_carved`, `false`
    /// under the frost default).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        settled_field_viewport: [f32; 2],
        col_left: f32,
        col_w: f32,
        rail_carved: bool,
        gutter_rect: Option<[f32; 4]>,
        frost_seeds: &[[f32; 4]],
        frost_params: [f32; 3],
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
            field_viewport: field_viewport(
                [width as f32, height as f32],
                settled_field_viewport,
            ),
            blob_count: BACKDROP_BLOBS.len() as u32,
            dither: dithered as u32,
            rail: rail_carved as u32,
            gutter: gutter_rect.is_some() as u32,
            margin: [col_left, col_left + col_w, MARGIN_GAP_PX, edge.mask_mode()],
            anim: [phase, 0.0, 0.0, 0.0],
            ground: srgb_u8_to_linear(ground),
            blob_lo: srgb_u8_to_linear(blob_lo),
            blob_hi: srgb_u8_to_linear(blob_hi),
            blobs,
            gutter_rect: gutter_rect.unwrap_or([0.0; 4]),
            frost: {
                let n = frost_seeds.len().min(MAX_FROST_SEEDS);
                [frost_params[0], frost_params[1], frost_params[2], n as f32]
            },
            seeds: {
                let mut ss = [[0.0f32; 4]; MAX_FROST_SEEDS];
                for (dst, src) in ss.iter_mut().zip(frost_seeds.iter()) {
                    *dst = *src;
                }
                ss
            },
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
                b[2] * vp.1 >= 90.0,
                "backdrop blob is substantial at 1200×800 (floor lowered from 100 \
                 to 90 for the COMPOSITION-C2 ~15% shrink — still a real lamp, not a dot)"
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

    /// THE OUTLINE-RAIL CARVE at the mask seam: `rail_carved = false` is the
    /// exact identity (byte-for-byte the plain column mask — every pre-carve
    /// frame is untouched); `rail_carved = true` zeroes the ENTIRE left margin
    /// (the rail) while the right margin stays byte-identical to the uncarved
    /// mask — the lamp moves over, it doesn't dim. Phase never enters the mask
    /// (a pure function of x), so the carve is phase-independent by
    /// construction.
    #[test]
    fn rail_carve_flattens_the_left_margin_and_keeps_the_right_byte_identical() {
        let (col_left, col_right, gap) = (300.0, 900.0, MARGIN_GAP_PX);
        // OFF is the identity with the plain column mask, everywhere.
        for x in [0.0, 20.0, 150.0, 272.0, 285.0, 300.0, 600.0, 900.0, 914.0, 928.0, 1100.0] {
            assert_eq!(
                lava_mask(x, col_left, col_right, gap, false),
                column_mask(x, col_left, col_right, gap),
                "rail off is the plain column mask at x={x}"
            );
        }
        // ON: every pixel of the LEFT margin — including the deep margin where
        // the uncarved mask is FULL strength — is a no-lava zone.
        for x in [0.0, 5.0, 20.0, 150.0, 271.9, 285.0, 299.0] {
            assert_eq!(
                lava_mask(x, col_left, col_right, gap, true),
                0.0,
                "the rail band holds no lava at x={x}"
            );
        }
        // Witness the carve does real work: the uncarved mask WOULD paint there.
        assert_eq!(column_mask(150.0, col_left, col_right, gap), 1.0);
        // The column itself stays clear (unchanged) ...
        assert_eq!(lava_mask(600.0, col_left, col_right, gap, true), 0.0);
        // ... and the RIGHT margin (edge, feather, deep) is byte-identical.
        for x in [900.0, 910.0, 914.0, 928.0, 1000.0, 1199.0] {
            assert_eq!(
                lava_mask(x, col_left, col_right, gap, true),
                column_mask(x, col_left, col_right, gap),
                "the right margin keeps the lamp untouched at x={x}"
            );
        }
    }

    /// The carved distance also owns the Glow treatment's `could_glow` gate in
    /// the shader: with the rail carved, every left-margin AND left-edge pixel
    /// sits far below the glow bleed window (`dist_outside <= -(column width)`),
    /// so no under-glass bleed can tint the page beside a flat rail; the right
    /// edge's distance is byte-identical to the uncarved one.
    #[test]
    fn rail_carve_moves_the_glow_distance_off_the_left_edge() {
        let (col_left, col_right) = (300.0, 900.0);
        // Just inside the LEFT edge (the old glow window, x a shade past
        // col_left): carved distance is nearly the full column width away —
        // structurally outside any bleed window.
        for x in [301.0, 330.0, 355.0] {
            let carved = rail_dist_outside(x, col_left, col_right, true);
            assert!(
                carved < -100.0,
                "left-edge glow is structurally unreachable when carved: x={x} dist={carved}"
            );
            // The uncarved distance sat within a plausible bleed window there.
            let plain = rail_dist_outside(x, col_left, col_right, false);
            assert!(plain > -60.0 && plain < 0.0, "uncarved x={x} dist={plain}");
        }
        // Just inside the RIGHT edge: identical either way (the right glow stays).
        for x in [850.0, 875.0, 899.0] {
            assert_eq!(
                rail_dist_outside(x, col_left, col_right, true),
                rail_dist_outside(x, col_left, col_right, false),
                "right-edge glow distance unchanged at x={x}"
            );
        }
    }

    /// THE GUTTER LOCAL CORNER CARVE at the mask seam (the shader mirror,
    /// `lava_mask_2d` / `gutter_corner_dist_outside`): a bounded bottom-left rect
    /// is carved to zero INSIDE its bounds, while OUTSIDE (the upper-left margin
    /// and the right margin) the lamp is byte-for-byte the un-carved mask — the
    /// whole point, so an ordinary (gutter-only) doc keeps BOTH margins. Unlike
    /// the outline's full carve, the left margin ABOVE the corner band is
    /// untouched.
    #[test]
    fn gutter_corner_carve_zeroes_only_its_bounds_and_keeps_both_margins() {
        let (col_left, col_right, gap) = (300.0, 900.0, MARGIN_GAP_PX);
        // A bottom-left corner rect: x in [0, 260], y in [820, 1000] (a bottom
        // band a shade shy of the column, the gutter's own box).
        let rect = [0.0, 820.0, 260.0, 1000.0];
        // `None` is the exact 1-D identity everywhere (no gutter → nothing new).
        for &(x, y) in &[(20.0, 900.0), (150.0, 400.0), (600.0, 500.0), (1000.0, 900.0)] {
            assert_eq!(
                lava_mask_2d(x, y, col_left, col_right, gap, false, None),
                column_mask(x, col_left, col_right, gap),
                "gutter None is the plain column mask at ({x},{y})"
            );
        }
        // INSIDE the corner rect (well past the feathered faces): the mask is 0,
        // even where the un-carved left margin is FULL strength.
        for &(x, y) in &[(20.0, 970.0), (120.0, 900.0), (200.0, 860.0)] {
            assert_eq!(column_mask(x, col_left, col_right, gap), 1.0);
            assert_eq!(
                lava_mask_2d(x, y, col_left, col_right, gap, false, Some(rect)),
                0.0,
                "the gutter corner band holds no lava at ({x},{y})"
            );
        }
        // ABOVE the band in the LEFT margin: the lamp is untouched (both margins
        // reclaimed — the corner carve is LOCAL, not the whole margin).
        for &(x, y) in &[(20.0, 200.0), (150.0, 400.0), (120.0, 600.0)] {
            assert_eq!(
                lava_mask_2d(x, y, col_left, col_right, gap, false, Some(rect)),
                column_mask(x, col_left, col_right, gap),
                "the left margin above the gutter band keeps its lamp at ({x},{y})"
            );
        }
        // The RIGHT margin, at every y, is byte-identical to the un-carved mask.
        for &(x, y) in &[(950.0, 900.0), (1000.0, 970.0), (1100.0, 500.0)] {
            assert_eq!(
                lava_mask_2d(x, y, col_left, col_right, gap, false, Some(rect)),
                column_mask(x, col_left, col_right, gap),
                "the right margin keeps its lamp beside a gutter corner carve at ({x},{y})"
            );
        }
    }

    /// The gutter corner distance is a true box "outside distance": <= 0 strictly
    /// inside, positive just outside each face, and it feathers the carve over
    /// `gap` px at the top/right faces (the canvas-corner left/bottom faces sit
    /// off-screen). Mirrors `shaders/lava.wgsl`'s `rect_dist_outside`.
    #[test]
    fn gutter_corner_dist_outside_is_a_box_signed_distance() {
        let rect = [0.0, 820.0, 260.0, 1000.0];
        // Deep interior: negative.
        assert!(gutter_corner_dist_outside(120.0, 900.0, rect) < 0.0);
        // Just outside the RIGHT face by 10px → +10 (top-face term is negative).
        assert!((gutter_corner_dist_outside(270.0, 900.0, rect) - 10.0).abs() < 1e-4);
        // Just outside the TOP face by 20px → +20 (right-face term is negative).
        assert!((gutter_corner_dist_outside(120.0, 800.0, rect) - 20.0).abs() < 1e-4);
        // A full gap past the top face → the mask has ramped to full lamp.
        let (col_left, col_right, gap) = (300.0, 900.0, MARGIN_GAP_PX);
        let above = 820.0 - gap - 1.0;
        assert!(
            (lava_mask_2d(120.0, above, col_left, col_right, gap, false, Some(rect))
                - column_mask(120.0, col_left, col_right, gap))
            .abs()
                < 1e-4,
            "a full gap above the corner band the lamp is back to full"
        );
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
            lava_should_tick(true, true, false, true, false),
            "all conditions met → tick"
        );
        // Each single negation kills the tick (0% idle preserved).
        assert!(
            !lava_should_tick(false, true, false, true, false),
            "non-lava world never ticks"
        );
        assert!(
            !lava_should_tick(true, false, false, true, false),
            "ambient_motion off → no tick"
        );
        assert!(
            !lava_should_tick(true, true, true, true, false),
            "reduce motion → no tick"
        );
        assert!(
            !lava_should_tick(true, true, false, false, false),
            "blurred → paused, no tick"
        );
        assert!(
            !lava_should_tick(true, true, false, true, true),
            "resize, move, or blur pause holds phase"
        );
    }

    #[test]
    fn any_transient_live_interaction_pauses_the_lamp() {
        // The OR-composition the live App feeds `lava_should_tick`'s `paused`
        // term (previously inline in `about_to_wait`, untested in isolation):
        // each transient interaction alone must hold the lamp.
        assert!(
            !lava_paused(false, false, false),
            "truly idle: the lamp may drift"
        );
        assert!(lava_paused(true, false, false), "a live RESIZE stream holds it");
        assert!(lava_paused(false, true, false), "a live MOVE stream holds it");
        assert!(
            lava_paused(false, false, true),
            "a blur-eligible overlay (frost) holds it"
        );
        // And composed pauses (a corner drag streams resize AND move) hold too.
        assert!(lava_paused(true, true, false));
    }

    #[test]
    fn field_viewport_holds_settled_geometry_until_explicit_snap() {
        let mut settled = [1200.0, 800.0];
        assert_eq!(field_viewport([1320.0, 840.0], settled), settled);
        assert_eq!(
            field_viewport([1400.0, 900.0], settled),
            settled,
            "successive resize ticks keep the same field"
        );
        settled = [1400.0, 900.0];
        assert_eq!(
            field_viewport([1400.0, 900.0], settled),
            [1400.0, 900.0],
            "settle snaps exactly once to the final viewport"
        );
        assert_eq!(
            field_viewport([1400.0, 900.0], [0.0, 0.0]),
            [1400.0, 900.0],
            "first frame falls back to live geometry"
        );
    }

    #[test]
    fn blur_capture_relaxes_only_the_lava_posterization_invariant() {
        assert!(dither_for_blur(true, false), "live Mangrove stays dithered");
        assert!(!dither_for_blur(true, true), "frost source is smooth");
        assert!(!dither_for_blur(false, false), "Firetail stays smooth");
        assert!(!dither_for_blur(false, true), "blur never invents dither");
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

    // --- FROST pill mirror (the shader-mirror laws, `shaders/lava.wgsl`) --------

    /// FROST is the shipped headed-doc default, active unless the dev knob is off.
    #[test]
    fn frost_is_the_shipped_default() {
        assert!(FROST_RAIL_DEFAULT, "the user's pick — frost ships");
        // No `AWL_LAVA_FROST` set in the test env → frost is on.
        assert!(frost_on(), "frost is on by default (no gallery knob)");
    }

    /// THE FROST DPI LAW: a 2× surface must receive 2× physical blur taps,
    /// feather, and pill padding at the same zoom, preserving the 1× logical feel.
    #[test]
    fn frost_dimensions_scale_with_zoom_and_device_dpi() {
        for zoom in [0.8_f32, 1.0, 1.25] {
            for logical in [FROST_BLUR_PX, FROST_FEATHER_PX, FROST_PILL_PAD_X] {
                let one = frost_px(logical, zoom, 1.0);
                let two = frost_px(logical, zoom, 2.0);
                assert!(
                    (two - 2.0 * one).abs() < f32::EPSILON,
                    "logical Frost dimension {logical} at zoom {zoom}: 2× physical {two} \
                     must be exactly twice 1× {one}"
                );
            }
        }
    }

    /// THE FROST BLUR: [`frost_field`] averages the SMOOTH field over a 3×3 tap
    /// cross, so a blob center softens (peak drops below the raw peak) while a
    /// point on bare ground stays ~0 — a genuine blur of the field, never the
    /// dither.
    #[test]
    fn frost_field_softens_the_smooth_field() {
        let blobs = [[0.5f32, 0.5, 0.1, 1.0]];
        let vp = (1000.0, 800.0);
        let center = animated_center(0, 0.5, 0.5, 0.1, vp, 0.0);
        let cpx = (center.0 * vp.0, center.1 * vp.1);
        let raw = metaball_field(cpx, vp, &blobs, 0.0);
        let soft = frost_field(cpx, vp, &blobs, 0.0, FROST_BLUR_PX);
        assert!(soft > 0.0 && soft < raw, "blurred peak sits below the raw peak: {soft} < {raw}");
        // Far from any blob: the blurred field is still negligible (no invented light).
        let far = frost_field((cpx.0 + 400.0, cpx.1), vp, &blobs, 0.0, FROST_BLUR_PX);
        assert!(far < 0.01, "bare ground stays dark under the blur: {far}");
    }

    /// THE SEED HALO BUMP: [`frost_seed_bump`] is 1 on the ink run, decays
    /// smoothly to 0 by a radius, and is symmetric about the run — the soft halo
    /// each glyph seeds.
    #[test]
    fn frost_seed_bump_is_one_on_the_ink_and_decays_to_zero_by_a_radius() {
        // A run [100, 300] at yc=230, radius 40.
        let seed = [100.0f32, 300.0, 230.0, 40.0];
        // On the run, at the row centre → full core bump.
        assert!((frost_seed_bump(200.0, 230.0, seed) - 1.0).abs() < 1e-6, "1 on the ink");
        assert!((frost_seed_bump(100.0, 230.0, seed) - 1.0).abs() < 1e-6, "1 at the run's left end");
        // Past a radius beyond the end / above the row → 0 (compact support).
        assert_eq!(frost_seed_bump(300.0 + 41.0, 230.0, seed), 0.0, "0 past a radius right");
        assert_eq!(frost_seed_bump(200.0, 230.0 + 41.0, seed), 0.0, "0 past a radius up");
        // Monotone decay stepping out past the right end.
        let mut prev = 1.0;
        for k in 0..=42 {
            let x = 300.0 + k as f32;
            let b = frost_seed_bump(x, 230.0, seed);
            assert!(b <= prev + 1e-6, "bump decays monotonically at x={x}: {b} <= {prev}");
            prev = b;
        }
        // Vertically symmetric about the row centre.
        assert!(
            (frost_seed_bump(200.0, 230.0 - 20.0, seed) - frost_seed_bump(200.0, 230.0 + 20.0, seed)).abs() < 1e-6,
            "the halo is symmetric above/below the ink"
        );
    }

    /// THE ORGANIC MERGE: two seeds spaced within reach SUM above [`FROST_ISO`] in
    /// the GAP between them (one island), while the same seeds pushed far apart
    /// leave the gap below iso (two islands) — the metaball neck, with NO union/max
    /// and NO per-row rule. The transition is purely the continuous summed field.
    #[test]
    fn frost_coverage_merges_nearby_seeds_and_splits_far_ones() {
        let r = 40.0f32;
        // Two point-seeds (x0==x1) on the same row, 50px apart (< 2r) → the
        // midpoint's summed field clears iso: ONE island.
        let near = [[100.0f32, 100.0, 200.0, r], [150.0, 150.0, 200.0, r]];
        assert!(frost_coverage(125.0, 200.0, &near) > 0.5, "nearby seeds bridge into one island");
        // The same seeds 140px apart (> 2r + skirt) → the midpoint falls below iso:
        // TWO islands, yet each seed's own core still frosts.
        let far = [[100.0f32, 100.0, 200.0, r], [240.0, 240.0, 200.0, r]];
        assert!(frost_coverage(170.0, 200.0, &far) < 0.5, "far seeds leave a live gap between islands");
        assert!(frost_coverage(100.0, 200.0, &far) > 0.5, "each far seed still frosts its own core");
        // MERGE IN ANY DIRECTION: two seeds stacked vertically within reach also
        // bridge (no per-row separation).
        let stacked = [[100.0f32, 200.0, 200.0, r], [100.0, 200.0, 250.0, r]];
        assert!(frost_coverage(150.0, 225.0, &stacked) > 0.5, "vertically-close rows merge organically");
    }

    /// FROST GATING: [`frost_coverage`] frosts a pixel over a seed's ink, stays
    /// live far from every seed, and an EMPTY seed list is a total no-op (0
    /// everywhere) — the inert path a non-frost frame uploads.
    #[test]
    fn frost_coverage_frosts_the_ink_and_empty_is_inert() {
        let a = [10.0f32, 40.0, 20.0, 40.0];
        let b = [200.0f32, 240.0, 220.0, 40.0];
        assert!(frost_coverage(25.0, 20.0, &[a, b]) > 0.999, "over seed A's ink → frosted");
        assert!(frost_coverage(220.0, 220.0, &[a, b]) > 0.999, "over seed B's ink → frosted");
        assert_eq!(frost_coverage(1000.0, 1000.0, &[a, b]), 0.0, "far from every seed the lamp is live");
        // No seeds → inert everywhere (the non-frost frame).
        assert_eq!(frost_coverage(25.0, 20.0, &[]), 0.0, "an empty seed list frosts nothing");
    }

    /// THE FROST PIXEL: below the field threshold it is EXACTLY the flat ground
    /// (the pill is pure ground where no blob reaches); with a blob present it
    /// dims toward the ground and never brightens past the phase-free worst bound
    /// `mix(blob_hi, ground, dim)` — the value the contrast law leans on.
    #[test]
    fn frost_pixel_dims_toward_ground_and_stays_bounded() {
        let ground = Srgb { r: 0x17, g: 0x09, b: 0x0c, a: 0xff };
        let lo = Srgb { r: 0x24, g: 0x0c, b: 0x14, a: 0xff };
        let hi = Srgb { r: 0x52, g: 0x18, b: 0x2c, a: 0xff };
        // Field below the edge band → pure ground.
        let dark = frost_pixel(0.0, ground, lo, hi, FROST_DIM);
        assert_eq!((dark.r, dark.g, dark.b), (ground.r, ground.g, ground.b), "no blob → flat ground");
        // A saturated field → the brightest the pill reaches, dimmed toward ground.
        let bright = frost_pixel(1.0, ground, lo, hi, FROST_DIM);
        let lerp = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t).round() as i32;
        // The worst bound: mix(blob_hi, ground, dim), per channel.
        let bound = (
            lerp(hi.r, ground.r, FROST_DIM),
            lerp(hi.g, ground.g, FROST_DIM),
            lerp(hi.b, ground.b, FROST_DIM),
        );
        assert_eq!((bright.r as i32, bright.g as i32, bright.b as i32), bound, "saturated frost == the worst bound");
        // And the worst bound is genuinely dimmer than the raw blob_hi (the dim works).
        assert!((bright.r as i32) < hi.r as i32, "the value dim pulls the pill back toward ground");
    }

    // The theme-preview CROSSING classification (and its no-wildcard roster law)
    // was RETIRED 2026-07-18 — the bracket now arms unconditionally on every
    // preview step, so there is no per-pair decision left to pin. The behaviour it
    // used to gate is now verified at the App state machine: see
    // `crate::app::tests::every_preview_step_brackets_and_teardown_waits_for_the_reshape_present`.
}
