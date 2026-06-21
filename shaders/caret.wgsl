// Caret quad shader: draws the animated text caret as a single GPU quad (not a
// glyph). The caret morphs between two amber shapes — a RESTING "roundish square"
// sitting on the current glyph (advance-wide, tall, soft/large corner radius) and
// a MOVING "trailing underline" lying on the baseline (long, thin, small corner
// radius). The morph (size + corner) is computed on the CPU and handed in per
// instance; this shader just rasterizes a clean anti-aliased rounded rectangle.
// NO glow, NO halo.
//
// Coordinates are in PIXELS. `viewport` carries the framebuffer size so we can
// map pixel space to clip space ([-1,1], y-up) in the vertex stage.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    // Center of the caret rect, in pixels.
    @location(0) center: vec2<f32>,
    // Half-size of the rect, in pixels (width/2, height/2). Carries the morph.
    // (Named `half_size`, NOT `half`, because `half` is a reserved Metal type.)
    @location(1) half_size: vec2<f32>,
    // Per-instance rounded-rect corner radius (px): large at rest, small in motion.
    @location(2) corner: f32,
    // Overall alpha multiplier.
    @location(3) alpha: f32,
    // Linear amber color.
    @location(4) color: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Position relative to the caret center, in pixels (for the SDF).
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) corner: f32,
    @location(3) alpha: f32,
    @location(4) color: vec3<f32>,
};

// Unit quad corners (two triangles) in [-1,1].
var<private> CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: Instance) -> VsOut {
    let corner = CORNERS[vid];
    // Expand the quad by 1px beyond the rect so the anti-aliased edge has room.
    let extent = inst.half_size + vec2<f32>(1.0, 1.0);
    let local = corner * extent;
    let px = inst.center + local;

    // Pixel -> clip space. y flips (pixels are y-down, clip is y-up).
    let ndc = vec2<f32>(
        px.x / g.viewport.x * 2.0 - 1.0,
        1.0 - px.y / g.viewport.y * 2.0,
    );

    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.local = local;
    out.half_size = inst.half_size;
    out.corner = inst.corner;
    out.alpha = inst.alpha;
    out.color = inst.color;
    return out;
}

// Signed distance to a rounded rectangle centered at origin with half-size `b`
// and corner radius `r`. Negative inside, positive outside.
fn sd_round_rect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Clamp the corner radius to the smaller half-extent so a thin streak / small
    // shape stays sane (the streak's short edge becomes a rounded cap, never a
    // bulge bigger than the bar).
    let r = min(in.corner, min(in.half_size.x, in.half_size.y));
    let d = sd_round_rect(in.local, in.half_size, r);

    // Solid amber inside the rect with a ~1px anti-aliased edge. No glow.
    let cov = 1.0 - smoothstep(-0.75, 0.75, d);
    let a = clamp(cov, 0.0, 1.0) * in.alpha;
    return vec4<f32>(in.color, a);
}
