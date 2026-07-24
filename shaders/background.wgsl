// Background / MARGIN gradient shader (PAGE MODE, N++ figure/ground).
//
// Draws ONE fullscreen triangle whose fragment splits the canvas into the calm
// PAGE column and the styled MARGINS:
//   * inside the column rect [col_left, col_left+col_w) -> alpha 0, so the flat
//     base_100 clear shows through perfectly (the page reads as a clean shape).
//   * outside (the margins) -> a per-world gradient (mix(from,to,t) along `dir`),
//     alpha 1, painting the ground the page floats on.
//
// Static (no time uniform) so the headless capture is byte-deterministic. The
// gradient colors arrive in LINEAR space (the render target is sRGB; the host
// converts the per-world theme bytes before upload), like the selection shader.
//
// When page mode is OFF the host passes col_w == viewport width, so the column
// covers everything and the margins vanish — identical to the old flat clear.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    // Page column left edge + width, in physical pixels.
    col_left: f32,
    col_w: f32,
    // Gradient endpoints (LINEAR rgb; a is the margin opacity, normally 1).
    // NOTE: named `c_from`/`c_to` — `from` is a reserved keyword in WGSL.
    c_from: vec4<f32>,
    c_to: vec4<f32>,
    // Unit gradient direction in UV space (e.g. (0,1)=vertical, (.7,.7)=diagonal).
    // For Stripes this is (cos angle, sin angle), so the gradient runs ALONG the
    // stripe angle.
    dir: vec2<f32>,
    // Procedural margin ground: 0=plain gradient, 1=dots, 2=starfield,
    // 3=pinstripe, 4=stripes, 5=bands, 6=waves. Matches `Background::shader_id`
    // in src/theme/model.rs.
    shader: u32,
    pad: u32,
    // Mark/band tint (LINEAR rgb; a is the max coverage of the marks/band). For
    // shader 5/6 (Bands/Waves) this is the MIDDLE of three authored tones —
    // `c_from`/`c_pat`/`c_to` read as tones 0/1/2 (see `bands_rgb`/`waves_rgb`).
    c_pat: vec4<f32>,
    // Per-ground params: params.x = Dots proximity flag (0/1), params.y = the
    // Stripes/Bands angle (radians), .zw reserved. Both are 0 for every
    // unchanged ground, so those grounds take their exact original code path.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Pixel position of the fragment (top-left origin).
    @location(0) px: vec2<f32>,
};

// A single oversized triangle covering the whole clip space.
var<private> VERTS: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0), vec2<f32>( 3.0, -1.0), vec2<f32>(-1.0,  3.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let ndc = VERTS[vid];
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    // Map clip [-1,1] (y-up) back to pixels (y-down, top-left origin).
    out.px = vec2<f32>(
        (ndc.x * 0.5 + 0.5) * g.viewport.x,
        (0.5 - ndc.y * 0.5) * g.viewport.y,
    );
    return out;
}

// A scalar hash in [0,1) from a 2D integer-ish cell id. Deterministic (no clock),
// so the starfield is byte-stable across captures.
fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

// Proximity to the PAGE-COLUMN boundary as an intensity in [0,1]: 1.0 right at
// the page edge, decaying outward into the margin (exp falloff). Drives the
// Stripes band + the proximity-scaled Dots — the "play area radiates into the
// ground" feel. Pure pixel math, no time.
const EDGE_FALLOFF: f32 = 90.0;
fn edge_intensity(px: vec2<f32>) -> f32 {
    var d = 0.0;
    if (px.x < g.col_left) {
        d = g.col_left - px.x;
    } else {
        d = px.x - (g.col_left + g.col_w);
    }
    return exp(-max(d, 0.0) / EDGE_FALLOFF);
}

// Linear proximity to the PAGE-COLUMN boundary, normalized across the FULL
// margin width: 1.0 right at the page edge, 0.0 out at the viewport edge. Unlike
// `edge_intensity` (a fast exp band), this ramps over the WHOLE margin, so a dot
// RADIUS keyed off it reads its size gradient across the entire ground instead
// of collapsing into one full-size band at the edge. Pure pixel math, no time.
fn edge_proximity(px: vec2<f32>) -> f32 {
    var d = 0.0;
    var span = 1.0;
    if (px.x < g.col_left) {
        d = g.col_left - px.x;
        span = max(g.col_left, 1.0);
    } else {
        d = px.x - (g.col_left + g.col_w);
        span = max(g.viewport.x - (g.col_left + g.col_w), 1.0);
    }
    return clamp(1.0 - d / span, 0.0, 1.0);
}

