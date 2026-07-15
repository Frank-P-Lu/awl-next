//! OFFLINE WGSL -> GLSL ES 300 (WebGL2) shader validation for the DITHER
//! round's shader changes. `cargo build --target wasm32-unknown-unknown`
//! only proves the RUST side compiles — WGSL itself is parsed/validated by
//! `wgpu`/`naga` at RUNTIME (`create_shader_module`), which needs a real
//! GPU/browser context this sandbox cannot open. This file closes that gap
//! OFFLINE: it runs the exact same `naga` (pinned to the identical
//! `=29.0.3` version `wgpu` uses) parse -> validate -> GLSL-backend pipeline
//! wgpu's own GL/WebGL backend runs internally, without needing a device at
//! all — dev-only (`naga` is a `[dev-dependencies]`-only addition, never a
//! runtime dependency of the shipped binary).
//!
//! Targets `naga::back::glsl::Version::Embedded { version: 300, is_webgl:
//! true }` — GLSL ES 3.00, exactly what WebGL2 (awl's wasm fallback per
//! CLAUDE.md) speaks. A pass here means: the WGSL parses, the module
//! validates against naga's default (WebGPU-shaped) capability set, AND
//! every fragment entry point this round touches or added
//! (`background.wgsl`'s `fs_main`; `selection.wgsl`'s `fs_main` AND the new
//! `fs_invert`) translates to GLSL ES 300 with no backend error — the
//! concrete risks a WebGL2 target could plausibly hit (an unsupported
//! feature, an unresolvable entry point, a version-gated construct) are
//! exactly what this exercises.

fn validate_and_glsl(source: &str, stage: naga::ShaderStage, entry_point: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("WGSL parse failed for entry point {entry_point}: {e}"));

    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("naga validation failed for entry point {entry_point}: {e}"));

    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::Embedded { version: 300, is_webgl: true },
        writer_flags: naga::back::glsl::WriterFlags::empty(),
        binding_map: Default::default(),
        zero_initialize_workgroup_memory: true,
    };
    let pipeline_options = naga::back::glsl::PipelineOptions {
        shader_stage: stage,
        entry_point: entry_point.to_string(),
        multiview: None,
    };
    let mut out = String::new();
    let mut writer = naga::back::glsl::Writer::new(
        &mut out,
        &module,
        &info,
        &options,
        &pipeline_options,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .unwrap_or_else(|e| panic!("GLSL ES 300 (WebGL2) writer construction failed for {entry_point}: {e}"));
    writer
        .write()
        .unwrap_or_else(|e| panic!("GLSL ES 300 (WebGL2) translation failed for {entry_point}: {e}"));
    assert!(
        out.contains("void main"),
        "GLSL output for {entry_point} looks empty/malformed: {out:?}"
    );
}

#[test]
fn background_wgsl_vs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/background.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Vertex, "vs_main");
}

#[test]
fn background_wgsl_fs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/background.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Fragment, "fs_main");
}

#[test]
fn selection_wgsl_vs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/selection.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Vertex, "vs_main");
}

/// The ORDINARY fill / DITHER MODE fragment (both share one entry point,
/// branching at runtime on `Globals::dither`) — the round's "one shader, one
/// owner" highlight/search-match mechanism.
#[test]
fn selection_wgsl_fs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/selection.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Fragment, "fs_main");
}

/// THE NEW true inverse-video fragment entry point — must ALSO survive the
/// WebGL2 translation, including its `discard` statement (core WGSL, and
/// `discard` is standard in GLSL ES fragment shaders too).
#[test]
fn selection_wgsl_fs_invert_targets_webgl2() {
    let src = include_str!("../../../shaders/selection.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Fragment, "fs_invert");
}

// ── lava.wgsl (the animated blob backdrop) ─────────────────────────────────
// Closes a logged gap: lava's WGSL was validated only implicitly at native
// runtime (`create_shader_module` against Metal/Vulkan), never against the
// WebGL2/GLSL-ES-300 downlevel target its browser fallback would take. Its
// fragment `fs_main` is the risk-bearer here — a per-pixel loop summing the
// animated blob field (branches, `for`, transcendentals) is exactly the kind
// of body a GLSL-ES backend can choke on where the simple fills above do not.
#[test]
fn lava_wgsl_vs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/lava.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Vertex, "vs_main");
}

#[test]
fn lava_wgsl_fs_main_targets_webgl2() {
    let src = include_str!("../../../shaders/lava.wgsl");
    validate_and_glsl(src, naga::ShaderStage::Fragment, "fs_main");
}
