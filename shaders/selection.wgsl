// Selection highlight shader: draws each visible line of the active text region
// as a single translucent, soft-cornered GPU quad. Unlike the caret shader
// there is NO glow and NO trail — just a flat rounded rectangle with a ~1px
// antialiased edge, so it reads as a calm highlight band behind the glyphs.
//
// Coordinates are in PIXELS (top-left origin). `viewport` maps pixel space to
// clip space ([-1,1], y-up) in the vertex stage, identical to caret.wgsl.
//
// NOTE: the half-size field is named `hsize` (not `half`) because `half` is a
// reserved type keyword in Metal Shading Language and breaks WGSL->MSL codegen.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    // Rounded-rect corner radius (px).
    corner: f32,
    pad: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    // Center of the rectangle, in pixels.
    @location(0) center: vec2<f32>,
    // Half-size (width/2, height/2), in pixels.
    @location(1) hsize: vec2<f32>,
    // Linear RGBA color (alpha is the highlight translucency).
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Position relative to the rect center, in pixels (for the SDF edge).
    @location(0) local: vec2<f32>,
    @location(1) hsize: vec2<f32>,
    @location(2) color: vec4<f32>,
};

// Unit quad corners (two triangles) in [-1,1].
var<private> CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: Instance) -> VsOut {
    let corner = CORNERS[vid];
    // 1px margin so the antialiased edge is not clipped by the quad.
    let extent = inst.hsize + vec2<f32>(1.0, 1.0);
    let local = corner * extent;
    let px = inst.center + local;

    let ndc = vec2<f32>(
        px.x / g.viewport.x * 2.0 - 1.0,
        1.0 - px.y / g.viewport.y * 2.0,
    );

    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.local = local;
    out.hsize = inst.hsize;
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
    // Clamp the corner radius to the smaller half-extent so thin/short rects
    // stay sane.
    let r = min(g.corner, min(in.hsize.x, in.hsize.y));
    let d = sd_round_rect(in.local, in.hsize, r);
    // Solid inside with a ~1px antialiased edge.
    let fill = 1.0 - smoothstep(-1.0, 1.0, d);
    let a = clamp(fill, 0.0, 1.0) * in.color.a;
    return vec4<f32>(in.color.rgb, a);
}
