// Frosted-backdrop blur shader (the full-overlay BACKDROP).
//
// "Do the effect, do it cheap" (PHILOSOPHY): when a full-takeover overlay opens we
// capture the document framebuffer ONCE, downsample it to quarter resolution, run a
// few separable-Gaussian ping-pong passes, and composite the frosted result behind
// the overlay card. Blur PRESERVES hue — it is a defocus, not a desaturation — so
// the theme's own colours stay intact (the whole point); a small `dim` toward the
// theme's own `base_100` lets the doc recede a value without going neutral grey.
//
// Three fragment entry points share one fullscreen-triangle vertex + one bind group
// (uniform / sampled texture / linear sampler):
//   * fs_down — 4-tap box downsample (full-res doc -> quarter-res).
//   * fs_blur — 9-tap separable Gaussian along `u.step.xy` (the H then V passes).
//   * fs_comp — upsample + dim toward `u.tint` (base_100), the backdrop composite.
//
// Determinism: no time uniform — a pure pixel function of the captured doc, so an
// overlay capture is byte-stable. Colours flow in LINEAR space (the targets are
// sRGB; the GPU decodes on sample and re-encodes on store), so the blur math is
// correct and hue-preserving.

struct U {
    // Sample STEP in UV space: for fs_down the source texel (1/W, 1/H); for fs_blur
    // the quarter texel times the pass direction (e.g. (1/qw, 0) then (0, 1/qh)).
    step: vec4<f32>,
    // Composite tint: rgb = the theme's base_100 (LINEAR), a = the dim amount in
    // [0,1] (0 = pure blur, no recede). Unused by fs_down / fs_blur.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    // A single oversized triangle covering the viewport (no vertex buffer).
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vi];
    var out: VOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Map clip space to texture UV (origin top-left): x in [0,1], y flipped so the
    // framebuffer top (clip y = +1) samples uv.y = 0.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

// 4-tap bilinear box downsample: average four half-texel-offset taps of the SOURCE,
// so the quarter-res target is a smooth average (not a single aliased point sample).
@fragment
fn fs_down(in: VOut) -> @location(0) vec4<f32> {
    let t = u.step.xy;
    var c = textureSample(tex, samp, in.uv + vec2<f32>(-1.0, -1.0) * t);
    c += textureSample(tex, samp, in.uv + vec2<f32>( 1.0, -1.0) * t);
    c += textureSample(tex, samp, in.uv + vec2<f32>(-1.0,  1.0) * t);
    c += textureSample(tex, samp, in.uv + vec2<f32>( 1.0,  1.0) * t);
    return c * 0.25;
}

// 9-tap separable Gaussian along `u.step.xy` (one axis per pass), normalized weights.
@fragment
fn fs_blur(in: VOut) -> @location(0) vec4<f32> {
    let o = u.step.xy;
    var c = textureSample(tex, samp, in.uv) * 0.2270270270;
    c += textureSample(tex, samp, in.uv + o * 1.0) * 0.1945945946;
    c += textureSample(tex, samp, in.uv - o * 1.0) * 0.1945945946;
    c += textureSample(tex, samp, in.uv + o * 2.0) * 0.1216216216;
    c += textureSample(tex, samp, in.uv - o * 2.0) * 0.1216216216;
    c += textureSample(tex, samp, in.uv + o * 3.0) * 0.0540540541;
    c += textureSample(tex, samp, in.uv - o * 3.0) * 0.0540540541;
    c += textureSample(tex, samp, in.uv + o * 4.0) * 0.0162162162;
    c += textureSample(tex, samp, in.uv - o * 4.0) * 0.0162162162;
    return c;
}

// Upsample the blurred quarter texture (linear filtering smooths it back to full
// res) and dim a touch toward the theme's base_100 so the doc recedes a value.
@fragment
fn fs_comp(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv).rgb;
    let dimmed = mix(c, u.tint.rgb, u.tint.a);
    return vec4<f32>(dimmed, 1.0);
}
