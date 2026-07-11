//! Markdown styling spans — the "dim the markup + style the content" model for
//! awl's prose docs. We parse the document with `pulldown-cmark`'s OFFSET
//! iterator (each event carries its byte range into the source text) and turn
//! the events into a flat list of `(byte-range, MdKind)` spans. The renderer
//! lays these as per-span `Attrs` over each line's `AttrsList`, exactly like the
//! CJK + focus spans — the markup characters (`#`, `*`, `` ` ``, `>`, list
//! markers, link brackets/URL, `==`) recede to the DIM ink while staying fully
//! present and editable, and the CONTENT gains structure (bold weight, italic
//! style, mono code, heading SIZE, accent link text, a highlighter wash behind
//! `==marked==` text). Headings take NO accent color and
//! NO bold — figure/ground by value + size, so the amber stays the caret's alone
//! (DESIGN.md §3, the one-organic-element law) and the title renders in the world's
//! own face at any size — a DESIGN call: size alone carries the hierarchy. (Inline
//! `**bold**` DOES shape in a real bold face on proportional worlds — the 10
//! display faces bundle a 700 weight, `render::FONT_THEME_BOLD_FACES`; the mono
//! worlds stay Regular-only and bold falls back gracefully there.)
//!
//! This is PURE: the spans are a deterministic function of the text (no clock,
//! no layout), so a headless capture renders the settled styled state and the
//! sidecar can report the spans verbatim.
//!
//! HEADING SIZE: a heading's level now also drives a per-line font/line-height
//! SCALE (see [`heading_scale`]). The renderer reads it in `render.rs` to lay the
//! whole heading line at a larger `Attrs::metrics`, so headings render physically
//! BIGGER (not just bolder). This relies on render.rs's VARIABLE-row-height layout
//! pass (a per-row geometry table feeding scroll / hit-test / caret), so the kind
//! enum still carries only the LEVEL — the concrete pixel ramp lives in one place
//! ([`heading_scale`]) and every non-heading span kind stays line-height-neutral
//! (scale 1.0), keeping a plain prose / code buffer byte-identical.
//!
//! WYSIWYG (the PHILOSOPHY.md amendment): "if the caret is on that line, show the
//! actual markdown; otherwise show the preview." A settled markdown line already
//! dims its markup and styles its content; WYSIWYG goes one step further and
//! CONCEALS the markup entirely (transparent ink, same trick as the pre-existing
//! hr/bullet reveal-on-cursor) for headings, bold/italic, inline code, and
//! `==highlight==` off the caret's line, plus a fenced code block's marker lines
//! off the caret's whole BLOCK — seed [`MdKind::ConcealMarkup`] / [`ConcealKind`]
//! for which spans qualify and `render::spans::add_wysiwyg_conceal_spans` for the
//! mechanism. Gated by the sticky [`wysiwyg_on`] global (default ON; `false`
//! reproduces today's always-visible markup byte-identically) — mirrors
//! `nits::NITS_ON` / `spell::SPELLCHECK_ON` exactly: a process-global read by the
//! renderer, set once at launch from the config sticky pref (`config/`).

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the WYSIWYG markup conceal is active. DEFAULT ON — the editor opens
/// with headings/emphasis/inline-code/highlight markup hiding off the caret's
/// line (and a fenced block's markers hiding off the caret's whole block); OFF
/// reproduces the always-visible markup this round shipped without, byte-for-byte
/// (no conceal, no pill, no panel — just the pre-existing dim-the-markup styling).
static WYSIWYG_ON: AtomicBool = AtomicBool::new(true);

/// True when the WYSIWYG conceal is active (read by the renderer each reshape).
pub fn wysiwyg_on() -> bool {
    WYSIWYG_ON.load(Ordering::Relaxed)
}

/// Set the WYSIWYG conceal on/off explicitly — the config sticky-pref launch-
/// apply (mirrors [`crate::nits::set_nits_on`]).
pub fn set_wysiwyg_on(on: bool) {
    WYSIWYG_ON.store(on, Ordering::Relaxed);
}

/// Whether INLINE IMAGES are active. DEFAULT ON — a markdown `![alt](path.png)`
/// image reference conceals its source off the caret's line and reserves a TALL
/// row (fit-to-column, its display height), which the renderer fills with the
/// decoded image (the GPU draw lands next phase). OFF reproduces the
/// pre-feature rendering byte-for-byte: no image span is ever emitted (see
/// [`spans`]), so the `![alt](path)` source renders as plain default-ink text
/// exactly as it did before this round — no conceal, no tall row, no image.
///
/// NATIVE-ONLY: images read a file's header dimensions off disk and (next
/// phase) decode its pixels, neither of which the wasm build does — so
/// [`inline_images_on`] is unconditionally `false` on `wasm32`, making the
/// whole feature vanish there (the source renders plain, byte-identical to the
/// native-off case). Mirrors the daemon/session native-only gate.
static INLINE_IMAGES_ON: AtomicBool = AtomicBool::new(true);

/// True when inline images are active (read by [`spans`] to gate the image
/// span + by the renderer to gate the tall row / draw). Always `false` on wasm
/// (the feature is native-only — see [`INLINE_IMAGES_ON`]).
pub fn inline_images_on() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        INLINE_IMAGES_ON.load(Ordering::Relaxed)
    }
}

/// Set inline images on/off explicitly — the config sticky-pref launch-apply
/// (mirrors [`set_wysiwyg_on`]). A no-op-in-effect on wasm, where
/// [`inline_images_on`] ignores the flag and always reports `false`.
pub fn set_inline_images_on(on: bool) {
    INLINE_IMAGES_ON.store(on, Ordering::Relaxed);
}


mod conceal;
mod headings;
mod refs;
mod spans;
mod tables;

pub use conceal::ConcealKind;
pub use headings::{headings, headings_from_spans, heading_scale, type_scale, Heading};
#[allow(unused_imports)] // ImageRef: public API surface, no in-crate caller outside tests
pub use refs::{image_refs, image_width_hint_edit, link_at, link_at_full, parse_image_source, ImageRef, LinkAt};
#[allow(unused_imports)] // ListItem/READING_WPM: public API surface, no in-crate caller today
pub use spans::{
    break_kind, frontmatter_end, is_thematic_break, list_item, reading_time_min, spans,
    word_count, BreakKind, ListItem, MdKind, LIST_INDENT, READING_WPM,
};
pub(crate) use tables::ColAlign;
pub use tables::{align_table, table_block_lines};
#[allow(unused_imports)] // table_pan_max: public API surface, no in-crate caller today
pub(crate) use tables::{
    parse_col_align, split_row_cells, table_align_offset, table_column_layout, table_pan_bar,
    table_pan_clamp, table_pan_max,
};

#[cfg(test)]
mod tests;
