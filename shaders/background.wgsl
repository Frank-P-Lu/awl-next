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
    dir: vec2<f32>,
    // Procedural margin pattern: 0=plain gradient, 1=dot-grid, 2=starfield,
    // 3=pinstripe. Matches `BgPattern::shader_id` in src/theme.rs.
    pattern: u32,
    pad: u32,
    // Pattern mark tint (LINEAR rgb; a is the max coverage of the marks).
    c_pat: vec4<f32>,
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

// Coverage [0,1] of the assigned margin pattern at pixel `px`. All patterns are
// pure functions of pixel coordinates — STATIC, no time. Tuned to whisper.
fn pattern_coverage(px: vec2<f32>) -> f32 {
    // --- 1: DOT GRID — a subtle perforated grid of round dots. ---
    if (g.pattern == 1u) {
        let cell = 24.0;
        let c = fract(px / cell) - vec2<f32>(0.5, 0.5);
        let d = length(c * cell);
        // ~1.4px dots with a 1px feather.
        return 1.0 - smoothstep(1.4, 2.4, d);
    }
    // --- 3: PINSTRIPE — fine vertical parallel lines (ledger / print rules). ---
    if (g.pattern == 3u) {
        let period = 9.0;
        let x = abs(fract(px.x / period) - 0.5) * period; // px distance to line
        return 1.0 - smoothstep(0.5, 1.2, x);
    }
    // --- 2: STARFIELD — scattered dots + the occasional 4-point sparkle. ---
    if (g.pattern == 2u) {
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Inside the page column: punch a hole so the flat base_100 clear shows.
    if (in.px.x >= g.col_left && in.px.x < g.col_left + g.col_w) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // Margin: evaluate the gradient along `dir`. UV is centered so the diagonal
    // worlds read symmetrically; t is clamped to [0,1].
    let uv = in.px / g.viewport;
    let t = clamp(dot(uv - vec2<f32>(0.5, 0.5), g.dir) + 0.5, 0.0, 1.0);
    var rgb = mix(g.c_from.rgb, g.c_to.rgb, t);
    let a = mix(g.c_from.a, g.c_to.a, t);
    // Overlay the procedural pattern: mix the dim tint in at a low coverage so the
    // marks whisper and the page column stays the clear figure.
    let cov = pattern_coverage(in.px) * g.c_pat.a;
    rgb = mix(rgb, g.c_pat.rgb, cov);
    return vec4<f32>(rgb, a);
}
