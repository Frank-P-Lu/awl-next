// LAVA-LAMP GROUND shader — awl's first TIME-VARYING background. A 2D metaball
// field ("lava lamp" register) authored as ONE viewport-space backdrop behind
// the centered page column, drawn as ONE fullscreen triangle right AFTER the
// margin-gradient background pass and BEFORE every foreground layer (washes /
// selection / caret / text) — so it reads as a genuine GROUND the document
// floats on.
//
// See `src/lava.rs` for the host wiring (the `Background::Lava` theme variant,
// the `LavaPipeline`, the slow ~10 fps ambient-tick cadence, and the pure-Rust
// mirror of the field + column-mask math this file must stay in lockstep with),
// and `THEMES.md` / `src/theme/model.rs::Background::Lava` for the negotiated
// laws (0%-idle tick, Reduce-Motion freeze, capture t=0 determinism, figure/
// ground by value).
//
// DETERMINISM: the field is a pure function of pixel position + the `phase`
// uniform (never a clock read in the shader), so a headless capture at the fixed
// t=0 phase is byte-deterministic.
//
// ONE FIELD, MARGINS-ONLY VISIBILITY: the immutable viewport field is masked OUT
// of the writing column entirely via the live column bounds in `g.margin` (fed
// by `TextPipeline::column_left`/`column_width`, the ONE geometry owner — never
// a parallel computation here).
// Inside the column the fragment is fully TRANSPARENT (alpha 0), so the flat
// base_100 page clear shows through — except the `Glow` treatment's faint
// sub-threshold tail bleeding a short way under the edge.

struct Globals {
    viewport: vec2<f32>,
    // Last-settled viewport used ONLY for blob geometry. During live resize the
    // live viewport above still drives pixel coordinates and the margin mask.
    field_viewport: vec2<f32>,
    blob_count: u32,
    dither: u32,
    // THE LEFT-MARGIN RAIL CARVE: 1 when the margin OUTLINE is DRAWN this frame
    // (a HEADED doc — `TextPipeline::lava_rail_carved`), making the whole LEFT
    // margin its rail — another no-lava zone, so the outline's dim entries sit on
    // the flat ground instead of inside the lamp. 0 (outline hidden) reclaims the
    // full margin. The bottom-left gutter no longer sets this — it drives the
    // LOCAL corner carve below. MUST match `lava::rail_dist_outside`.
    rail: u32,
    // THE GUTTER'S LOCAL CORNER CARVE: 1 when the bottom-left gutter is DRAWN this
    // frame, so the bounded `gutter_rect` region is carved out of the field mask
    // while the rest of BOTH margins keep the lamp (an ordinary doc goes both
    // sides). MUST match `lava::gutter_corner_dist_outside` / `lava_mask_2d`.
    gutter: u32,
    // MARGINS-ONLY mask, packed as one vec4 (16-byte aligned per WGSL's
    // uniform-address-space rules): [col_left_px, col_right_px, gap_px,
    // mask_mode] — mask_mode 1.0 = hard (fade before the edge), 2.0 = edge-glow
    // (hard + a faint tail under the edge). See `LavaEdge::mask_mode`.
    margin: vec4<f32>,
    // Animation, packed: x = phase (one unit is one vertical bob; the complete
    // field loops after two because horizontal sway runs at half-frequency), yzw
    // reserved. Frozen at 0.0 in a headless capture and under Reduce Motion.
    anim: vec4<f32>,
    // Linear-space rgba (alpha unused, always 1): the margin floor + the
    // metaball's dim-edge and bright-core tones.
    ground: vec4<f32>,
    blob_lo: vec4<f32>,
    blob_hi: vec4<f32>,
    // BASE blob layout: xy = center in UV [0,1] (0,0 = top-left); z = radius as a
    // fraction of viewport HEIGHT (round regardless of aspect); w = field weight.
    // The shader animates each blob's position from `anim.x` (see `blob_center`).
    blobs: array<vec4<f32>, 8>,
    // The GUTTER's local corner carve rect [left, top, right, bottom] (px), used
    // when `gutter == 1`. All-zero otherwise.
    gutter_rect: vec4<f32>,
    // PROBE-ONLY (env AWL_LAVA_BOTH, gallery-only): the OUTLINE rail band rect
    // [left, top, right, bottom] (px) for the `plate`/`band` auditions. Inert
    // when `probe.x == 0` (every ship frame).
    outline_rect: vec4<f32>,
    // PROBE-ONLY: [mode, column_dim, 0, 0] — mode 0 ship / 1 plate / 2 band /
    // 3 bleed; column_dim = the full-bleed under-column alpha floor. 0 in ship.
    probe: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) px: vec2<f32>,
};

