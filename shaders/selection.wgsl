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
//
// THREE fragment entry points share this ONE vertex stage + Globals/Instance
// layout — the "one owner" shape the DITHER round asks for:
//   * `fs_main`   — the ORIGINAL soft rounded-rect fill (every non-one-bit
//     consumer, unchanged code path when `g.dither <= 0.0`) PLUS its new
//     DITHER MODE (`g.dither > 0.0`): THE ONE WAGTAIL HIGHLIGHT TEXTURE, an
//     ordered-Bayer stipple shared by `==highlight==` spans and search
//     matches on a one-bit world. Every drawn dither pixel is the pure quad
//     color at FULL alpha or fully transparent — never a fractional alpha —
//     so no blend step can introduce a forbidden intermediate grey.
//   * `fs_invert` — TRUE INVERSE-VIDEO (one-bit worlds only), for BOTH the
//     selection AND the caret: a hard-edged (no AA — see its own doc below)
//     ROUNDED-RECT SILHOUETTE (the same `sd_round_rect` SDF + clamp `fs_main`
//     uses, reading the SAME `g.corner` uniform), drawn with its OWN
//     `wgpu::RenderPipeline` object built with a `OneMinusDst` blend state
//     (blend state is baked in at pipeline construction, so this MUST be a
//     separate pipeline — see `src/selection.rs::SelectionPipeline::
//     new_invert`). A SELECTION instance leaves `g.corner` at its
//     construction default `0.0` (a plain rectangle — selection ranges are
//     rectangles, never rounded); a CARET instance uploads its own animated
//     radius via `SelectionPipeline::set_corner` each frame, so the 1-bit
//     caret keeps the same rounded silhouette every other world's caret has.

struct Globals {
    // Framebuffer size in physical pixels.
    viewport: vec2<f32>,
    // Rounded-rect corner radius (px).
    corner: f32,
    // DITHER MODE: 0.0 = the original soft alpha-blended fill (`fs_main`'s
    // pre-round behavior, byte-identical). > 0.0 = THE ONE WAGTAIL HIGHLIGHT
    // TEXTURE is active, and this value IS the ordered-dither density (e.g.
    // 0.25 — see `render::dither::WAGTAIL_HIGHLIGHT_DITHER_DENSITY`, the
    // single Rust-side owner of the actual number). Unused by `fs_invert`.
    dither: f32,
    // OUTLINE / STROKE MODE (V6 P5 round): 0.0 = the original SOLID fill
    // (`fs_main`, byte-identical to before this field existed — every
    // shipping consumer). > 0.0 = draw only a HOLLOW RING `stroke` px wide
    // just inside the rounded-rect edge (the interior is left transparent),
    // so the quad reads as an OUTLINE — the `BarFill::Outline` bars and the
    // `FacetStyle::Chips` inactive ghost pills. Unused by `fs_invert` and the
    // dither branch (both keep their hard on/off contract).
    stroke: f32,
    // DITHER CELL (CHUNK round): the edge, in PHYSICAL pixels, of ONE Bayer
    // cell — the quantization the dither branch snaps its absolute canvas
    // position to BEFORE the Bayer lookup, so a block of `cell`x`cell`
    // physical pixels shares one on/off decision and the stipple reads as
    // DELIBERATE dithered pixels rather than fine per-pixel noise. `1.0`
    // (every non-one-bit consumer's construction default) is a NO-OP —
    // `floor(px/1.0) == floor(px)` — so every other dither consumer (the
    // placard stipple, the always-on page frame at density 1.0) stays
    // byte-identical. THE ONE WAGTAIL HIGHLIGHT TEXTURE's three consumers
    // raise it to ~2 logical px (`render::spans::wagtail_stipple_cell_px`,
    // Retina-aware). Unused by `fs_invert`. Std140 pads the struct to a
    // 32-byte multiple (`_pad` below) so the uniform stays aligned.
    cell: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    // Center of the rectangle, in pixels.
    @location(0) center: vec2<f32>,
    // Half-size (width/2, height/2), in pixels.
    @location(1) hsize: vec2<f32>,
    // Linear RGBA color (alpha is the highlight translucency). Unused by
    // `fs_invert`, which always writes pure white (the invert-blend trick
    // needs `src == 1.0` exactly — see its own doc below).
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Position relative to the rect center, in pixels (for the SDF edge).
    @location(0) local: vec2<f32>,
    @location(1) hsize: vec2<f32>,
    @location(2) color: vec4<f32>,
    // ABSOLUTE canvas pixel position (center + local) — used by the dither
    // branch so a highlight band spanning several quads reads its Bayer
    // pattern as ONE continuous texture rather than restarting phase at
    // every quad's own local origin.
    @location(3) px: vec2<f32>,
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
    out.px = px;
    return out;
}

// Signed distance to a rounded rectangle centered at origin with half-size `b`
// and corner radius `r`. Negative inside, positive outside.
fn sd_round_rect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - r;
}

// THE ONE WAGTAIL HIGHLIGHT TEXTURE's Bayer matrix — identical values to
// `background.wgsl`'s copy (both mirror `src/render/dither.rs::BAYER8`; see
// that file's module doc for why the small cross-file/cross-language
// duplication is the accepted answer here rather than a shared WGSL include,
// which naga/wgpu has no mechanism for).
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