// Coverage [0,1] of the assigned margin ground at pixel `px`. All grounds are
// pure functions of pixel coordinates — STATIC, no time. Tuned to whisper.
fn pattern_coverage(px: vec2<f32>) -> f32 {
    // --- 1: DOTS — a grid of round dots; `params.x` flips proximity scaling. ---
    if (g.shader == 1u) {
        let cell = 24.0;
        let c = fract(px / cell) - vec2<f32>(0.5, 0.5);
        let d = length(c * cell);
        if (g.params.x > 0.5) {
            // edge=true: the dot RADIUS scales with page proximity — a FULL, fat
            // dot hugging the page boundary, SHRINKING to ~28% out at the far
            // margin (the N++ reference look). SIZE carries the gradient (keyed
            // off the linear, full-margin `edge_proximity`, NOT the fast exp
            // band), so the alpha only floors GENTLY — far dots stay visible-small
            // instead of dissolving before their size can read.
            let p = edge_proximity(px);
            let radius = mix(0.85, 3.0, p); // ~28% far -> a full fat dot at the edge
            let dot = 1.0 - smoothstep(radius, radius + 0.9, d);
            let alpha = mix(0.5, 1.0, p);   // gentle falloff; brightest hugging the page
            return dot * alpha;
        }
        // edge=false: today's UNIFORM ~1.4px dots with a 1px feather (unchanged).
        return 1.0 - smoothstep(1.4, 2.4, d);
    }
    // --- 4: STRIPES — diagonal stripes in a bright band hugging the page edge,
    // dissolving outward into the gradient (the N++ look). The band peaks at the
    // boundary (edge_intensity) and the stripes run perpendicular to `dir`. ---
    if (g.shader == 4u) {
        let a = g.params.y;
        // Coordinate across the stripes (perpendicular bands give the diagonal look).
        let coord = px.x * cos(a) + px.y * sin(a);
        let period = 13.0;
        let f = abs(fract(coord / period) - 0.5) * period; // px distance to a stripe
        let line = 1.0 - smoothstep(2.0, 3.5, f);          // ~bright diagonal stripe
        return line * edge_intensity(px);                  // dissolve outward
    }
    // --- 3: PINSTRIPE — fine vertical parallel lines (ledger / print rules). ---
    if (g.shader == 3u) {
        let period = 9.0;
        let x = abs(fract(px.x / period) - 0.5) * period; // px distance to line
        return 1.0 - smoothstep(0.5, 1.2, x);
    }
    // --- 2: STARFIELD — scattered dots + the occasional 4-point sparkle. ---
    if (g.shader == 2u) {
        let cell = 34.0;
        let id = floor(px / cell);
        let local = fract(px / cell);
        // Per-cell jittered star position + a presence roll (only some cells lit).
        let jx = hash21(id + vec2<f32>(1.0, 0.0));
        let jy = hash21(id + vec2<f32>(0.0, 7.0));
        let present = hash21(id + vec2<f32>(3.0, 5.0));
        let star = vec2<f32>(jx, jy);
        let dpx = (local - star) * cell;
        let r = length(dpx);
        // A small round dot for every lit cell.
        var cov = (1.0 - smoothstep(0.7, 1.7, r)) * step(0.55, present);
        // The brightest ~1/6 cells also get a thin 4-point sparkle cross.
        if (present > 0.84) {
            let cross = (1.0 - smoothstep(0.4, 1.0, abs(dpx.x))) * (1.0 - smoothstep(2.5, 4.5, abs(dpx.y)))
                      + (1.0 - smoothstep(0.4, 1.0, abs(dpx.y))) * (1.0 - smoothstep(2.5, 4.5, abs(dpx.x)));
            cov = max(cov, clamp(cross, 0.0, 1.0));
        }
        return cov;
    }
    // 0: plain gradient — no marks.
    return 0.0;
}

