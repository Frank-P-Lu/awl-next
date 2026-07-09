//! UNIT TESTS for the `TextPipeline` GPU aggregation root, split by feature
//! area (the 2026-07 code-organization pass) out of one formerly-9.7k-line
//! `render::tests` module into this `render/tests/` directory -- every test's
//! NAME is unchanged, only its module path grew one segment
//! (`render::tests::foo` -> `render::tests::<area>::foo`). `use super::*;`
//! here still resolves to the `render` root exactly as before the split; each
//! child module re-derives render access directly via its own
//! `use super::super::*;` (a single glob, so it can never collide with a
//! sibling test module of the same name as a real render/theme module -- see
//! `theme.rs`/`geometry.rs`) plus a targeted `use super::{..};` for whichever
//! of this module's own shared test helpers it actually calls.

use super::*;

mod caret;
mod caret_block;
mod chrome_overlay;
mod chrome_panels;
mod cjk;
mod geometry;
mod geometry_reshape;
mod hud;
mod images;
mod markdown;
mod markdown_headings;
mod nits;
mod oracle;
mod outline;
mod syntax_ligatures;
mod syntax_roles;
mod tables;
mod theme;
mod washes;
mod wysiwyg;

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

pub(super) fn view(text: &str, line: usize, col: usize) -> ViewState {
    ViewState {
        text: text.to_string(),
        cursor_line: line,
        cursor_col: col,
        scroll_lines: 0,
        zoom: 1.0,
        selection: None,
        preedit: String::new(),
        misspelled: Vec::new(),
        is_edit_move: false,
        held: false,
        search_matches: Vec::new(),
        search_current: None,
        search_query: String::new(),
        search_active: false,
        search_case_sensitive: false,
        search_replace_active: false,
        search_replacement: String::new(),
        search_editing_replacement: false,
        overlay_active: false,
        overlay_crisp: false,
        overlay_query: String::new(),
        overlay_items: Vec::new(),
        overlay_empty: None,
        overlay_bindings: Vec::new(),
        overlay_times: Vec::new(),
        overlay_git: Vec::new(),
        overlay_selected: 0,
        overlay_scroll: 0,
        overlay_window_rows: 12,
        overlay_hint: String::new(),
    overlay_lens: Vec::new(),
    overlay_sections: Vec::new(),
        caret_preview: None,
        gutter_name: String::new(),
        gutter_project: String::new(),
        is_markdown: false,
        doc_dir: None,
        syn_lang: None,
        overlay_spell: None,
        notice: String::new(),
        cjk_priority: crate::frontmatter::DEFAULT_CJK_PRIORITY.to_vec(),
        eol: crate::buffer::Eol::Lf,
    }
}

/// A markdown [`view`] — same as [`view`] but with `is_markdown` set, so the
/// styling + outline passes run (used by the margin-outline tests).
pub(super) fn view_md(text: &str, line: usize, col: usize) -> ViewState {
    let mut v = view(text, line, col);
    v.is_markdown = true;
    v
}
