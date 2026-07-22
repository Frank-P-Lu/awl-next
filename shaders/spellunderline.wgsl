// Spell-check squiggle shader: draws each misspelled word's underline as a
// wavy (cosine) red line inside a band quad. The vertex stage expands a unit quad
// to the band + a small margin so the antialiased stroke is not clipped. The
// fragment stage evaluates the distance from the pixel to the wave curve
//   y = -amp * cos(x * 2*pi / period)
// (taken about the band's vertical center) and shades a soft, ~`thickness`-wide
// antialiased stroke. Drawn UNDER the text so glyphs stay crisp on top.
//
// PHASE (item 38): the wave BEGINS AT ITS TOP under the word's first glyph. `x0`
// is the band's left edge (the first glyph), so at `px.x == x0` the phase is 0 and
// `-cos(0) == -1` puts the curve at `center.y - amp` — the crest (top, since y is
// screen-DOWN). A plain `sin` would start at the vertical center (a zero-crossing)
// and dive DOWN first; the cosine start lands a crest right under the first letter.
//
// Coordinates are in PIXELS (top-left origin). `viewport` maps pixel space to
// clip space ([-1,1], y-up) in the vertex stage, identical to selection.wgsl.
//
// NOTE: the half-size field is named `hsize` (not `half`) because `half` is a
// reserved type keyword in Metal Shading Language and breaks WGSL->MSL codegen.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    // Center of the band, in pixels.
    @location(0) center: vec2<f32>,
    // Half-size (width/2, height/2) of the band, in pixels.
    @location(1) hsize: vec2<f32>,
    // Pixel x of the band's LEFT edge (wave phase anchor).
    @location(2) x0: f32,
    // Sine amplitude (px).
    @location(3) amp: f32,
    // Sine period (px).
    @location(4) period: f32,
    // Stroke thickness (px).
    @location(5) thickness: f32,
    // Linear RGBA color.
    @location(6) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Pixel position of this fragment (absolute, for the wave phase).
    @location(0) px: vec2<f32>,
    // Band center (px) so the wave is taken about the vertical mid-line.
    @location(1) center: vec2<f32>,
    @location(2) hsize: vec2<f32>,
    @location(3) x0: f32,
    @location(4) amp: f32,
    @location(5) period: f32,
    @location(6) thickness: f32,
    @location(7) color: vec4<f32>,
};

// Unit quad corners (two triangles) in [-1,1].
var<private> CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0),
);

const PI: f32 = 3.14159265;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: Instance) -> VsOut {
    let corner = CORNERS[vid];
    // 2px margin so the antialiased stroke + wave crests are not clipped by the
    // quad (the band height already includes the amplitude, but pad for AA).
    let extent = inst.hsize + vec2<f32>(2.0, 2.0);
    let local = corner * extent;
    let px = inst.center + local;

    let ndc = vec2<f32>(
        px.x / g.viewport.x * 2.0 - 1.0,
        1.0 - px.y / g.viewport.y * 2.0,
    );

    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.px = px;
    out.center = inst.center;
    out.hsize = inst.hsize;
    out.x0 = inst.x0;
    out.amp = inst.amp;
    out.period = inst.period;
    out.thickness = inst.thickness;
    out.color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Horizontal extent of the band (clip the wave to the word, with a tiny
    // soft edge at each end so it doesn't hard-cut mid-crest).
    let left = in.center.x - in.hsize.x;
    let right = in.center.x + in.hsize.x;
    let phase = (in.px.x - in.x0) * (2.0 * PI / in.period);
    // Curve height about the band's vertical center. `-cos` so the wave BEGINS at
    // its TOP (crest) under the first glyph (phase 0 → center.y - amp); see the
    // header note (item 38).
    let wave_y = in.center.y - in.amp * cos(phase);

    // Distance from this fragment to the curve. We approximate the true
    // perpendicular distance by dividing the vertical gap by the local slope
    // magnitude sqrt(1 + dy/dx^2), which keeps the stroke an even width even
    // on the steep parts of the wave (a plain vertical |dy| would fatten the
    // flats and thin the slopes). Slope of `-amp*cos(phase)` is `+amp*(…)*sin(phase)`.
    let dydx = in.amp * (2.0 * PI / in.period) * sin(phase);
    let dist = abs(in.px.y - wave_y) / sqrt(1.0 + dydx * dydx);

    // Antialiased stroke of half-width thickness/2 with a ~1px feather.
    let half_w = in.thickness * 0.5;
    var a = 1.0 - smoothstep(half_w - 0.75, half_w + 0.75, dist);

    // Fade the very ends so the squiggle starts/stops softly within the word.
    let edge = 1.5;
    a = a * smoothstep(left - 0.5, left + edge, in.px.x);
    a = a * (1.0 - smoothstep(right - edge, right + 0.5, in.px.x));

    a = clamp(a, 0.0, 1.0) * in.color.a;
    return vec4<f32>(in.color.rgb, a);
}
