//! EAGER + KEYED spell-verdict cache tests (the completed-word-lag fix,
//! 2026-07): `App::recompute_spell_cache` + `crate::spell::visible` composing
//! correctly across a REAL `Buffer` edit, at the App level — no GPU, clock, or
//! fs needed (both are plain, deterministic state mutators). Complements the
//! pure `spell::tests` seam (which proves the KEYED mechanism itself in
//! isolation) with the actual cache field wired to a real edit sequence.
//!
//! `App::sync_view`'s EAGER half (calling `recompute_spell_cache` inline on
//! every buffer-version change) lives behind the `gpu.is_some()` gate — a
//! structural, live-only seam a hermetic App can't reach (mirrors every other
//! gpu-gated `sync_view` side effect). These tests instead call
//! `recompute_spell_cache` directly, exactly as `sync_view` would, and assert
//! the two things that actually matter: the cache reflects reality
//! immediately after a rescan (EAGER), and `spell::visible` — the one filter
//! `sync_view` routes `ViewState.misspelled` through — refuses to show a
//! verdict whose text has since changed even if a rescan is skipped (KEYED).

use super::*;
use std::path::PathBuf;

/// EAGER: a rescan run right after an edit already reflects the CURRENT text
/// — the cache holds no leftover verdict for a word that's since been fixed.
/// Mirrors what `App::sync_view`'s eager version check does on every text
/// change (no debounce step for the cache to "catch up" through).
#[test]
fn recompute_spell_cache_reflects_the_current_text_immediately() {
    if crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs).is_err() {
        return; // bundled dictionary unavailable in this environment
    }
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer = Buffer::from_str("helo world\n");
    app.recompute_spell_cache();
    let visible = crate::spell::visible(&app.spell_cache, &app.buffer.text());
    assert_eq!(visible.len(), 1, "helo starts out flagged: {visible:?}");
    assert_eq!((visible[0].start_col, visible[0].end_col), (0, 4));

    // Correct it IN PLACE at the SAME start column the verdict above is
    // keyed to ("helo" -> "hello", inserting 'l' before the trailing 'o').
    let idx = app.buffer.line_col_to_char(0, 3);
    app.buffer.set_cursor(idx);
    app.buffer.insert_text("l");
    assert_eq!(app.buffer.text(), "hello world\n");

    // EAGER rescan (what the very next `sync_view` does): the cache is fresh
    // again immediately — no leftover flagged span for the now-correct word,
    // and nothing else newly flagged either.
    app.recompute_spell_cache();
    let visible2 = crate::spell::visible(&app.spell_cache, &app.buffer.text());
    assert!(
        visible2.is_empty(),
        "the corrected word must not still be flagged: {visible2:?}"
    );
}

/// KEYED, at the App level: hold the STALE cache from BEFORE an edit (as if
/// the eager rescan somehow hadn't run yet — the belt-and-suspenders case)
/// and prove `spell::visible` refuses to paint it against the post-edit text.
/// This is exactly the just-completed-word flash the mechanism makes
/// structurally impossible: even a caller that skipped the eager recompute
/// can't show a stale squiggle over text that has since changed.
#[test]
fn a_stale_cache_never_paints_through_the_visible_filter_after_an_edit() {
    if crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs).is_err() {
        return;
    }
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer = Buffer::from_str("helo world\n");
    app.recompute_spell_cache();
    let stale_cache = app.spell_cache.clone();
    assert_eq!(
        crate::spell::visible(&stale_cache, &app.buffer.text()).len(),
        1,
        "sanity: the pre-edit cache does flag helo"
    );

    // The SAME correction as above, WITHOUT calling `recompute_spell_cache` —
    // simulating a read that lands before the eager rescan has caught up.
    let idx = app.buffer.line_col_to_char(0, 3);
    app.buffer.set_cursor(idx);
    app.buffer.insert_text("l");
    assert_eq!(app.buffer.text(), "hello world\n");

    assert!(
        crate::spell::visible(&stale_cache, &app.buffer.text()).is_empty(),
        "a verdict keyed to the pre-edit text must never paint on the post-edit text"
    );
}

/// SEQUENCE mirroring the reported bug shape end-to-end on a real `App`: type
/// a typo, let it flag, correct it, then check ANOTHER word on the same line
/// still flags (the fix is scoped to the CHANGED word, not a blanket cache
/// clear) while the corrected one stays clean through a further no-op rescan
/// (idempotent — a second eager pass changes nothing).
#[test]
fn eager_rescan_fixes_only_the_edited_word_leaving_a_real_typo_flagged() {
    if crate::spell::SpellChecker::new(crate::spell::DictVariant::EnUs).is_err() {
        return;
    }
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer = Buffer::from_str("helo wrld\n");
    app.recompute_spell_cache();
    let v1 = crate::spell::visible(&app.spell_cache, &app.buffer.text());
    assert_eq!(v1.len(), 2, "both helo and wrld start out flagged: {v1:?}");

    // Fix only "helo" -> "hello".
    let idx = app.buffer.line_col_to_char(0, 3);
    app.buffer.set_cursor(idx);
    app.buffer.insert_text("l");
    assert_eq!(app.buffer.text(), "hello wrld\n");
    app.recompute_spell_cache();

    let v2 = crate::spell::visible(&app.spell_cache, &app.buffer.text());
    assert_eq!(v2.len(), 1, "only wrld remains flagged: {v2:?}");
    let w2 = crate::spell::word_at(&app.buffer.text(), &v2[0]);
    assert_eq!(w2, "wrld", "the surviving flag is the real, still-unfixed typo");

    // A second eager pass over UNCHANGED text is idempotent.
    app.recompute_spell_cache();
    let v3 = crate::spell::visible(&app.spell_cache, &app.buffer.text());
    assert_eq!(v3, v2, "a repeat rescan of unchanged text must not move anything");
}