// --- 5: BANDS — EXACTLY THREE broad, tone-on-tone diagonal bands spanning the
// WHOLE margin field (cut-paper grass, not a repeating stripe-tile). Unlike
// `pattern_coverage`'s whisper-marks-over-a-gradient grounds, this (and
// `waves_rgb` below) computes the FINAL rgb directly — the three tones ARE the
// field, not a low-coverage overlay — so `fs_main` branches to it before the
// base-gradient/dither/pattern-overlay pipeline runs (see the early-return
// there). Pure function of pixel position: static, no time, no assets. ---
//
// `coord` projects `px` onto the band direction (`params.y` = angle, the same
// slot Stripes uses); `extent` is that SAME projection of the full viewport
// rect, so `t = coord / extent` lands in [0,1] over the WHOLE canvas — the two
// boundaries at 1/3 and 2/3 are therefore FRACTIONS of the viewport, not a
// fixed pixel period, so a narrower/wider page CROPS OR SCALES the identical
// three-band field instead of tiling more stripes into it. A small smoothstep
// (`aa`, ~1.5px in `t`-space) feathers each boundary — "crisp-but-quiet": a
// tight edge, but between three low-mutual-contrast ladder rungs.
// Shared by `bands_rgb`/`waves_rgb`: three world tones (c_from/c_pat/c_to),
// split at two boundaries along `coord` with a shared antialias half-width
// `aa` — the ONE owner of the "two-boundary tri-tone mix" both fields do,
// only their boundary/coord math differs.
fn tri_tone_mix(coord: f32, b1: f32, b2: f32, aa: f32) -> vec3<f32> {
    let m1 = smoothstep(b1 - aa, b1 + aa, coord);
    let m2 = smoothstep(b2 - aa, b2 + aa, coord);
    let tone01 = mix(g.c_from.rgb, g.c_pat.rgb, m1);
    return mix(tone01, g.c_to.rgb, m2);
}

fn bands_rgb(px: vec2<f32>) -> vec3<f32> {
    let a = g.params.y;
    let dir = vec2<f32>(cos(a), sin(a));
    let extent = max(dot(g.viewport, dir), 1.0);
    let t = clamp(dot(px, dir) / extent, 0.0, 1.0);
    let aa = 1.5 / extent;
    return tri_tone_mix(t, 1.0 / 3.0, 2.0 / 3.0, aa);
}

// --- 6: WAVES — THREE stacked, NON-OVERLAPPING shallow wave tiers (wide
// scalloped crests, horizontally phase-offset tier-to-tier so they read as
// layered swells, never a grid). Tier geometry (amplitude/wavelength/phase) is
// a FIXED constant, never per-world data — every `Waves` world shares this
// exact shape (only the three tones differ). Static: pure function of `px`. ---
//
// The viewport height splits into thirds; each of the two boundaries between
// tiers is that third's y plus a sine wobble in x (a "scallop"), with tier 2's
// boundary carrying a DIFFERENT phase than tier 1's so the two crest-lines
// visibly drift apart ("layer") instead of tracking each other like a grid.
// The wobble amplitude is held well under a third of the viewport height for
// any real window, so the two boundaries never cross — the three tiers stay
// NON-OVERLAPPING by construction.
const WAVE_AMP: f32 = 22.0;
const WAVE_FREQ: f32 = 0.024166097; // 2*pi / 260px — wide, shallow scallops
const WAVE_PHASE_1: f32 = 0.0;
const WAVE_PHASE_2: f32 = 2.4;
fn waves_rgb(px: vec2<f32>) -> vec3<f32> {
    let b1 = g.viewport.y * (1.0 / 3.0) + WAVE_AMP * sin(px.x * WAVE_FREQ + WAVE_PHASE_1);
    let b2 = g.viewport.y * (2.0 / 3.0) + WAVE_AMP * sin(px.x * WAVE_FREQ + WAVE_PHASE_2);
    return tri_tone_mix(px.y, b1, b2, 1.5);
}

