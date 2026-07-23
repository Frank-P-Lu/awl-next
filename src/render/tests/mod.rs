//! UNIT TESTS for the `TextPipeline` GPU aggregation root, split by feature
//! area (the 2026-07 code-organization pass) out of one formerly-9.7k-line
//! `render::tests` module into this `render/tests/` directory -- every test's
//! NAME is unchanged, only its module path grew one segment
//! (`render::tests::foo` -> `render::tests::<area>::foo`). `use super::*;`
//! here still resolves to the `render` root exactly as before the split; each
//! child module re-derives render access directly via its own
//! `use super::super::*;` (a single glob, so it can never collide with a
//! sibling test module of the same name as a real render/theme module -- see
//! `theme/`/`geometry.rs`) plus a targeted `use super::{..};` for whichever
//! of this module's own shared test helpers it actually calls.

use super::*;

mod build_integrity;
mod caret;
mod caret_block;
mod chrome_overlay;
mod chrome_panels;
mod cjk;
mod dither;
mod firetail_showcase;
mod float_surface_law;
mod frost;
mod geometry;
mod geometry_reshape;
mod glide_anchor_law;
mod hud;
mod images;
mod list_surfaces;
mod markdown;
mod markdown_headings;
mod nits;
mod distinguishability;
mod one_bit;
mod oracle;
mod outline;
mod overlay_align_law;
mod overlay_personality;
mod overlay_right_hug_law;
mod page_frame;
mod folds;
mod pixeldiff;
mod stars;
mod syntax_ligatures;
mod syntax_roles;
mod tables;
mod theme;
mod theme_caps_law;
mod washes;
mod wrap_affinity;
#[cfg(not(target_arch = "wasm32"))]
mod webgl_shader_validation;
mod wysiwyg;
mod zoom_anchor;

// 800px tall, TEXT_TOP 16, LINE_HEIGHT 32 -> floor((800-16)/32) = 24 rows.
pub(super) const H: f32 = 800.0;

/// Build a headless pipeline, or `None` if no wgpu adapter is available.
pub(super) fn headless_pipeline() -> Option<TextPipeline> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p = TextPipeline::new(
            &device,
            &queue,
            &cache,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        p.set_size(1200.0, 800.0);
        Some(p)
    })
}

/// A `(Device, Queue, TextPipeline)` triple sized `w`×`h`, or `None` on a
/// GPU-less machine — for tests that must READ what a real `prepare()` left in
/// the pipeline (instance counts, shaped-buffer geometry) and so need a device
/// and queue of their own to drive it.
pub(super) fn headless_dqp(w: f32, h: f32) -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl test device (dqp)"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(w, h);
        Some((device, queue, p))
    })
}

pub(super) fn view(text: &str, line: usize, col: usize) -> ViewState {
    ViewState {
        text: text.to_string(),
        cursor_line: line,
        cursor_col: col,
        ..ViewState::base()
    }
}

/// A markdown [`view`] — same as [`view`] but with `is_markdown` set, so the
/// styling + outline passes run (used by the margin-outline tests).
pub(super) fn view_md(text: &str, line: usize, col: usize) -> ViewState {
    let mut v = view(text, line, col);
    v.is_markdown = true;
    v
}
