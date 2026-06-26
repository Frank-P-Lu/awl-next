// Glyph-silhouette ("Morph") caret shader. Instead of a solid rounded block, the
// caret takes the cursor GLYPH'S shape: a small per-glyph coverage MASK (R8 alpha,
// rasterized on the CPU from the same swash cache glyphon uses) is sampled and the
// accent is painted through it. Two masks are bound — the glyph the caret is
// LEAVING (`mask_from`) and the glyph it is ARRIVING at (`mask_to`) — and the
// fragment cross-fades between them by `morph_t` so the SHAPE morphs as the caret
// glides. There is NO glow/halo/dilation: the silhouette is the glyph's own crisp,
// anti-aliased coverage filled SOLID in the accent. The caret quad is drawn OVER
// the document text (after the glyph pass), so the accent silhouette lands exactly
// on the real letter and recolours it — the cursor's letter reads as the accent.
//
// Coordinates are in PIXELS. Each mask maps onto its own pixel rect (placement
// box) at the caret; the two rects are handed in per-instance so a "from"/"to"
// glyph of different size each map correctly while the body slides via the spring.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var mask_from: texture_2d<f32>;
@group(0) @binding(2) var mask_to: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct Instance {
    // Union pixel rect (min corner + size) the quad covers — the bounding box of
    // BOTH glyph placement boxes plus a small anti-alias margin. The quad is drawn
    // over this.
    @location(0) rect_min: vec2<f32>,
    @location(1) rect_size: vec2<f32>,
    // FROM-glyph placement box (min + size) in pixels (the leaving glyph).
    @location(2) from_min: vec2<f32>,
    @location(3) from_size: vec2<f32>,
    // TO-glyph placement box (min + size) in pixels (the arriving glyph).
    @location(4) to_min: vec2<f32>,
    @location(5) to_size: vec2<f32>,
    // morph_t (0 = show from, 1 = show to), overall alpha.
    @location(6) morph_t: f32,
    @location(7) alpha: f32,
    // Linear accent color.
    @location(8) color: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Absolute pixel position of this fragment (for per-mask uv mapping).
    @location(0) px: vec2<f32>,
    @location(1) from_min: vec2<f32>,
    @location(2) from_size: vec2<f32>,
    @location(3) to_min: vec2<f32>,
    @location(4) to_size: vec2<f32>,
    @location(5) morph_t: f32,
    @location(6) alpha: f32,
    @location(7) color: vec3<f32>,
};

// Unit quad corners (two triangles) in [0,1].
var<private> CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: Instance) -> VsOut {
    let corner = CORNERS[vid];
    let px = inst.rect_min + corner * inst.rect_size;

    // Pixel -> clip space. y flips (pixels are y-down, clip is y-up).
    let ndc = vec2<f32>(
        px.x / g.viewport.x * 2.0 - 1.0,
        1.0 - px.y / g.viewport.y * 2.0,
    );

    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.px = px;
    out.from_min = inst.from_min;
    out.from_size = inst.from_size;
    out.to_min = inst.to_min;
    out.to_size = inst.to_size;
    out.morph_t = inst.morph_t;
    out.alpha = inst.alpha;
    out.color = inst.color;
    return out;
}

// Sample a mask's coverage at absolute pixel `p`, mapping `p` into the mask's
// placement box [min, min+size]. Outside the box coverage is 0. `size.x<=0`
// means the mask is empty (no glyph) → 0.
fn cov_at(tex: texture_2d<f32>, p: vec2<f32>, box_min: vec2<f32>, box_size: vec2<f32>) -> f32 {
    if (box_size.x <= 0.0 || box_size.y <= 0.0) {
        return 0.0;
    }
    let uv = (p - box_min) / box_size;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 0.0;
    }
    return textureSampleLevel(tex, samp, uv, 0.0).r;
}

// Morphed coverage at pixel `p`: cross-fade the two glyph masks by morph_t.
fn morph_cov(in: VsOut, p: vec2<f32>) -> f32 {
    let cf = cov_at(mask_from, p, in.from_min, in.from_size);
    let ct = cov_at(mask_to, p, in.to_min, in.to_size);
    return mix(cf, ct, in.morph_t);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // The glyph's own anti-aliased coverage, cross-faded between the leaving and
    // arriving glyph. No dilation, no halo, no glow — just the crisp silhouette
    // filled SOLID in the accent. The caret draws OVER the text, so this recolours
    // the real letter the accent hue.
    let cov = morph_cov(in, in.px);
    let a = clamp(cov, 0.0, 1.0) * in.alpha;
    return vec4<f32>(in.color, a);
}