var<private> VERTS: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0), vec2<f32>( 3.0, -1.0), vec2<f32>(-1.0,  3.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let ndc = VERTS[vid];
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.px = vec2<f32>(
        (ndc.x * 0.5 + 0.5) * g.viewport.x,
        (0.5 - ndc.y * 0.5) * g.viewport.y,
    );
    return out;
}

const TAU: f32 = 6.28318530718;

// Gaussian falloff steepness — tuned so a lone blob's APPARENT edge (where the
// field crosses THRESHOLD below) lands close to its nominal radius rather than
// collapsing to a much smaller bright core.
const FIELD_K: f32 = 1.2;
const THRESHOLD: f32 = 0.5;
const EDGE_WIDTH: f32 = 0.12;
const CORE_WIDTH: f32 = 0.35;

// The slow lava BOB: each blob rises/falls on its own sine of `phase`, keyed off
// its index so the lamps never move in unison (a per-blob amplitude / period /
// offset). MUST match `lava::animated_center` in `src/lava.rs` (the pure-Rust
// mirror the field tests run against).
fn blob_center(i: u32, base: vec4<f32>) -> vec2<f32> {
    let fi = f32(i);
    let phase = g.anim.x;
    // Per-blob vertical amplitude (fraction of UV height) + a gentle horizontal
    // sway an octave slower, both offset by the index so the field necks + splits
    // as neighbours drift past each other. The half-frequency horizontal term
    // makes TWO phase cycles the first seamless endpoint (see
    // `lava::LAVA_LOOP_CYCLES`). Horizontal amplitude follows the
    // authored viewport-relative radius, never a margin measurement. MUST match
    // `lava::animated_center`.
    let amp_y = 0.055 + 0.020 * fract(fi * 0.37);
    let aspect = g.field_viewport.y / max(g.field_viewport.x, 1.0);
    let amp_x = base.z * aspect * (0.18 + 0.08 * fract(fi * 0.61));
    let off = fi * 1.7;
    let cy = base.y + amp_y * sin(phase * TAU + off);
    let cx = base.x + amp_x * sin(phase * TAU * 0.5 + off * 1.3);
    return vec2<f32>(cx, cy);
}

fn metaball_field(px: vec2<f32>) -> f32 {
    var total = 0.0;
    for (var i = 0u; i < g.blob_count; i = i + 1u) {
        let b = g.blobs[i];
        let c = blob_center(i, b);
        let center = vec2<f32>(c.x * g.field_viewport.x, c.y * g.field_viewport.y);
        let r_px = max(b.z * g.field_viewport.y, 1.0);
        let d = px - center;
        let dist_sq = dot(d, d);
        total = total + b.w * exp(-FIELD_K * dist_sq / (r_px * r_px));
    }
    return total;
}

// The classic 8x8 ordered (Bayer) dither matrix — the SAME values as
// `shaders/background.wgsl`'s BAYER8 (a small, accepted cross-shader
// duplication; the product's own copy stays the single source of truth).
var<private> BAYER8: array<u32, 64> = array<u32, 64>(
     0u, 32u,  8u, 40u,  2u, 34u, 10u, 42u,
    48u, 16u, 56u, 24u, 50u, 18u, 58u, 26u,
    12u, 44u,  4u, 36u, 14u, 46u,  6u, 38u,
    60u, 28u, 52u, 20u, 62u, 30u, 54u, 22u,
     3u, 35u, 11u, 43u,  1u, 33u,  9u, 41u,
    51u, 19u, 59u, 27u, 49u, 17u, 57u, 25u,
    15u, 47u,  7u, 39u, 13u, 45u,  5u, 37u,
    63u, 31u, 55u, 23u, 61u, 29u, 53u, 21u,
);

fn bayer_threshold01(px: vec2<f32>) -> f32 {
    let x = u32(floor(px.x)) % 8u;
    let y = u32(floor(px.y)) % 8u;
    return f32(BAYER8[y * 8u + x]) / 64.0;
}