// BANDING KILL — the classic 8x8 ordered (Bayer) dither matrix, values 0..64.
// A pure function of PIXEL POSITION alone (no time, no random), so the headless
// capture stays deterministic. Rust mirror + full derivation notes:
// `src/render/dither.rs` (kept in sync by hand — see that file's module doc for
// why a small cross-language duplication is the accepted answer here).
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

// The Bayer threshold at pixel `px`, normalized to [0,1) — tiles every 8px.
fn bayer_threshold01(px: vec2<f32>) -> f32 {
    let x = u32(floor(px.x)) % 8u;
    let y = u32(floor(px.y)) % 8u;
    return f32(BAYER8[y * 8u + x]) / 64.0;
}

// sRGB transfer function (encode: linear -> sRGB, decode: sRGB -> linear),
// applied per-channel. NEEDED for the dither below: the render target is
// `Rgba8UnormSrgb`, so the GPU auto-encodes this shader's LINEAR output to
// sRGB and quantizes THAT to 8 bits on write — a dither meant to land "at
// ±half an 8-bit step before quantization" must therefore perturb the
// SRGB-ENCODED value (the space that's actually rounded to a byte), not the
// linear one: the sRGB curve is steep near black, so a fixed linear-space
// nudge would land as a much LARGER swing in the shadows than in the
// highlights (confirmed empirically: it broke the round's own ≤1-LSB law —
// see `render::tests::dither`). Encoding here, dithering, then decoding back
// to linear before `return` makes the GPU's own re-encode land exactly where
// intended, channel by channel.
fn srgb_encode1(c: f32) -> f32 {
    if (c <= 0.0031308) {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}
fn srgb_decode1(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Inside the page column: punch a hole so the flat base_100 clear shows.
    if (in.px.x >= g.col_left && in.px.x < g.col_left + g.col_w) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // 5/6: BANDS / WAVES compute their own final rgb directly (three opaque
    // authored tones ARE the field) — bypass the gradient/dither/pattern-
    // overlay pipeline below entirely, which every OTHER ground still takes
    // unchanged (byte-identical).
    if (g.shader == 5u) {
        return vec4<f32>(bands_rgb(in.px), 1.0);
    }
    if (g.shader == 6u) {
        return vec4<f32>(waves_rgb(in.px), 1.0);
    }
    // Margin: evaluate the gradient along `dir`. UV is centered so the diagonal
    // worlds read symmetrically; t is clamped to [0,1].
    let uv = in.px / g.viewport;
    let t = clamp(dot(uv - vec2<f32>(0.5, 0.5), g.dir) + 0.5, 0.0, 1.0);
    var rgb = mix(g.c_from.rgb, g.c_to.rgb, t);
    let a = mix(g.c_from.a, g.c_to.a, t);
    // BANDING KILL: an ordered ±half-8-bit-step dither, added in sRGB-ENCODED
    // space (see `srgb_encode1`'s doc for why) BEFORE the GPU quantizes it to
    // the 8-bit render target — imperceptible as its own texture, breaks up
    // the visible banding a smooth `mix()` produces across a wide gradient. A
    // FLAT gradient (from == to, e.g. Wagtail's one-bit background) is an
    // EXACT no-op — any nonzero nudge on a pure #000000/#FFFFFF would round
    // to a forbidden third value under the one-bit law, so this is gated,
    // not merely small.
    let flat = all(g.c_from.rgb == g.c_to.rgb) && (g.c_from.a == g.c_to.a);
    if (!flat) {
        let offset = (bayer_threshold01(in.px) - 0.5) / 255.0;
        let srgb = vec3<f32>(srgb_encode1(rgb.x), srgb_encode1(rgb.y), srgb_encode1(rgb.z));
        let dithered = clamp(srgb + vec3<f32>(offset, offset, offset), vec3<f32>(0.0), vec3<f32>(1.0));
        rgb = vec3<f32>(srgb_decode1(dithered.x), srgb_decode1(dithered.y), srgb_decode1(dithered.z));
    }
    // Overlay the procedural pattern: mix the dim tint in at a low coverage so the
    // marks whisper and the page column stays the clear figure.
    let cov = pattern_coverage(in.px) * g.c_pat.a;
    rgb = mix(rgb, g.c_pat.rgb, cov);
    return vec4<f32>(rgb, a);
}
