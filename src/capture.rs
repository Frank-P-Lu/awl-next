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
pub const SCHEMA_PLAIN: &str = "awl-capture/89";
pub const SCHEMA_TIMELINE: &str = "awl-capture/90";
pub const SCHEMA_HELD: &str = "awl-capture/91";

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