// The Bayer threshold at absolute canvas position `px`, quantized to `cell`-px
// blocks first (CHUNK round): `floor(px / cell)` lands every physical pixel in
// its `cell`x`cell` block on ONE Bayer coordinate, so the whole block shares a
// rank and the stipple coarsens. `cell = 1.0` is the exact pre-chunk behavior
// (`floor(px / 1.0) == floor(px)`); a `max(cell, 1.0)` guard keeps a stray
// `0.0` uniform (never uploaded — the field defaults to `1.0`) from dividing by
// zero.
fn bayer_threshold01(px: vec2<f32>, cell: f32) -> f32 {
    let c = max(cell, 1.0);
    let x = u32(floor(px.x / c)) % 8u;
    let y = u32(floor(px.y / c)) % 8u;
    return f32(BAYER8[y * 8u + x]) / 64.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Clamp the corner radius to the smaller half-extent so thin/short rects
    // stay sane.
    let r = min(g.corner, min(in.hsize.x, in.hsize.y));
    let d = sd_round_rect(in.local, in.hsize, r);

    if (g.dither > 0.0) {
        // THE ONE WAGTAIL HIGHLIGHT TEXTURE: a HARD-edged (no smoothstep —
        // any fractional coverage at the rect boundary would blend a
        // forbidden intermediate grey once multiplied through), Bayer-
        // thresholded stipple. Every pixel this draws is either the pure
        // instance color at FULL alpha, or nothing at all (fully
        // transparent, so the ground/text beneath shows through
        // unmodified) — never a fractional alpha, satisfying the one-bit
        // pixel law even with a highlight/search-match band on screen.
        if (d > 0.0) {
            return vec4<f32>(0.0, 0.0, 0.0, 0.0);
        }
        if (bayer_threshold01(in.px, g.cell) < g.dither) {
            return vec4<f32>(in.color.rgb, 1.0);
        }
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // The ORIGINAL soft fill: solid inside with a ~1px antialiased edge.
    let fill = 1.0 - smoothstep(-1.0, 1.0, d);
    if (g.stroke > 0.0) {
        // OUTLINE MODE: keep only the RING between the outer edge and a rect
        // shrunk `stroke` px inward. `inner` is the fill coverage of that
        // shrunk rect (SDF offset by +stroke); the ring is the outer fill MINUS
        // the inner fill, each with its own ~1px AA edge — a clean hairline that
        // leaves the interior transparent (the room / text shows through).
        let inner = 1.0 - smoothstep(-1.0, 1.0, d + g.stroke);
        let ring = clamp(fill - inner, 0.0, 1.0) * in.color.a;
        return vec4<f32>(in.color.rgb, ring);
    }
    let a = clamp(fill, 0.0, 1.0) * in.color.a;
    return vec4<f32>(in.color.rgb, a);
}

// TRUE INVERSE-VIDEO (one-bit worlds only): this entry point is used ONLY by
// a `RenderPipeline` built with a `OneMinusDst` color blend factor
// (`src_factor: OneMinusDst, dst_factor: Zero` — see
// `SelectionPipeline::new_invert`), which computes, per channel,
// `result = (1 - dst) * src`. Writing `src = (1,1,1)` here makes that exactly
// `result = 1 - dst` — a true "flip every channel" invert: black text
// becomes white, white ground becomes black, wherever this quad covers.
//
// HARD discard, deliberately (no smoothstep/AA): the `OneMinusDst`/`Zero`
// blend factors don't reference the fragment's alpha at all, so there is no
// way to FADE this quad's edge through the blend equation the way the
// ordinary alpha-blended fill above does — a soft-feathered edge here would
// need a genuinely different (unsupported) blend trick. But a hard EDGE
// doesn't mean a hard RECTANGLE: `fs_invert` still evaluates the identical
// `sd_round_rect` SDF (+ the identical `min(g.corner, min(hsize))` clamp)
// `fs_main` above does — ONE owner for the silhouette shape, never a second
// radius/geometry formula — and simply DISCARDS any fragment outside it
// rather than blending toward it. Every SURVIVING pixel is still an exact
// `1 - dst` inversion (the one-bit pixel law holds by construction, only the
// corners end up aliased rather than antialiased — the accepted 1-bit
// tradeoff). `g.corner` is `0.0` for a SELECTION invert instance (no
// `set_corner` call — selection ranges are rectangles, not rounded-rects, so
// this degenerates to the original hard rectangle) and a CARET invert
// instance's own live-animated radius otherwise (`SelectionPipeline::
// set_corner`, mirroring `caret.wgsl`'s per-instance `corner` field). Text
// glyphs drawn UNDER a surviving (inverted) fragment keep their own
// pre-existing antialiased edges — inverting a ~50%-grey AA pixel still
// yields ~50%-grey, the SAME AA tolerance the one-bit pixel law already
// grants ordinary (non-inverted) text edges.
@fragment
fn fs_invert(in: VsOut) -> @location(0) vec4<f32> {
    let r = min(g.corner, min(in.hsize.x, in.hsize.y));
    let d = sd_round_rect(in.local, in.hsize, r);
    if (d > 0.0) {
        discard;
    }
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
