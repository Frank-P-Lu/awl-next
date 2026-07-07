// Inline-image textured-quad shader: draws one decoded image per instance as a
// single textured, soft-cornered GPU quad. Templated on caret_glyph.wgsl (the
// textured-quad pipeline) — the delta is an Rgba8UnormSrgb texture SAMPLED for
// color (rather than an R8 coverage mask painted a flat accent) plus a rounded-
// corner SDF borrowed from selection.wgsl for a gentle card edge.
//
// Coordinates are in PIXELS (top-left origin). `viewport` maps pixel space to
// clip space ([-1,1], y-up) in the vertex stage, identical to selection.wgsl.
//
// One texture is bound per draw (the pipeline rebinds + issues one draw per
// visible image), so the fragment shader samples a single `tex`.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Instance {
    // Top-left corner of the destination rect, in pixels.
    @location(0) dst_min: vec2<f32>,
    // Destination rect size (width, height), in pixels.
    @location(1) dst_size: vec2<f32>,
    // Overall opacity (1.0 = opaque).
    @location(2) alpha: f32,
    // Rounded-corner radius (px); 0 = square.
    @location(3) corner: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // UV into the texture (0..1).
    @location(0) uv: vec2<f32>,
    // Position relative to the rect center, in pixels (for the SDF edge).
    @location(1) local: vec2<f32>,
    @location(2) hsize: vec2<f32>,
    @location(3) alpha: f32,
    @location(4) corner: f32,
};

// Unit quad corners (two triangles) in [0,1] (UV space).
var<private> UV_CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: Instance) -> VsOut {
    let uv = UV_CORNERS[vid];
    let px = inst.dst_min + uv * inst.dst_size;

    let ndc = vec2<f32>(
        px.x / g.viewport.x * 2.0 - 1.0,
        1.0 - px.y / g.viewport.y * 2.0,
    );

    let hsize = inst.dst_size * 0.5;
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.local = px - (inst.dst_min + hsize);
    out.hsize = hsize;
    out.alpha = inst.alpha;
    out.corner = inst.corner;
    return out;
}

// Signed distance to a rounded rectangle centered at origin with half-size `b`
// and corner radius `r`. Negative inside, positive outside. (selection.wgsl.)
fn sd_round_rect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    // Rounded-corner mask: clamp the radius to the smaller half-extent so a thin
    // image stays sane, then a ~1px antialiased edge.
    let r = min(in.corner, min(in.hsize.x, in.hsize.y));
    let d = sd_round_rect(in.local, in.hsize, r);
    let mask = 1.0 - smoothstep(-1.0, 1.0, d);
    let a = clamp(mask, 0.0, 1.0) * in.alpha * c.a;
    return vec4<f32>(c.rgb, a);
}
