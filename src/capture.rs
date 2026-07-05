//! Headless one-frame capture: render the shared text pipeline to an offscreen
//! texture, read the pixels back to the CPU, and write a PNG + a JSON sidecar.
//!
//! This is the PRIMARY verification path for the project: same input => byte
//! stable PNG, plus a machine-readable description of render state.
//!
//! The harness is split into focused submodules (the `render.rs` precedent), with
//! this file as the module ROOT holding only the shared constants + the wiring:
//! - [`gpu`]: the headless wgpu device / offscreen target / pixel readback.
//! - [`opts`]: the public INPUT types ([`CaptureOpts`] + its metadata blocks).
//! - [`modes`]: the SINGLE-FRAME capture entry points + shared snapshot helpers.
//! - [`animated`]: the `--capture-timeline` / `--capture-held` per-step drivers.
//! - [`sidecar`]: the hand-rolled JSON sidecar writer.
//! - [`oracle`]: the headless visual-line motion oracle for `--keys` replay.
//!
//! Every public item is re-exported here so the `capture::*` call sites resolve
//! exactly as before.

/// Deterministic canvas size for headless renders.
pub const CANVAS_WIDTH: u32 = 1200;
pub const CANVAS_HEIGHT: u32 = 800;
/// Offscreen format. Srgb so glyphon's default (sRGB) blending matches windowed.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The sidecar SCHEMA strings, one per emitted shape — the SINGLE source of truth
/// for the version number so a bump is one edit and the `write_sidecar` match arms
/// can't drift from each other:
/// - [`SCHEMA_PLAIN`]: the `--screenshot` single frame (caret block absent).
/// - [`SCHEMA_TIMELINE`]: a `--capture-timeline` step (caret block, no `trail`).
/// - [`SCHEMA_HELD`]: a `--capture-held` step (caret block WITH the `trail`).
///
/// `/86` (was `/83`) added `font.cjk` — the Japanese-bundle round's resolved CJK
/// family + whether it's the bundled Noto Serif/Sans JP face (see
/// `render::TextPipeline::cjk_report`), `null` when the buffer has no CJK run.
/// (Landed alongside the WYSIWYG round's `/83` `wysiwyg` block bump, so that
/// merge carried both additions in one further bump.)
///
/// `/89` (was `/86`) adds `buffers` — the multi-buffer core round's `{ open,
/// active }` report (see `crate::buffers::BufferRegistry`, CAPTURE.md).
///
/// `/92` (was `/89`) is the i18n round: a top-level `doc_lang` field (the
/// document's own frontmatter `lang:` tag, `null` when untagged/non-markdown —
/// see `crate::frontmatter::detect`), and `font.scripts` — `font.cjk`'s shape
/// generalized to all four non-Latin scripts (`{ ja, zh_hans, zh_hant, ko }`,
/// each `{family, bundled}|null` — see `render::TextPipeline::script_font_report`).
/// The HUD block also gains a `lang` field (see `hud::Stats`).
///
/// `/95` (was `/92`) FIXES the `gutter` block to always agree with the pixels
/// (the gutter-elision bug: a long filename used to WRAP mid-word in the
/// left-margin box while `gutter.name`/`gutter.project` kept reporting the raw,
/// un-drawn text — see `render::rowlayout::gutter_plan` +
/// `render::TextPipeline::gutter_layout`). Same shape, corrected meaning: BOTH
/// `name` and `project` are EXACTLY as drawn — each independently fit to ONE
/// line, middle-elided (extension preserved) the instant the margin can't hold
/// it whole. A taste pass (still under this same `/95`, before either shape
/// shipped anywhere) settled the two lines' relationship: neither yields to the
/// other from width pressure — `project` is `""` here only when there is
/// genuinely no project to show, never as a forced yield to protect the
/// filename. Unaffected at any margin wide enough to hold both lines whole
/// (every existing wide-window capture).
///
/// `/98` (was `/95`) is the PROSE/CODE PAGE-WIDTH SPLIT: the 70-char measure is
/// a PROSE number; a recognized code file now reads its own `page_width_code`
/// (default 100, rustfmt's `max_width`) instead of sharing the prose measure.
/// The `page` block gains `class` (`"prose"`/`"code"` — `TextPipeline::page_class`,
/// the SAME classifier `Buffer::page_class` uses), so a reviewer can assert which
/// sticky measure is in effect directly from the sidecar. Every OTHER `page`
/// field is unchanged; a document whose class was already implicitly "prose"
/// under the old single `page_width` key renders byte-identically (same default
/// measure, `class: "prose"` newly reported).
pub const SCHEMA_PLAIN: &str = "awl-capture/98";
pub const SCHEMA_TIMELINE: &str = "awl-capture/99";
pub const SCHEMA_HELD: &str = "awl-capture/100";

mod animated;
mod gpu;
mod modes;
mod opts;
mod oracle;
mod sidecar;

pub use animated::{capture_held, capture_timeline, HeldDir};
pub use modes::{
    capture_motion, capture_motion_diagonal, capture_motion_vertical, capture_with,
};
pub use opts::{BuffersInfo, CaptureInfo, CaptureOpts, OverlayInfo, ProjectInfo};
pub use oracle::build_oracle;

// The [`OraclePipeline`] type is part of the module's public surface but is not
// named at a call site today (the oracle is returned only as `Option<_>`), so
// re-exporting it as a bin-crate would otherwise warn unused.
#[allow(unused_imports)]
pub use oracle::OraclePipeline;

#[cfg(test)]
mod tests;
