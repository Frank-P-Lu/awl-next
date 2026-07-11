//! WEB ESCAPE HATCH — "Download file" (`Action::DownloadFile`, `commands.rs`'s
//! `web_only: true`): export the active buffer's text as a browser download,
//! via a `Blob` + object URL + a synthetic `<a download>` click. This is the
//! one door that gets your words OFF the browser's virtual `localStorage`
//! filesystem (see `WEB.md`'s "Storage is `localStorage`, not a real disk")
//! and onto the user's actual disk — the escape hatch a native build never
//! needs (a native user's file already lives on real disk), which is exactly
//! why the catalog hides this command there.
//!
//! Two halves, split so the FILENAME derivation stays honestly unit-testable
//! from a plain native `cargo test` run (mirrors `commands::Platform::
//! current()`'s "assert the web view without an actual wasm build" pattern):
//! [`filename_for`] is pure and compiled everywhere; [`trigger_download`] is
//! the actual DOM handoff and only exists on `wasm32` (nothing to call it on
//! native — the command itself is gated off there by `web_only`).

/// The download's suggested FILENAME — a thin wrapper over
/// [`crate::buffer::Buffer::display_name`], the ONE existing owner of "what
/// does this buffer call itself" (already shared with the page-mode gutter),
/// so the export never re-derives naming logic. This is what escapes the
/// SCRATCH buffer too: a no-path buffer's `display_name()` already derives
/// its would-be-saved name (the slugified first line + `.md`, or the
/// `"scratch.md"` placeholder for an empty buffer) — the export uses exactly
/// that name, whether or not the buffer has ever been saved.
// Compiled (and unit-tested) on every platform for the reason in the module
// doc, but only ever CALLED from `App::download_file`'s `wasm32`-gated arm —
// on native there is no caller, so `dead_code` would otherwise fire there.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn filename_for(buffer: &crate::buffer::Buffer) -> String {
    buffer.display_name()
}

/// The actual DOM handoff, LIVE-APP-ONLY (`wasm32` only — nothing calls this
/// on native, since `Action::DownloadFile` is gated off there entirely by
/// `commands::action_available`). Builds a `text/plain` `Blob` from `text`,
/// a temporary object URL for it, clicks a synthetic `<a download="filename">`
/// anchor pointing at that URL (the standard no-server-round-trip browser
/// download recipe), then revokes the URL — best-effort throughout: any DOM
/// step failing (no `window`, no `document`, a rejected `Blob`/`Url` call)
/// degrades to a calm no-op, mirroring `follow_link`'s / the clipboard
/// mirror's swallowed-error discipline. Never panics.
#[cfg(target_arch = "wasm32")]
pub fn trigger_download(filename: &str, text: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };

    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(text));
    let opts = BlobPropertyBag::new();
    opts.set_type("text/plain");
    let Ok(blob) = Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = Url::create_object_url_with_blob(&blob) else { return };

    if let Ok(el) = document.create_element("a") {
        if let Ok(anchor) = el.dyn_into::<HtmlAnchorElement>() {
            anchor.set_href(&url);
            anchor.set_download(filename);
            anchor.click();
        }
    }
    let _ = Url::revoke_object_url(&url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    #[test]
    fn filename_for_a_saved_file_is_its_own_file_name() {
        let mut b = Buffer::from_str("hello");
        b.set_path(std::path::PathBuf::from("/tmp/notes.md"));
        assert_eq!(filename_for(&b), "notes.md");
    }

    #[test]
    fn filename_for_scratch_derives_the_same_name_a_save_would() {
        let b = Buffer::from_str("# My Title\nbody");
        // A no-path buffer derives the slugified first line, exactly like
        // display_name()/Buffer::save's own auto-naming.
        assert_eq!(filename_for(&b), b.display_name());
        assert!(filename_for(&b).ends_with(".md"));
    }

    #[test]
    fn filename_for_empty_scratch_falls_back_to_the_scratch_placeholder() {
        let b = Buffer::from_str("");
        assert_eq!(filename_for(&b), "scratch.md");
    }
}