// EDGE-GLOW boundary tunables — a genuinely SEPARATE, gentler falloff of the raw
// field (never a peek at the blob's own `edge_t` silhouette), so a faint tint can
// bleed a short way UNDER the column edge even when no blob's visible silhouette
// touches it. See `src/lava.rs`'s notes on why decoupling from `edge_t` is what
// makes the two treatments actually differ.
const GLOW_BLEED_PX: f32 = 56.0;
const GLOW_FIELD_REF: f32 = 0.12;
const GLOW_MAX: f32 = 0.16;

// The signed "distance outside" an axis-aligned rect [left, top, right, bottom]
// (px): <= 0 inside, positive out. Per-axis outside distance combined by max, so
// negative iff BOTH axes are inside. MUST match `lava::gutter_corner_dist_outside`.
fn rect_dist_outside(p: vec2<f32>, rect: vec4<f32>) -> f32 {
    let dx = max(rect.x - p.x, p.x - rect.z);
    let dy = max(rect.y - p.y, p.y - rect.w);
    return max(dx, dy);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let x = in.px.x;
    let mode = g.margin.w;
    let probe = u32(g.probe.x + 0.5);
    // `dist_outside`: positive in a lava-bearing margin, <= 0 inside a no-lava
    // zone. Ordinarily the one zone is the writing column (both edges via
    // max()); with the LEFT-MARGIN RAIL carved (`g.rail == 1u` — the OUTLINE
    // draws there, a headed doc) the whole LEFT margin joins it — only the
    // RIGHT margin's distance counts, so the rail renders the flat ground
    // (and, through `could_glow` below, sheds the left-edge bleed a flat rail
    // would make read as an unexplained tint). The `plate`/`band` probes
    // deliberately DON'T full-carve — they let a headed doc keep both margins.
    // The lava is ALWAYS margins-only (ship), so `mode` is 1.0 (hard) or 2.0
    // (glow) — never 0. `mask` = 0 at the zone edge, ramping to full strength
    // `gap` px further out into the margin, so the field fades entirely OUTSIDE
    // the zones and the page (and a carved rail) stays a clean flat ground.
    // MUST match `lava::rail_dist_outside`/`lava::lava_mask` (the Rust mirror).
    let dist_all = max(g.margin.x - x, x - g.margin.y);
    let rail_on = g.rail == 1u && probe != 1u && probe != 2u;
    let dist_outside = select(dist_all, x - g.margin.y, rail_on);
    let gap = max(g.margin.z, 1.0);
    var mask = smoothstep(0.0, gap, dist_outside);

    // GUTTER LOCAL CORNER CARVE (ship): a bounded bottom-left region around the
    // gutter block is carved from the mask, feathered over `gap` px at its
    // top/right faces, while the rest of both margins keep the lamp. MUST match
    // `lava::lava_mask_2d`.
    if (g.gutter == 1u) {
        mask = mask * smoothstep(0.0, gap, rect_dist_outside(in.px, g.gutter_rect));
    }

    // PROBE `band` (gallery-only): a local band carve around the OUTLINE rail,
    // so a headed doc keeps both margins outside the rail's own band. Its
    // left-edge glow is shed locally too (see `near_rail_edge` below) — the
    // carve and the glow-shed cover the SAME rail band, never the far margin.
    if (probe == 2u) {
        mask = mask * smoothstep(0.0, gap, rect_dist_outside(in.px, g.outline_rect));
    }

    // FULL-BLEED probe: the lamp continues UNDER the writing column at a faint,
    // heavily value-dimmed alpha floor, so the document floats over one field.
    let bleed = probe == 3u;

    // EDGE-GLOW: within the short bleed distance INSIDE the column, a faint,
    // field-driven tail may show (glow mode only). The `plate`/`band` probes
    // shed it ONLY where their local rail treatment sits — the LEFT edge, within
    // the rail band's own vertical extent — so a flat local band (or solid plate)
    // sheds the same left-edge bleed a carved rail does WITHOUT touching the
    // opposite margin's ordinary glow. `x - g.margin.x < GLOW_BLEED_PX` is TRUE
    // only for the LEFT-edge tail (the RIGHT edge sits a full column-width away),
    // so the right margin's edge-glow survives both probes untouched.
    let rail_probe = probe == 1u || probe == 2u;
    let in_rail_band_y = in.px.y >= g.outline_rect.y && in.px.y <= g.outline_rect.w;
    let near_rail_edge = rail_probe && in_rail_band_y && (x - g.margin.x) < GLOW_BLEED_PX;
    let could_glow = mode > 1.5 && !near_rail_edge && dist_outside < 0.0 && dist_outside > -GLOW_BLEED_PX;

    // PROBE `plate` (gallery-only): a SOLID ground plate behind just the rail
    // entries — opaque ground inside the outline band, lava both sides elsewhere.
    // The plate-to-column boundary reads as a clean solid floor: the left-edge
    // glow tail is shed inside the rail band (`near_rail_edge`), so no hair of
    // blob crest bleeds across the plate's edge.
    if (probe == 1u && rect_dist_outside(in.px, g.outline_rect) <= 0.0) {
        return vec4<f32>(g.ground.rgb, 1.0);
    }

    // Deep inside the column with no glow possible: the fragment is fully
    // TRANSPARENT so the flat base_100 page clear shows through untouched — both
    // a legibility guarantee and a free perf win (most of a page's pixels). The
    // full-bleed probe skips this (it fills the column instead).
    if (mask < 0.02 && !could_glow && !bleed) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let field = metaball_field(in.px);
    let edge_t = smoothstep(THRESHOLD - EDGE_WIDTH, THRESHOLD + EDGE_WIDTH, field);
    let core_t = smoothstep(THRESHOLD, THRESHOLD + CORE_WIDTH, field);
    let blob_rgb = mix(g.blob_lo.rgb, g.blob_hi.rgb, core_t);
    let rgb_smooth = mix(g.ground.rgb, blob_rgb, edge_t);

    var margin_rgb: vec3<f32>;
    if (g.dither == 1u) {
        // A pixel untouched by any blob's influence stays the PLAIN flat ground
        // (no dither noise on bare ground — the "whisper" discipline for
        // background marks).
        if (edge_t < 0.02) {
            margin_rgb = g.ground.rgb;
        } else {
            // COARSE ordered dither: posterize the smooth blend into a handful of
            // per-channel levels, offset by a 16px-effective Bayer cell (double
            // the product's 8px banding-kill cell) — the print-grain texture reads
            // across the WHOLE soft blob silhouette, not just a threshold ring.
            let levels = 5.0;
            let coarse = floor(in.px * 0.5);
            let d = bayer_threshold01(coarse) - 0.5;
            let q = floor(rgb_smooth * (levels - 1.0) + d + 0.5) / (levels - 1.0);
            margin_rgb = clamp(q, vec3<f32>(0.0), vec3<f32>(1.0));
        }
    } else {
        margin_rgb = rgb_smooth;
    }

    // MARGIN: the mask fades the whole lava (ground + blobs) toward FULLY
    // TRANSPARENT as it approaches the column edge, so the margin ground never
    // spills a hard seam onto the page. Opaque (alpha `mask`) out in the margin;
    // the straight-alpha blend composites it over the base-ground pass beneath.
    var out_rgb = margin_rgb;
    var out_a = mask;

    // FULL-BLEED probe (gallery-only): under the column the mask is ~0, but the
    // lamp keeps a faint value-dimmed presence there — the field's own color at a
    // low alpha floor, so the document reads over one continuous ground.
    if (bleed) {
        out_a = max(out_a, g.probe.y * clamp(edge_t, 0.0, 1.0));
    }

    if (could_glow) {
        // A SEPARATE, gentler falloff (see the const doc above): a faint blob_lo
        // tint bleeding a short way under the glass, capped at GLOW_MAX.
        let falloff = smoothstep(-GLOW_BLEED_PX, 0.0, dist_outside);
        let strength = clamp(field / GLOW_FIELD_REF, 0.0, 1.0);
        let glow = falloff * strength * GLOW_MAX;
        // Inside the column the base is transparent, so the glow is the blob_lo
        // tint at a low alpha (composites over the base_100 page clear).
        out_rgb = g.blob_lo.rgb;
        out_a = max(out_a, glow);
    }

    return vec4<f32>(out_rgb, out_a);
}
