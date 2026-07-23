//! Tests for the `markdown` module (spans, conceal, tables, headings,
//! refs) -- split verbatim out of the former `markdown.rs` monolith's
//! embedded `mod tests` (2026-07 code-organization pass); every test's NAME
//! and MODULE PATH are unchanged (`markdown::tests::foo`) -- only which
//! file its source lives in moved.

use super::spans::push_highlight_spans;
use super::*;
use std::ops::Range;

fn has(spans: &[(Range<usize>, MdKind)], lo: usize, hi: usize, k: MdKind) -> bool {
    spans.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
}

#[test]
fn parse_image_source_extracts_path_alt_and_hint() {
    // No hint: alt + path recovered, hint None.
    assert_eq!(
        parse_image_source("![a cat](cat.png)"),
        Some(ImageRef { alt: "a cat".into(), path: "cat.png".into(), width_hint: None })
    );
    // `|300` width hint parsed OUT of the alt (Obsidian convention).
    assert_eq!(
        parse_image_source("![a cat|300](cat.png)"),
        Some(ImageRef { alt: "a cat".into(), path: "cat.png".into(), width_hint: Some(300) })
    );
    // `|WxH` → the WIDTH is the hint (H rides the intrinsic aspect in v1).
    assert_eq!(
        parse_image_source("![cat|300x200](cat.png)"),
        Some(ImageRef { alt: "cat".into(), path: "cat.png".into(), width_hint: Some(300) })
    );
    // A NON-numeric `|` suffix is NOT a hint — the alt (which legitimately
    // contains `|`) is preserved verbatim.
    assert_eq!(
        parse_image_source("![a | b](cat.png)"),
        Some(ImageRef { alt: "a | b".into(), path: "cat.png".into(), width_hint: None })
    );
    // A `(path "title")` — the path is the first whitespace token.
    assert_eq!(
        parse_image_source("![x](cat.png \"my title\")"),
        Some(ImageRef { alt: "x".into(), path: "cat.png".into(), width_hint: None })
    );
    // Not an image: None (never panics).
    assert_eq!(parse_image_source("just text"), None);
    assert_eq!(parse_image_source("![no dest]"), None);
}

#[test]
fn image_width_hint_edit_inserts_replaces_and_bails_cleanly() {
    // INSERT: a hint-less alt gains `|NNN` after the alt text, before `]`.
    let src = "![a cat](cat.png)";
    let (b0, b1, new_alt) = image_width_hint_edit(src, 300).unwrap();
    assert_eq!(&src[b0..b1], "a cat", "byte range spans exactly the raw alt");
    assert_eq!(new_alt, "a cat|300");
    // Splicing the replacement into the range yields the Obsidian form.
    let spliced = format!("{}{}{}", &src[..b0], new_alt, &src[b1..]);
    assert_eq!(spliced, "![a cat|300](cat.png)");

    // REPLACE: an existing `|NNN` is swapped, the alt text preserved.
    let src = "![a cat|300](cat.png)";
    let (b0, b1, new_alt) = image_width_hint_edit(src, 512).unwrap();
    assert_eq!(&src[b0..b1], "a cat|300");
    assert_eq!(new_alt, "a cat|512");
    let spliced = format!("{}{}{}", &src[..b0], new_alt, &src[b1..]);
    assert_eq!(spliced, "![a cat|512](cat.png)");

    // A `|WxH` hint is also replaced (collapsing to the single width form).
    let (_, _, new_alt) = image_width_hint_edit("![cat|300x200](c.png)", 120).unwrap();
    assert_eq!(new_alt, "cat|120");

    // An alt that legitimately contains `|` (no numeric suffix) keeps it and
    // appends the new hint cleanly.
    let (_, _, new_alt) = image_width_hint_edit("![a | b](c.png)", 90).unwrap();
    assert_eq!(new_alt, "a | b|90");

    // An EMPTY alt gets a bare `|NNN`.
    let (_, _, new_alt) = image_width_hint_edit("![](c.png)", 64).unwrap();
    assert_eq!(new_alt, "|64");

    // Not a well-formed image -> None (never panics).
    assert_eq!(image_width_hint_edit("just text", 100), None);
    assert_eq!(image_width_hint_edit("![no dest]", 100), None);
}

#[test]
#[cfg(not(target_arch = "wasm32"))] // `inline_images_on()` is always false on wasm
fn spans_emits_image_conceal_span_when_on_and_nothing_when_off() {
    let _g = crate::testlock::serial();
    let prev = inline_images_on();
    // ON: the whole `![alt](path)` is one line-scoped ConcealMarkup(Image) span.
    set_inline_images_on(true);
    let src = "![a cat|300](cat.png)";
    let on = spans(src);
    assert!(
        on.iter().any(|(r, k)| *k == MdKind::ConcealMarkup(ConcealKind::Image)
            && r.start == 0
            && r.end == src.len()),
        "images ON should emit one ConcealMarkup(Image) over the whole ref: {on:?}"
    );
    // OFF (native): NO image span at all — byte-identical to the pre-feature
    // editor, which emitted no span for an image line.
    set_inline_images_on(false);
    let off = spans(src);
    assert!(
        !off.iter().any(|(_, k)| *k == MdKind::ConcealMarkup(ConcealKind::Image)),
        "images OFF should emit no image span: {off:?}"
    );
    set_inline_images_on(prev);
}

#[test]
fn heading_dims_hashes_and_styles_title() {
    let s = spans("# Title");
    // "# " (hash + space) is dim, WYSIWYG-concealable markup; "Title" is H1 content.
    let heading_markup = MdKind::ConcealMarkup(ConcealKind::Heading);
    assert!(has(&s, 0, 2, heading_markup), "leading '# ' should be markup: {s:?}");
    assert!(has(&s, 2, 7, MdKind::Heading(1)), "title should be h1: {s:?}");
}

#[test]
fn h2_level_detected() {
    let s = spans("## Sub");
    assert!(has(&s, 0, 3, MdKind::ConcealMarkup(ConcealKind::Heading)));
    assert!(has(&s, 3, 6, MdKind::Heading(2)));
}

#[test]
fn atx_closing_hashes_dim() {
    // `# Title #`: the leading `# ` AND the trailing ` #` both dim as Markup
    // (the backward close-fence scan), with `Title` the h1 content between.
    let s = spans("# Title #");
    let heading_markup = MdKind::ConcealMarkup(ConcealKind::Heading);
    assert!(has(&s, 0, 2, heading_markup), "leading '# ' dim: {s:?}");
    assert!(has(&s, 2, 7, MdKind::Heading(1)), "'Title' is h1: {s:?}");
    assert!(has(&s, 7, 9, heading_markup), "trailing ' #' close dim: {s:?}");
}

#[test]
fn headings_extracts_level_text_and_line() {
    let doc = "# Title\n\nsome prose\n\n## Section A\n\nbody\n\n### Deep\n";
    let h = headings(doc);
    assert_eq!(h.len(), 3, "three headings: {h:?}");
    assert_eq!(h[0], Heading { level: 1, text: "Title".into(), line: 0 });
    assert_eq!(h[1], Heading { level: 2, text: "Section A".into(), line: 4 });
    assert_eq!(h[2], Heading { level: 3, text: "Deep".into(), line: 8 });
    // The picker label indents two spaces per level below the top.
    assert_eq!(h[0].label(), "Title");
    assert_eq!(h[1].label(), "  Section A");
    assert_eq!(h[2].label(), "    Deep");
}

#[test]
fn headings_one_entry_per_line_for_styled_title() {
    // A title with an inline-styled run still yields ONE outline row for the
    // line (the first plain run), not a duplicate.
    let h = headings("# Hello *world*\n");
    assert_eq!(h.len(), 1, "one row per heading line: {h:?}");
    assert_eq!(h[0].line, 0);
    assert_eq!(h[0].level, 1);
}

#[test]
fn headings_empty_without_headings() {
    assert!(headings("just some prose\nwith no headings\n").is_empty());
}

#[test]
fn setext_underline_is_not_a_heading_in_the_outline() {
    // The reported bug: typing a `-`/`---`/`===` on the line below a paragraph
    // (a SETEXT heading to CommonMark) silently promoted that paragraph to an
    // outline heading — even though heading-SIZE (which counts leading `#`s)
    // never treated it as one. awl is ATX-only everywhere; the outline must
    // agree. A paragraph + underline yields ZERO outline headings.
    for underline in ["-", "---", "===", "=", "--------"] {
        let doc = format!("Just a sentence.\n{underline}\n");
        let hs = headings(&doc);
        assert!(
            hs.is_empty(),
            "paragraph + {underline:?} underline must NOT be an outline heading, got {hs:?}"
        );
    }
    // ATX `#` headings are unaffected — still extracted with level + title.
    let atx = headings("# Real Heading\n\nbody\n");
    assert_eq!(atx.len(), 1, "ATX heading still counts: {atx:?}");
    assert_eq!(atx[0].level, 1);
    assert_eq!(atx[0].text, "Real Heading");
}

#[test]
fn headings_from_spans_core_matches_the_wrapper() {
    // The persistent margin outline distills headings from an ALREADY-parsed
    // span list (no second pulldown parse); the core must produce the exact
    // same list as the text-only wrapper. Also proves the core is the shared
    // owner (the wrapper delegates to it).
    let doc = "# Title\n\nprose\n\n## Section A\n\nbody\n\n### Deep\n";
    let via_core = headings_from_spans(doc, &spans(doc));
    assert_eq!(via_core, headings(doc));
    assert_eq!(via_core.len(), 3, "three headings: {via_core:?}");
    assert_eq!(via_core[0], Heading { level: 1, text: "Title".into(), line: 0 });
    assert_eq!(via_core[1], Heading { level: 2, text: "Section A".into(), line: 4 });
    assert_eq!(via_core[2], Heading { level: 3, text: "Deep".into(), line: 8 });
}

#[test]
fn bold_run_has_dim_stars_and_bold_inner() {
    let s = spans("**bold**");
    let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
    assert!(has(&s, 0, 2, emph_markup), "opening ** dim: {s:?}");
    assert!(has(&s, 6, 8, emph_markup), "closing ** dim: {s:?}");
    assert!(has(&s, 2, 6, MdKind::Bold), "inner bold: {s:?}");
}

#[test]
fn bold_italic_triple_star() {
    // `***x***` is BOTH strong and emphasis: pulldown nests an emphasis (outer
    // single `*`) around a strong (inner `**`), so the inner `x` is BoldItalic
    // and the three stars at each end dim as Markup (outer 1 + inner 2).
    let s = spans("***x***");
    let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
    assert!(has(&s, 3, 4, MdKind::BoldItalic), "inner x is bold+italic: {s:?}");
    assert!(has(&s, 0, 1, emph_markup), "outer opening `*` dim: {s:?}");
    assert!(has(&s, 1, 3, emph_markup), "inner opening `**` dim: {s:?}");
    assert!(has(&s, 4, 6, emph_markup), "inner closing `**` dim: {s:?}");
    assert!(has(&s, 6, 7, emph_markup), "outer closing `*` dim: {s:?}");
}

#[test]
fn italic_underscore() {
    let s = spans("_it_");
    let emph_markup = MdKind::ConcealMarkup(ConcealKind::Emphasis);
    assert!(has(&s, 0, 1, emph_markup));
    assert!(has(&s, 3, 4, emph_markup));
    assert!(has(&s, 1, 3, MdKind::Italic));
}

#[test]
fn inline_code_dims_backticks() {
    let s = spans("`code`");
    let code_markup = MdKind::ConcealMarkup(ConcealKind::Code);
    assert!(has(&s, 0, 1, code_markup));
    assert!(has(&s, 5, 6, code_markup));
    assert!(has(&s, 1, 5, MdKind::Code { inline: true }));
}

#[test]
fn link_markup_conceals_and_text_stays_content_ink() {
    // `[awl](http://x)`: the `[` and the `](http://x)` tail are the concealable
    // PLUMBING (WYSIWYG `Link`); the visible text `awl` keeps its own content-ink
    // `LinkText` span. The old whole-range dim `Markup` span is gone — off-caret
    // the plumbing hides to zero-width and only the text shows.
    let s = spans("[awl](http://x)");
    let link = MdKind::ConcealMarkup(ConcealKind::Link);
    assert!(has(&s, 0, 1, link), "opening '[' conceals: {s:?}");
    assert!(has(&s, 4, 15, link), "'](url)' tail conceals: {s:?}");
    assert!(has(&s, 1, 4, MdKind::LinkText), "link text stays content ink: {s:?}");
    // The text bytes are NOT covered by any conceal span (so the conceal pass
    // never hides the visible text).
    assert!(
        !s.iter().any(|(r, k)| *k == link && r.start <= 1 && r.end >= 4),
        "link text must not sit under a conceal span: {s:?}"
    );
}

#[test]
fn reference_link_falls_back_to_plain_markup_no_conceal() {
    // A reference-style link (`[text][ref]`) has no `](`, so it keeps the plain
    // NON-concealing `Markup` — never mis-concealed.
    let s = spans("[awl][ref]\n\n[ref]: http://x\n");
    assert!(
        s.iter().any(|(r, k)| r.start == 0 && *k == MdKind::Markup),
        "reference link is plain Markup: {s:?}"
    );
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::ConcealMarkup(ConcealKind::Link)),
        "no Link conceal span for a reference link: {s:?}"
    );
}

#[test]
fn link_at_returns_url_inside_and_none_outside() {
    // `see [the essay](http://x/y) now`
    //  0123456789...  caret in `essay` (byte ~9) is inside the link.
    let text = "see [the essay](http://x/y) now";
    let inside = text.find("essay").unwrap() + 1; // a byte within the link text
    assert_eq!(link_at(text, inside).as_deref(), Some("http://x/y"));
    // Caret in the leading `see ` prose (byte 1) is OUTSIDE every link.
    assert_eq!(link_at(text, 1), None);
    // Caret in the trailing ` now` prose is outside too.
    assert_eq!(link_at(text, text.find("now").unwrap()), None);
    // A doc with no link at all: always None.
    assert_eq!(link_at("just prose here", 3), None);
}

#[test]
fn blockquote_marker_conceals_text_quote() {
    // The `> ` marker is now a WYSIWYG-concealable `Blockquote` span (not plain
    // `Markup`): dim off-cursor, zero-width off the caret's line — the pull-quote
    // round. The body text keeps its `Quote` styling span.
    let bq = MdKind::ConcealMarkup(ConcealKind::Blockquote);
    let s = spans("> quoted");
    assert!(has(&s, 0, 2, bq), "'> ' marker conceal span: {s:?}");
    assert!(s.iter().any(|(_, k)| *k == MdKind::Quote), "quote text: {s:?}");
}

#[test]
fn multiline_and_nested_quote_markers_conceal() {
    // A two-line blockquote emits ONE `Blockquote` conceal span per line (the
    // per-line `[ \t]*(> ?)+` scan), not one for the whole range — so each line
    // conceals/reveals independently on the caret's line.
    let bq = MdKind::ConcealMarkup(ConcealKind::Blockquote);
    let s = spans("> a\n> b");
    assert!(has(&s, 0, 2, bq), "first line '> ' marker: {s:?}");
    assert!(has(&s, 4, 6, bq), "second line '> ' marker: {s:?}");
    // A nested `>>` conceals its whole leading marker run as one span.
    let s = spans(">> deep");
    assert!(has(&s, 0, 3, bq), "'>> ' nested marker run: {s:?}");
}

#[test]
fn list_marker_dim() {
    let s = spans("- item");
    assert!(has(&s, 0, 2, MdKind::ListMarker), "marker dim: {s:?}");
}

/// THE NESTED-LIST MIS-HIGHLIGHT FIX: a nested item's `ListMarker` span covers its
/// WHOLE prefix — indent + marker + space — not just the marker, mirroring the
/// shared [`list_item`] scanner's own `0..content` shape. Before the fix, pulldown's
/// `Tag::Item` range (which starts at the marker CHARACTER, excluding indentation)
/// meant a nested item's leading indent bytes carried NO span at all — the "a space
/// missing for syntax highlighting" gap the notes named. Two levels deep + an
/// ordered nested item are all covered, so the fix generalizes past depth 1.
#[test]
fn nested_list_marker_dims_its_full_indent_prefix() {
    // Depth 1 (2-space indent): the marker span starts at byte 0 of "  - nested",
    // covering the indent too, not just "- " at byte 2.
    let doc = "- top\n  - nested\n";
    let s = spans(doc);
    assert!(
        has(&s, 6, 10, MdKind::ListMarker),
        "'  - ' (indent + marker + space) is ONE dim span: {s:?}"
    );
    assert!(
        !s.iter().any(|(r, k)| *k == MdKind::ListMarker && r.start == 8),
        "the marker span no longer starts mid-indent (the old excluded-indent bug): {s:?}"
    );

    // Depth 2 (4-space indent).
    let doc2 = "- top\n  - mid\n    - deep\n";
    let s2 = spans(doc2);
    // "- top\n" = 6 bytes, "  - mid\n" = 8 bytes -> depth-2 line starts at 14.
    assert!(
        has(&s2, 14, 20, MdKind::ListMarker),
        "'    - ' at depth 2 dims its full 4-space indent + marker: {s2:?}"
    );

    // A nested ORDERED item ("  1. text") gets the same full-prefix treatment.
    let doc3 = "- top\n  1. nested ordered\n";
    let s3 = spans(doc3);
    assert!(
        has(&s3, 6, 11, MdKind::ListMarker),
        "'  1. ' (indent + ordered marker + space) is ONE dim span: {s3:?}"
    );
}

/// The same nested-marker fix applies whether the nested item's CONTENT is plain
/// text, styled inline markup, or (the reported repro) an image reference — the
/// marker span is derived from the LINE alone, before any content-specific span is
/// pushed, so it can never depend on what follows it.
#[test]
fn nested_list_marker_fix_is_content_independent() {
    let plain = spans("- top\n  - plain nested\n");
    assert!(has(&plain, 6, 10, MdKind::ListMarker), "plain content: {plain:?}");

    let bold = spans("- top\n  - **bold** nested\n");
    assert!(has(&bold, 6, 10, MdKind::ListMarker), "bold content: {bold:?}");

    let prev = inline_images_on();
    set_inline_images_on(true);
    let img = spans("- top\n  - ![caption|400](x.png)\n");
    assert!(has(&img, 6, 10, MdKind::ListMarker), "image content: {img:?}");
    // The image's own conceal span starts EXACTLY where the marker span ends —
    // no gap, no overlap.
    assert!(
        img.iter().any(|(r, k)| *k == MdKind::ConcealMarkup(ConcealKind::Image) && r.start == 10),
        "the image span picks up right where the fixed marker span ends: {img:?}"
    );
    set_inline_images_on(prev);
}

/// ITEM 41 regression — a list item that OWNS a nested child must render its
/// OWN text at BODY weight, exactly like a childless sibling. The reopened bug
/// (survived item 4c, reported twice) showed the parent bolded — a
/// loose-list/span-range sibling of the 4c marker-range defect. A row's shaped
/// WEIGHT/STYLE is derived PURELY from the md-span KIND over its content bytes
/// (`render::md_attrs`: only `Bold`/`BoldItalic`/`Heading` re-weight and
/// `Italic` re-slants — a bare `ListMarker`/`Quote` only recolours, never
/// re-weights; verified live by pixel arithmetic, parent-content ink == sibling
/// ink on both a mono and a proportional world), so the pure-parse oracle for
/// "renders at body weight" is: NO weight/style span may cover a parent item's
/// CONTENT range. This is the neighborhood's missing appearance law — it pins
/// the plain nested list, a task parent, and a blockquote nested under a parent,
/// where bugs cluster.
#[test]
fn nested_parent_item_content_stays_body_weight_item_41() {
    // A span whose KIND re-weights or re-slants the run it covers — the exact
    // set `render::md_attrs` turns into a non-body face. `ListMarker`/`Quote`
    // recolour only, so they are deliberately NOT here.
    fn reweights(k: &MdKind) -> bool {
        matches!(
            k,
            MdKind::Bold | MdKind::BoldItalic | MdKind::Italic | MdKind::Heading(_)
        )
    }
    // True if any re-weighting span OVERLAPS the half-open content range [lo, hi).
    fn content_reweighted(spans: &[(Range<usize>, MdKind)], lo: usize, hi: usize) -> bool {
        spans.iter().any(|(r, k)| reweights(k) && r.start < hi && lo < r.end)
    }

    // Plain nested list: "parent" (bytes 2..8) OWNS the "  - child" nested item;
    // "sibling" (bytes 21..28) is childless. NEITHER content run may re-weight —
    // the childless sibling is the control that proves the parent's weight is
    // the anomaly, not a global style.
    let doc = "- parent\n  - child\n- sibling\n";
    let s = spans(doc);
    assert!(
        !content_reweighted(&s, 2, 8),
        "the parent-with-nested-child stays body weight (item 41): {s:?}"
    );
    assert!(
        !content_reweighted(&s, 21, 28),
        "the childless sibling stays body weight (the control): {s:?}"
    );

    // TASK parent with a nested task child — same law across the task neighborhood.
    let task = "- [ ] parent\n  - [ ] child\n";
    let st = spans(task);
    assert!(
        !content_reweighted(&st, 6, 12),
        "a task parent with a nested task child stays body weight: {st:?}"
    );

    // A blockquote nested under a parent list item — the parent's OWN text is
    // still body weight (the quote body recolours, but color is not weight).
    let bq = "- parent\n  > note\n- sibling\n";
    let sb = spans(bq);
    assert!(
        !content_reweighted(&sb, 2, 8),
        "a list parent above a nested blockquote stays body weight: {sb:?}"
    );
}

#[test]
fn table_pipes_separator_and_header_spans() {
    //        0      7 9        (line 0 "| a | b |" is 9 bytes incl newline at 9)
    let doc = "| a | b |\n|---|---|\n| c | d |\n";
    let s = spans(doc);
    // Every literal `|` on a data row is a dim TablePipe span. Header row pipes
    // sit at bytes 0, 4, 8.
    assert!(has(&s, 0, 1, MdKind::TablePipe), "leading header pipe: {s:?}");
    assert!(has(&s, 4, 5, MdKind::TablePipe), "middle header pipe: {s:?}");
    assert!(has(&s, 8, 9, MdKind::TablePipe), "trailing header pipe: {s:?}");
    // The separator row (`|---|---|`, bytes 10..19) is ONE dim TableSep span; its
    // pipes are NOT separately emitted as TablePipe.
    assert!(has(&s, 10, 19, MdKind::TableSep), "separator row dim: {s:?}");
    assert!(
        !s.iter().any(|(r, k)| *k == MdKind::TablePipe && r.start >= 10 && r.end <= 19),
        "no TablePipe inside the separator row: {s:?}"
    );
    // The header CELLS get the (no-op, full-ink) TableHeader tag; body cells do not.
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::TableHeader),
        "a header cell is tagged TableHeader: {s:?}"
    );
    // A body-row pipe on line 2 (byte 20) is still a TablePipe.
    assert!(has(&s, 20, 21, MdKind::TablePipe), "body-row pipe: {s:?}");
}

#[test]
fn aligned_separator_colons_dim_whole_row() {
    // A `:--:` / `:---` alignment separator is still recognized as the sep row and
    // dimmed whole (colons included).
    let doc = "| a | b |\n|:--|--:|\n| c | d |\n";
    let s = spans(doc);
    assert!(has(&s, 10, 19, MdKind::TableSep), "aligned separator row dim: {s:?}");
}

#[test]
fn non_table_pipe_in_prose_is_not_table_markup() {
    // A stray `|` in ordinary prose (no separator row => pulldown never rules it a
    // table) is never a TablePipe — we only scan INSIDE a parsed table range.
    let s = spans("a | b is a pipe, not a table\n");
    assert!(
        !s.iter().any(|(_, k)| k.is_table_markup()),
        "a prose pipe is not table markup: {s:?}"
    );
}

#[test]
fn align_table_pads_ragged_and_is_idempotent() {
    // A ragged, messily-spaced GFM table: uneven cell widths, a missing trailing
    // cell on the last row. Align re-pads so every `|` lines up.
    let src = "| Name | Value |\n|---|---|\n| a | 100 |\n| bb |";
    let out = align_table(src);
    let want = "| Name | Value |\n| ---- | ----- |\n| a    | 100   |\n| bb   |       |";
    assert_eq!(out, want, "ragged input aligns + fills the missing cell");
    // IDEMPOTENT: aligning the aligned output is a fixed point.
    assert_eq!(align_table(&out), out, "already-aligned input is unchanged");
}

#[test]
fn align_table_preserves_alignment_markers() {
    // `:---` left, `---:` right, `:--:` center — the colons must survive, and each
    // column is floored to its marker's minimum width (left/right≥2, center≥3), so
    // the one-char cells widen to keep the separator valid.
    let src = "| a | b | c |\n|:--|--:|:-:|\n| 1 | 2 | 3 |";
    let out = align_table(src);
    let want = "| a  | b  | c   |\n| :- | -: | :-: |\n| 1  | 2  | 3   |";
    assert_eq!(out, want, "left/right/center markers preserved: {out}");
    // A wider column keeps the markers at the ENDS, dashes in the middle.
    let src2 = "| xxxx | y | zzz |\n|:--|--:|:-:|\n| 1 | 2 | 3 |";
    let out2 = align_table(src2);
    let want2 = "| xxxx | y  | zzz |\n| :--- | -: | :-: |\n| 1    | 2  | 3   |";
    assert_eq!(out2, want2, "markers hug the ends at width: {out2}");
}

#[test]
fn align_table_uses_display_width_for_cjk() {
    // A CJK cell counts as 2 columns each, so the Latin column pads to match its
    // DISPLAY width, not its byte length (5 bytes for a 2-col wide char would
    // over-pad; the width helper counts it as 2).
    let src = "| 名前 | v |\n|---|---|\n| x | yy |";
    let out = align_table(src);
    // "名前" is 4 display cols; "x" pads to 4; header dashes fill 4.
    let want = "| 名前 | v  |\n| ---- | -- |\n| x    | yy |";
    assert_eq!(out, want, "CJK cell uses display width: {out}");
}

#[test]
fn table_block_lines_finds_the_block_and_needs_a_separator() {
    let text = "intro\n| a | b |\n|---|---|\n| c | d |\n\ntail | pipe";
    let lines: Vec<&str> = text.split('\n').collect();
    // Caret on any of the three table lines (1,2,3) finds the same [1,4) block.
    for caret in 1..=3 {
        assert_eq!(
            table_block_lines(&lines, caret),
            Some((1, 4)),
            "caret on table line {caret} finds the block"
        );
    }
    // Caret on prose (line 0) or the pipe-bearing-but-separator-less tail (line 5)
    // is None — a pipe run with no separator row is never a table.
    assert_eq!(table_block_lines(&lines, 0), None, "prose line is not a table");
    assert_eq!(table_block_lines(&lines, 5), None, "pipe prose w/o sep is not a table");
}

#[test]
fn table_conceal_span_covers_the_whole_block() {
    // The WYSIWYG whole-table conceal span spans the table's exact byte range.
    let text = "| a | b |\n|---|---|\n| c | d |\n";
    let s = spans(text);
    let table_end = "| a | b |\n|---|---|\n| c | d |".len();
    assert!(
        s.iter().any(|(r, k)| *k == MdKind::ConcealMarkup(ConcealKind::Table)
            && r.start == 0
            && r.end >= table_end),
        "whole-table conceal span present: {s:?}"
    );
}

#[test]
fn table_column_layout_fits_keeps_max_content() {
    // Regime 1 (fits): max-content total (200 + 2*10 = 220) < avail => columns
    // keep their max-content widths, left-anchored, gaps applied.
    let (xs, ws) = table_column_layout(&[20.0, 20.0, 20.0], &[100.0, 60.0, 40.0], 10.0, 1000.0);
    assert_eq!(ws, vec![100.0, 60.0, 40.0], "fitting keeps max-content widths");
    assert_eq!(xs[0], 0.0);
    assert!((xs[1] - 110.0).abs() < 1e-3, "col1 = 100 + gap 10");
    assert!((xs[2] - 180.0).abs() < 1e-3, "col2 = 110 + 60 + gap 10");
    // Empty input is inert.
    assert_eq!(table_column_layout(&[], &[], 10.0, 100.0), (vec![], vec![]));
}

#[test]
fn table_column_layout_squeeze_distributes_surplus_never_below_word_floor() {
    // A TOKEN column (min == max: "Time" fits exactly, a single word) and a
    // PHRASE column (min 40 = its longest word, max 300 = the whole phrase on
    // one line). Total max = 360 + gap 10 = 370 > avail 200, but total min =
    // 80 + gap 10 = 90 < 200 => the squeeze regime. The token column must stay
    // rigid at its width; the phrase column absorbs the whole squeeze.
    let mins = [40.0, 40.0]; // phrase longest-word, token whole-word
    let maxs = [300.0, 40.0]; // phrase whole-phrase, token (min == max)
    let (_xs, ws) = table_column_layout(&mins, &maxs, 10.0, 200.0);
    // The token column (no max−min spread) never yields — stays at its width.
    assert!((ws[1] - 40.0).abs() < 1e-3, "token column stays rigid: {ws:?}");
    // The phrase column absorbs the squeeze but NEVER drops below its word floor.
    assert!(ws[0] >= mins[0] - 1e-3, "phrase column keeps its word floor: {ws:?}");
    // The grid lands exactly at avail (200 = ws0 + ws1 + gap 10).
    let total = ws[0] + ws[1] + 10.0;
    assert!((total - 200.0).abs() < 1e-3, "squeeze lands at avail: {total}");
}

#[test]
fn table_column_layout_overflow_holds_word_floors_and_pans() {
    // Regime 3: the min-content floors themselves exceed avail. Every column
    // holds its floor (a word is NEVER broken to fit); the grid overflows and
    // pans rather than shrinking a column below its longest word.
    let mins = [120.0, 120.0];
    let maxs = [200.0, 200.0];
    let (xs, ws) = table_column_layout(&mins, &maxs, 10.0, 150.0);
    assert!((ws[0] - 120.0).abs() < 1e-3, "col0 holds its word floor: {ws:?}");
    assert!((ws[1] - 120.0).abs() < 1e-3, "col1 holds its word floor: {ws:?}");
    // The laid grid EXCEEDS avail (250 total) — it grows into the margins / pans.
    let total = xs[1] + ws[1];
    assert!(total > 150.0, "overflow grid exceeds avail (pans): {total}");
}

#[test]
fn table_pan_clamp_and_max_stay_on_rails() {
    // Nothing to pan when the grid fits.
    assert_eq!(table_pan_max(100.0, 200.0), 0.0, "fitting grid: no pan room");
    assert_eq!(table_pan_clamp(50.0, 100.0, 200.0), 0.0, "clamp kills a stale pan");
    // Overflow: pan room = content − view.
    assert!((table_pan_max(500.0, 200.0) - 300.0).abs() < 1e-3);
    assert!((table_pan_clamp(1000.0, 500.0, 200.0) - 300.0).abs() < 1e-3, "clamped to max");
    assert!((table_pan_clamp(-10.0, 500.0, 200.0)).abs() < 1e-3, "clamped to 0");
    assert!((table_pan_clamp(120.0, 500.0, 200.0) - 120.0).abs() < 1e-3, "in-range passes");
}

#[test]
fn table_pan_bar_is_a_proportional_thumb_or_none() {
    // A fitting grid shows no bar.
    assert_eq!(table_pan_bar(150.0, 200.0, 0.0, 10.0, 100.0, 3.0), None);
    // Overflow: the thumb width is the visible fraction of the track; at pan 0 it
    // sits at the table's left; at full pan it sits flush right.
    let content = 400.0;
    let view = 200.0;
    let left = 10.0;
    let bottom = 100.0;
    let thick = 3.0;
    let at0 = table_pan_bar(content, view, 0.0, left, bottom, thick).unwrap();
    // width = view * (view/content) = 200 * 0.5 = 100.
    assert!((at0[2] - 100.0).abs() < 1e-3, "thumb is the visible fraction: {at0:?}");
    assert!((at0[0] - left).abs() < 1e-3, "pan 0 sits at the table left: {at0:?}");
    assert!((at0[1] - (bottom - thick)).abs() < 1e-3, "bar hugs the bottom edge");
    let full = table_pan_bar(content, view, table_pan_max(content, view), left, bottom, thick)
        .unwrap();
    // Right edge flush with the viewport right (left + view).
    assert!((full[0] + full[2] - (left + view)).abs() < 1e-2, "full pan ends flush: {full:?}");
}

#[test]
fn table_align_offset_honors_alignment_and_clamps_overflow() {
    let pad = 4.0;
    let col = 100.0;
    let cell = 20.0;
    assert!((table_align_offset(ColAlign::Left, col, cell, pad) - pad).abs() < 1e-3);
    assert!((table_align_offset(ColAlign::None, col, cell, pad) - pad).abs() < 1e-3);
    // Right: 100 - 20 - 4 = 76.
    assert!((table_align_offset(ColAlign::Right, col, cell, pad) - 76.0).abs() < 1e-3);
    // Center: (100 - 20)/2 = 40.
    assert!((table_align_offset(ColAlign::Center, col, cell, pad) - 40.0).abs() < 1e-3);
    // Over-wide cell (wider than its column): every alignment left-anchors at pad.
    for a in [ColAlign::Left, ColAlign::Right, ColAlign::Center] {
        let off = table_align_offset(a, col, 200.0, pad);
        assert!((off - pad).abs() < 1e-3, "over-wide {a:?} clamps to pad: {off}");
    }
}

#[test]
fn ordered_list_markers_dim() {
    // `1. ` and `12) ` ordered markers (digit run + `.`/`)` + space) dim as the
    // ListMarker role, just like a bullet.
    let s = spans("1. item");
    assert!(has(&s, 0, 3, MdKind::ListMarker), "'1. ' ordered marker: {s:?}");
    let s = spans("12) item");
    assert!(has(&s, 0, 4, MdKind::ListMarker), "'12) ' ordered marker: {s:?}");
    // A bare number that is NOT a list (no `.`/`)`) must not be mis-marked.
    let s = spans("12 monkeys");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::ListMarker),
        "a plain number-led line is not a list: {s:?}"
    );
}

#[test]
fn list_item_detects_unordered_depth_and_marker() {
    // Top-level bullet: no indent, depth 0, unordered, content after "- ".
    let it = list_item("- item").expect("a bullet is a list item");
    assert_eq!(it.indent, 0);
    assert_eq!(it.depth(), 0);
    assert!(!it.ordered);
    assert_eq!(it.content, 2);
    assert!(!it.empty);
    // Nesting is by leading spaces, 2 per level: 2 -> depth 1, 4 -> depth 2.
    assert_eq!(list_item("  * nested").unwrap().depth(), 1);
    assert_eq!(list_item("    + deep").unwrap().depth(), 2);
    // Any of -,*,+ counts (the glyph is depth-derived, not char-derived).
    assert!(!list_item("+ plus").unwrap().ordered);
}

#[test]
fn list_item_detects_ordered_and_empty_and_rejects_non_lists() {
    let it = list_item("1. first").expect("ordered item");
    assert!(it.ordered);
    assert_eq!(it.depth(), 0);
    assert_eq!(list_item("  12) two").unwrap().depth(), 1);
    // An empty item (marker only) is flagged so Enter can END the list.
    assert!(list_item("- ").unwrap().empty);
    assert!(list_item("  1. ").unwrap().empty);
    // Non-lists: a bare dash (no space), a plain number, and prose.
    assert!(list_item("-nope").is_none());
    assert!(list_item("12 monkeys").is_none());
    assert!(list_item("just prose").is_none());
    assert!(list_item("").is_none());
}

#[test]
fn list_nesting_level_is_two_spaces() {
    // The list STRUCTURE ratio lives here; the depth→glyph mapping moved to the
    // theme (per-world bullets) — see `theme::tests::every_world_has_a_bullet_pair`.
    assert_eq!(LIST_INDENT, 2, "one nesting level is two spaces");
}

#[test]
fn fenced_and_indented_code_block_body_is_code() {
    // A FENCED block dims the WHOLE range as the WYSIWYG-concealable
    // `ConcealMarkup(Fence)` (fences + info), then the body Text overrides to
    // mono `Code { inline: false }` with HIGHEST priority.
    let s = spans("```\nlet x=1;\n```");
    assert!(
        has(&s, 0, 16, MdKind::ConcealMarkup(ConcealKind::Fence)),
        "whole fenced block is the concealable Fence markup: {s:?}"
    );
    assert!(has(&s, 4, 13, MdKind::Code { inline: false }), "fenced body is Code: {s:?}");
    // An INDENTED (no-fence) code block: the body (range excludes the 4-space
    // indent) is Code, and the whole-block wrapper stays PLAIN (non-concealing)
    // `Markup` — no fence to hide behind a panel.
    let s = spans("    code\n");
    assert!(has(&s, 4, 9, MdKind::Code { inline: false }), "indented body is Code: {s:?}");
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Markup),
        "an indented block's wrapper stays plain, non-concealing Markup: {s:?}"
    );
    assert!(
        !s.iter().any(|(_, k)| matches!(k, MdKind::ConcealMarkup(ConcealKind::Fence))),
        "an indented block must never carry the Fence conceal kind: {s:?}"
    );
}

#[test]
fn rust_tagged_fence_highlights_body_and_dims_markers() {
    use crate::syntax::{Lang, SynKind};
    // ```rust\n// c\nlet s="x";\n```
    //  bytes: fence+info "```rust" 0..7, body "// c\n" 8..13, `let s="x";\n` 13..24,
    //  closing "```" 24..27.
    let doc = "```rust\n// c\nlet s=\"x\";\n```";
    let s = spans(doc);
    // The fenced body's comment + string literal carry the Alabaster ROLE spans
    // (in the fence's language), translated into DOCUMENT byte offsets.
    assert!(
        has(&s, 8, 12, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Rust }),
        "'// c' is a rust comment role span: {s:?}"
    );
    assert!(
        has(&s, 19, 22, MdKind::CodeSyntax { role: SynKind::Str, lang: Lang::Rust }),
        "'\"x\"' is a rust string role span: {s:?}"
    );
    // The fence markers + the info string ("rust") stay dim, WYSIWYG-concealable
    // `ConcealMarkup(Fence)` — the whole block is dimmed first and NO role span
    // ever falls on the info-string bytes.
    assert!(
        s.iter().any(|(r, k)| {
            *k == MdKind::ConcealMarkup(ConcealKind::Fence) && r.start <= 3 && r.end >= 7
        }),
        "the info string 'rust' stays markup: {s:?}"
    );
    assert!(
        !s.iter().any(|(r, k)| matches!(k, MdKind::CodeSyntax { .. }) && r.start < 8),
        "no role span may touch the fence/info bytes before the body: {s:?}"
    );
}

#[test]
fn sh_tagged_fence_maps_to_bash_and_highlights_comment() {
    use crate::syntax::{Lang, SynKind};
    // ```sh\n# hi\n``` — the `sh` info string maps to the Bash lexer.
    let s = spans("```sh\n# hi\n```");
    assert!(
        has(&s, 6, 10, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Bash }),
        "'# hi' is a bash comment role span: {s:?}"
    );
}

#[test]
fn tilde_fence_highlights_body_same_as_backtick_fence() {
    use crate::syntax::{Lang, SynKind};
    // A `~~~` fence (pulldown's OTHER `CodeBlockKind::Fenced` delimiter) must
    // hit the exact same fence-syntax path as a backtick fence — the parse is
    // delimiter-agnostic by construction (pulldown reports both as `Fenced`),
    // this pins that with a real assertion rather than leaving it unverified.
    let doc = "~~~rust\n// c\nlet s=\"x\";\n~~~";
    let s = spans(doc);
    assert!(
        has(&s, 8, 12, MdKind::CodeSyntax { role: SynKind::Comment, lang: Lang::Rust }),
        "'// c' is a rust comment role span under a tilde fence: {s:?}"
    );
    assert!(
        has(&s, 19, 22, MdKind::CodeSyntax { role: SynKind::Str, lang: Lang::Rust }),
        "'\"x\"' is a rust string role span under a tilde fence: {s:?}"
    );
    assert!(
        s.iter().any(|(r, k)| {
            *k == MdKind::ConcealMarkup(ConcealKind::Fence) && r.start <= 3 && r.end >= 7
        }),
        "the info string 'rust' stays markup under a tilde fence: {s:?}"
    );
    assert!(
        !s.iter().any(|(r, k)| matches!(k, MdKind::CodeSyntax { .. }) && r.start < 8),
        "no role span may touch the fence/info bytes before the body: {s:?}"
    );
}

#[test]
fn unknown_and_no_lang_and_indented_fences_stay_plain_code() {
    // An UNKNOWN language: body stays plain mono Code, no role spans.
    let s = spans("```plaintext\n// c\n```");
    assert!(
        !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
        "an unknown-lang fence must not highlight: {s:?}"
    );
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Code { inline: false }),
        "body is still Code: {s:?}"
    );
    // A NO-LANG bare fence: same — plain Code, no role spans.
    let s = spans("```\n// c\n```");
    assert!(
        !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
        "a no-lang fence must not highlight: {s:?}"
    );
    // An INDENTED code block: no info string at all, so no role spans.
    let s = spans("    // c\n");
    assert!(
        !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
        "an indented block must not highlight: {s:?}"
    );
}

#[test]
fn non_fence_markdown_emits_no_code_syntax() {
    // Prose, headings, emphasis, inline code — none of these produce a fence
    // syntax span, so a non-fence markdown buffer stays byte-identical.
    let s = spans("# Title\n\nsome **bold** and `inline` words\n");
    assert!(
        !s.iter().any(|(_, k)| matches!(k, MdKind::CodeSyntax { .. })),
        "non-fence markdown must not emit CodeSyntax: {s:?}"
    );
}

#[test]
fn plain_prose_has_no_spans() {
    assert!(spans("just some words").is_empty());
}

#[test]
fn highlight_basic_pair_dims_markers_and_marks_content() {
    let s = spans("==marked==");
    let hl_markup = MdKind::ConcealMarkup(ConcealKind::Highlight);
    assert!(has(&s, 0, 2, hl_markup), "opening == dim: {s:?}");
    assert!(has(&s, 8, 10, hl_markup), "closing == dim: {s:?}");
    assert!(has(&s, 2, 8, MdKind::Highlight), "inner content highlighted: {s:?}");
}

#[test]
fn highlight_multiple_pairs_on_one_line() {
    let s = spans("==a== and ==b==");
    assert!(has(&s, 2, 3, MdKind::Highlight), "first pair 'a': {s:?}");
    assert!(has(&s, 12, 13, MdKind::Highlight), "second pair 'b': {s:?}");
    assert_eq!(
        s.iter().filter(|(_, k)| *k == MdKind::Highlight).count(),
        2,
        "exactly two highlight spans: {s:?}"
    );
}

#[test]
fn single_equals_never_matches() {
    // The whole motivation for choosing `==`: a bare `=` (prose like `x = y`,
    // or a single-equals assignment) must never be treated as a delimiter.
    let s = spans("if x = y then z");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Highlight),
        "a single '=' must never highlight: {s:?}"
    );
}

#[test]
fn unclosed_highlight_stays_literal() {
    // An opening `==` with no matching close: no span at all (not even a dim
    // Markup for the stray delimiter) — it just reads as plain `=` characters.
    let s = spans("==never closed");
    assert!(s.is_empty(), "an unclosed == must stay completely plain: {s:?}");
}

#[test]
fn adjacent_four_equals_is_inert() {
    // A run of exactly 4 `=` is ambiguous (not a valid isolated `==` pair at
    // any offset within it) and is left as plain literal text — no highlight,
    // no markup, matching a `===`/`====` divider-typo staying inert too.
    assert!(spans("before ==== after").is_empty(), "==== must not highlight");
    assert!(spans("a === b").is_empty(), "=== (odd run) must not highlight either");
}

#[test]
fn highlight_ignored_inside_inline_code() {
    // Inline code arrives via `Event::Code`, never `Event::Text`, so the
    // highlight scan structurally never sees it — `==x==` inside backticks
    // stays plain mono Code, no Highlight span.
    let s = spans("`==x==`");
    assert!(has(&s, 1, 6, MdKind::Code { inline: true }), "inner text is plain Code: {s:?}");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Highlight),
        "inline code must never highlight: {s:?}"
    );
}

#[test]
fn highlight_ignored_inside_fenced_code() {
    let s = spans("```\n==x==\n```");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Highlight),
        "a fenced code body must never highlight: {s:?}"
    );
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Code { inline: false }),
        "body is still Code: {s:?}"
    );
}

#[test]
fn highlight_no_cross_line_span_through_soft_wrap() {
    // A soft-wrapped paragraph ("==a" / newline / "b==") is ONE paragraph but
    // arrives as two `Text` events split at the break (pulldown emits a
    // `SoftBreak` between them, never embedding the `\n` in a `Text` range),
    // so neither half sees a complete pair — no highlight spans a line break.
    let s = spans("==a\nb==");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Highlight),
        "a highlight must never span a soft-wrapped line break: {s:?}"
    );
}

#[test]
fn highlight_no_cross_line_guard_fires_directly() {
    // A defensive unit test of the guard itself (pulldown's Text events don't
    // normally embed a raw '\n', so this constructs the case by hand): a
    // candidate pair separated by a newline is REJECTED, and the rejected
    // close is retried as a fresh open against the NEXT candidate.
    let mut out = Vec::new();
    let text = "==ab\ncd==ef==";
    push_highlight_spans(&mut out, text, &(0..text.len()));
    assert!(
        !out.iter().any(|(r, k)| *k == MdKind::Highlight && text[r.clone()].contains('\n')),
        "no highlight span may contain a newline: {out:?}"
    );
    assert!(
        has(&out, 9, 11, MdKind::Highlight),
        "the rejected close re-pairs with the next candidate ('ef'): {out:?}"
    );
}

#[test]
fn non_markdown_code_buffer_never_sees_highlight() {
    // `markdown::spans` is only ever CALLED on an `is_markdown` buffer (see
    // `render/text.rs::parse_doc_spans`'s `md_enabled` gate); a `.rs` file's
    // `a == b` comparison never reaches this module at all — the render-level
    // `markdown_highlight_inherits_wash_and_code_buffers_never_match` test in
    // `render/tests/washes.rs` pins that gate.
    // This is a belt-and-braces check on the function's OWN behavior: even
    // called directly on Rust-shaped text, a single comparison `==` (with no
    // SECOND `==` anywhere to pair with) can never highlight — an unpaired
    // marker is always the "unclosed" case, never a false-positive match.
    let s = spans("fn main() {\n    if a == b {}\n}\n");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Highlight),
        "a rust-shaped '==' comparison must never highlight: {s:?}"
    );
}

#[test]
fn open_task_marks_box_not_text() {
    // "- [ ] buy milk": '- ' is the list marker, '[ ] ' the open checkbox, and
    // the body text rides the DEFAULT ink (no span) so an open task stays present.
    let s = spans("- [ ] buy milk");
    assert!(has(&s, 0, 2, MdKind::ListMarker), "'- ' list marker: {s:?}");
    assert!(has(&s, 2, 6, MdKind::Task(false)), "'[ ] ' open checkbox: {s:?}");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::TaskDone),
        "an OPEN task must not dim its body: {s:?}"
    );
}

#[test]
fn checked_task_dims_box_and_text() {
    // "- [x] done thing": the checkbox is a CHECKED task marker and the body
    // text dims (TaskDone) so the whole line recedes like a struck todo.
    let s = spans("- [x] done thing");
    assert!(has(&s, 2, 6, MdKind::Task(true)), "'[x] ' checked checkbox: {s:?}");
    assert!(has(&s, 6, 16, MdKind::TaskDone), "checked body dims: {s:?}");
}

#[test]
fn task_done_does_not_leak_to_next_item() {
    // A checked item followed by an OPEN one: only the first item's body dims.
    let s = spans("- [x] closed\n- [ ] open");
    assert!(s.iter().any(|(_, k)| *k == MdKind::TaskDone), "first dims: {s:?}");
    assert_eq!(
        s.iter().filter(|(_, k)| *k == MdKind::TaskDone).count(),
        1,
        "the open sibling must NOT dim: {s:?}"
    );
}

#[test]
fn thematic_break_is_a_rule_span() {
    // A `---` alone on a line (blank lines around it) is a thematic break; the
    // Rule span covers the line (the renderer draws the rule quad over it).
    let s = spans("a\n\n---\n\nb");
    assert!(
        s.iter().any(|(r, k)| *k == MdKind::Rule && r.start == 3),
        "--- should yield a Rule span at byte 3: {s:?}"
    );
    // `***` and `___` are rules too.
    assert!(spans("\n***\n").iter().any(|(_, k)| *k == MdKind::Rule));
    assert!(spans("\n___\n").iter().any(|(_, k)| *k == MdKind::Rule));
}

#[test]
fn break_kind_tracks_the_syntax_and_maps_to_default_ornaments() {
    use crate::theme::ORNAMENTS_DEFAULT;
    // The three thematic-break spellings classify by their run character — incl.
    // the CommonMark 3+ / spaced / indented forms.
    assert_eq!(break_kind("---"), BreakKind::Dash);
    assert_eq!(break_kind("***"), BreakKind::Star);
    assert_eq!(break_kind("___"), BreakKind::Underscore);
    assert_eq!(break_kind("- - -"), BreakKind::Dash);
    assert_eq!(break_kind("  * * *"), BreakKind::Star);
    assert_eq!(break_kind("_____"), BreakKind::Underscore);
    // …and each default-world ornament is the expressive glyph for that syntax:
    // `---` → ❧ fleuron, `***` → ⁂ asterism (three stars), `___` → ❦ floral heart.
    assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Dash), '❧');
    assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Star), '⁂');
    assert_eq!(ORNAMENTS_DEFAULT.pick(BreakKind::Underscore), '❦');
}

/// THE FENCE-LANGUAGE-LABEL gate: [`fence_line_lang`] recognizes a fenced
/// block's opening line's info string EXACTLY when [`crate::syntax::Lang::
/// from_info`] would (the same gate `CodeSyntax` highlighting uses), so the
/// quiet render-only LABEL can never disagree with what the fence body actually
/// highlights as. A no-lang / unrecognized-lang / non-fence line yields `None`
/// (no label drawn).
#[test]
fn fence_line_lang_matches_the_syntax_highlighting_gate() {
    use crate::syntax::Lang;
    assert_eq!(fence_line_lang("```rust"), Some(Lang::Rust));
    assert_eq!(fence_line_lang("```python"), Some(Lang::Python));
    // Aliases / attribute tails resolve through the SAME `Lang::from_info` gate.
    assert_eq!(fence_line_lang("```rust,ignore"), Some(Lang::Rust));
    assert_eq!(fence_line_lang("```sh title=\"x\""), Some(Lang::Bash));
    // `~~~` fences work identically to backtick fences.
    assert_eq!(fence_line_lang("~~~rust"), Some(Lang::Rust));
    // Up to 3 leading indent spaces (CommonMark's fence-indent allowance).
    assert_eq!(fence_line_lang("   ```rust"), Some(Lang::Rust));
    // No language, an unrecognized language, or too little indent/run: no label.
    assert_eq!(fence_line_lang("```"), None, "bare fence: no label");
    assert_eq!(fence_line_lang("```made-up-lang"), None, "unrecognized language: no label");
    assert_eq!(fence_line_lang("not a fence"), None, "not a fence line at all: no label");
    assert_eq!(fence_line_lang("``rust"), None, "only 2 backticks is not a fence");
    assert_eq!(fence_line_lang("    ```rust"), None, "4-space indent is a CODE block, not a fence");

    // AGREEMENT LAW: for every recognized fence line, `fence_line_lang` agrees
    // with `Lang::from_info` on the raw info string alone — the label can never
    // name a language the body's own highlighting disagrees with.
    for (line, info) in [("```rust", "rust"), ("```python extra", "python extra"), ("~~~toml", "toml")] {
        assert_eq!(fence_line_lang(line), Lang::from_info(info), "label/highlight gate must agree: {line:?}");
    }
}

#[test]
fn setext_underline_is_not_a_rule() {
    // "Title\n---" is a setext H2 underline, NOT a thematic break — spans() must
    // not emit a Rule there (the heading is the authority, not the bare scan).
    let s = spans("Title\n---");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Rule),
        "a setext underline must not be a rule: {s:?}"
    );
}

#[test]
fn word_count_and_reading_time() {
    assert_eq!(word_count(""), 0);
    assert_eq!(word_count("   \n  "), 0);
    assert_eq!(word_count("one two three"), 3);
    assert_eq!(word_count("line one\nline two\n"), 4);
    // Reading time rounds UP and floors at 1 min for any prose; 0 for empty.
    assert_eq!(reading_time_min(0), 0);
    assert_eq!(reading_time_min(1), 1);
    assert_eq!(reading_time_min(READING_WPM), 1);
    assert_eq!(reading_time_min(READING_WPM + 1), 2);
    assert_eq!(reading_time_min(READING_WPM * 3), 3);
}

#[test]
fn tag_maps_deep_heading_levels() {
    // The sidecar wire tags for h4/h5/h6, plus the `_` catch-all that collapses
    // any level past 6 to "h6".
    assert_eq!(MdKind::Heading(4).tag(), "h4");
    assert_eq!(MdKind::Heading(5).tag(), "h5");
    assert_eq!(MdKind::Heading(6).tag(), "h6");
    assert_eq!(MdKind::Heading(9).tag(), "h6");
}

#[test]
fn heading_scale_has_three_sizes_then_flattens() {
    // The size ladder's SHAPE, asserted off the named rungs themselves (never
    // re-pinned literals — the rung VALUES are the consts' job, retuned by
    // taste rounds like Ladder J's 1.8/1.5/1.25 -> 1.6/1.3/1.15): each level
    // maps to its rung, the ladder descends strictly, and past ### the ramp
    // flattens to the subhead rung.
    assert_eq!(heading_scale(0), type_scale::BODY, "no hash => body size");
    assert_eq!(heading_scale(0), 1.0, "body rung is the 1.0 baseline");
    assert_eq!(heading_scale(1), type_scale::TITLE, "h1 => title");
    assert_eq!(heading_scale(2), type_scale::SECTION, "h2 => section");
    assert_eq!(heading_scale(3), type_scale::SUBHEAD, "h3 => subhead");
    // Strict ladder ordering, and 4+ hashes share the h3 (subhead) size.
    assert!(heading_scale(1) > heading_scale(2), "h1 > h2");
    assert!(heading_scale(2) > heading_scale(3), "h2 > h3");
    assert!(heading_scale(3) > 1.0, "h3 still bigger than body");
    assert_eq!(heading_scale(4), heading_scale(3), "4+ hashes == h3");
    assert_eq!(heading_scale(9), heading_scale(3), "deep counts clamp to h3");
    // The label rung sits BELOW body (for the future gutter/stats, faint ink).
    assert_eq!(type_scale::LABEL, 0.8, "label rung is 0.8");
    assert!(type_scale::LABEL < type_scale::BODY, "label reads smaller than body");
    // The ornament scale is PER-WORLD now (`theme::Theme::ornament_scale`), no
    // longer a single `type_scale` rung; its own tiers + row-coupling are asserted
    // in `theme::tests` and `render::tests` (see `every_world_has_an_ornament_scale`
    // + `md_line_scale_grows_thematic_break_rows_to_the_active_worlds_ornament_scale`).
    // The tallest ornate tier (2.2) still reads bigger than h1.
    assert!(
        crate::theme::ORNAMENT_SCALE_ORNATE > heading_scale(1),
        "the ornate ornament reads bigger than h1"
    );
}

#[test]
fn heading_weight_gate_title_never_bolds_and_force_overrides_only_the_bit() {
    use super::headings::heading_weight_bold_with_for_tests as gate;
    // No force (the shipping default): the world's bit decides, but ONLY for
    // level >= 2 — the TITLE (`#`) and a non-heading line (0) never bold.
    for level in 0u8..=9 {
        assert!(!gate(None, false, level), "bit off => never bold (level {level})");
    }
    assert!(!gate(None, true, 0), "level 0 (no heading) never bolds");
    assert!(!gate(None, true, 1), "TITLE never bolds, even with the bit set");
    for level in 2u8..=9 {
        assert!(gate(None, true, level), "bit on => level {level} bolds");
    }
    // The A/B gallery force replaces the BIT, never the level gate: `on` bolds
    // sections even on a bit-off world but STILL never the title; `off` kills
    // the bit everywhere.
    assert!(gate(Some(true), false, 2), "force on overrides a bit-off world at ##");
    assert!(!gate(Some(true), false, 1), "force on still never bolds the TITLE");
    assert!(!gate(Some(true), true, 1), "force on + bit on still never bolds the TITLE");
    assert!(!gate(Some(false), true, 2), "force off overrides a bit-on world");
    // And the public composition (env unset in the test process => no force)
    // agrees with the pure core's no-force arm.
    assert_eq!(
        crate::markdown::heading_weight_bold(true, 2),
        gate(None, true, 2),
        "public fn rides the same core (no env force set in tests)"
    );
}

#[test]
fn is_thematic_break_matches_commonmark_breaks_only() {
    // The three break syntaxes, bare and spaced/indented, all qualify.
    assert!(is_thematic_break("---"));
    assert!(is_thematic_break("***"));
    assert!(is_thematic_break("___"));
    assert!(is_thematic_break("- - -"), "spaced dashes are a break");
    assert!(is_thematic_break("   ---"), "up-to-3 indent still a break");
    assert!(is_thematic_break("*****"), "5 stars still a break");
    // NOT breaks: too few, mixed run chars, or any other content on the line.
    assert!(!is_thematic_break("--"), "two dashes is not a break");
    assert!(!is_thematic_break("-*-"), "mixed run chars are not a break");
    assert!(!is_thematic_break("- item"), "a list item is not a break");
    assert!(!is_thematic_break("# heading"), "a heading is not a break");
    assert!(!is_thematic_break("plain prose"), "prose is not a break");
    assert!(!is_thematic_break(""), "empty line is not a break");
}

// --- strikethrough (`~~struck~~`, the strikethrough-render round) ------------

#[test]
fn strikethrough_basic_pair_conceals_markers_and_marks_content() {
    let s = spans("~~struck~~");
    let st_markup = MdKind::ConcealMarkup(ConcealKind::Strikethrough);
    assert!(has(&s, 0, 2, st_markup), "opening ~~ dim + concealable: {s:?}");
    assert!(has(&s, 8, 10, st_markup), "closing ~~ dim + concealable: {s:?}");
    assert!(has(&s, 2, 8, MdKind::Strikethrough), "inner content struck: {s:?}");
}

#[test]
fn strikethrough_mid_sentence_pair() {
    let s = spans("keep ~~cut this~~ keep");
    assert!(has(&s, 7, 15, MdKind::Strikethrough), "inner run struck: {s:?}");
    assert_eq!(
        s.iter().filter(|(_, k)| *k == MdKind::Strikethrough).count(),
        1,
        "exactly one struck span: {s:?}"
    );
}

#[test]
fn single_tilde_never_strikes() {
    // pulldown's GFM option also accepts single-tilde `~x~`; awl deliberately
    // keeps it INERT (the `==` exactly-two precedent — the format command and
    // the writer's-diff serializer both speak `~~`). Prose like `2~3 weeks and
    // 4~5 days` must never be silently struck.
    let s = spans("2~3 weeks and 4~5 days");
    assert!(
        !s.iter().any(|(_, k)| matches!(
            k,
            MdKind::Strikethrough | MdKind::ConcealMarkup(ConcealKind::Strikethrough)
        )),
        "a single '~' must never strike: {s:?}"
    );
}

#[test]
fn strikethrough_ignored_inside_inline_code() {
    // Inline code is literal: `~~x~~` inside backticks stays plain mono Code.
    let s = spans("`~~x~~`");
    assert!(has(&s, 1, 6, MdKind::Code { inline: true }), "inner text is plain Code: {s:?}");
    assert!(
        !s.iter().any(|(_, k)| *k == MdKind::Strikethrough),
        "inline code must never strike: {s:?}"
    );
}

#[test]
fn strikethrough_ignored_inside_fenced_code() {
    let s = spans("```\n~~x~~\n```\n");
    assert!(
        !s.iter().any(|(_, k)| matches!(
            k,
            MdKind::Strikethrough | MdKind::ConcealMarkup(ConcealKind::Strikethrough)
        )),
        "a fence body must never strike: {s:?}"
    );
}

#[test]
fn strikethrough_is_additive_over_context_spans() {
    // Struck text inside a blockquote: the content carries BOTH the Quote
    // context span and (pushed AFTER, last-wins for ink) the Strikethrough span
    // — the `Highlight` lift precedent, but receding. The strike-line bucket
    // reads the Strikethrough span, so a struck quote still draws its line.
    let s = spans("> keep ~~cut~~\n");
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Strikethrough),
        "struck-inside-quote still emits Strikethrough: {s:?}"
    );
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Quote),
        "the quote context span is still present: {s:?}"
    );
}

#[test]
fn strikethrough_inside_diff_deletion_blockquote_line() {
    // Exactly the writer's-diff serializer's Deleted shape: `> ~~line~~` —
    // the transcript's struck deletions route through THIS parse (one strike
    // mechanism, no diff-only path).
    let s = spans("> ~~Drop this whole paragraph entirely.~~\n");
    assert!(
        s.iter().any(|(_, k)| *k == MdKind::Strikethrough),
        "the diff's deletion line parses as struck: {s:?}"
    );
    assert!(
        s.iter()
            .any(|(_, k)| *k == MdKind::ConcealMarkup(ConcealKind::Strikethrough)),
        "its ~~ markers conceal off-caret: {s:?}"
    );
}

#[test]
fn tilde_run_of_three_is_a_fence_not_a_strike() {
    // `~~~` opens a FENCED code block (the tilde-fence test elsewhere); it must
    // never read as strikethrough markers.
    let s = spans("~~~\nbody\n~~~\n");
    assert!(
        !s.iter().any(|(_, k)| matches!(
            k,
            MdKind::Strikethrough | MdKind::ConcealMarkup(ConcealKind::Strikethrough)
        )),
        "a tilde fence must never strike: {s:?}"
    );
    assert!(
        s.iter().any(|(_, k)| matches!(k, MdKind::Code { inline: false })),
        "the tilde fence still parses as a code block: {s:?}"
    );
}

#[test]
fn strikethrough_tag_strings_for_sidecar() {
    // The sidecar `md_spans` gains the new tag VALUES in the existing field —
    // a data change, not a shape change (no schema bump).
    assert_eq!(MdKind::Strikethrough.tag(), "strikethrough");
    assert_eq!(MdKind::ConcealMarkup(ConcealKind::Strikethrough).tag(), "markup");
    assert_eq!(ConcealKind::Strikethrough.tag(), "strikethrough");
}

// -----------------------------------------------------------------------
// Queue item 60: link/image DESTINATION exclusion (spell + writing-nits).
// -----------------------------------------------------------------------

#[test]
fn label_destination_range_isolates_just_the_parens_interior() {
    // A link's own conceal-tail shape (`push_link_markers`'s span starts
    // exactly at `](`).
    let link_tail = "](assets/pasted-18.png)";
    let r = label_destination_range(link_tail).expect("destination found");
    assert_eq!(&link_tail[r], "assets/pasted-18.png");

    // A whole image reference (`![alt](path)`) — the `](`-search skips past
    // the alt text automatically.
    let image_ref = "![](assets/pasted-18.png)";
    let r2 = label_destination_range(image_ref).expect("destination found");
    assert_eq!(&image_ref[r2], "assets/pasted-18.png");

    // Alt text with a size hint + a SPACE in the path — the whole parens
    // interior is returned regardless of what's inside it.
    let image_with_alt = "![a cat|300](assets/my photo.png)";
    let r3 = label_destination_range(image_with_alt).expect("destination found");
    assert_eq!(&image_with_alt[r3], "assets/my photo.png");

    // A fragment-only destination and an absolute URL both isolate cleanly.
    assert_eq!(
        &"](#a-fragment)"[label_destination_range("](#a-fragment)").unwrap()],
        "#a-fragment"
    );
    assert_eq!(
        &"](https://example.com/p?q=1)"[label_destination_range(
            "](https://example.com/p?q=1)"
        )
        .unwrap()],
        "https://example.com/p?q=1"
    );

    // Reference-style (`[text][ref]`) / malformed: no `](` at all — nothing
    // to exclude, mirroring `push_link_markers`'s own fallback.
    assert_eq!(label_destination_range("[text][ref]"), None);
    assert_eq!(label_destination_range("![alt]"), None);
}

#[test]
fn destination_ranges_excludes_addresses_but_never_label_or_alt_text() {
    // Item 60's exact motivating fixture (a raw pasted-image line) PLUS an
    // ordinary link, both carrying a misspelled LABEL/ALT word ("wrold")
    // that must stay eligible, alongside a misspelling-shaped word buried in
    // each destination that must NOT. The relative path carries a SPACE, which
    // CommonMark requires wrapping in `<...>` (an unescaped space in a bare
    // destination isn't a link at all — the angle-bracket form).
    let text = "See [wrold](<assets/relative wrold.png#frag>) and \
                ![wrold](https://example.com/wrold.png)\n";
    let s = spans(text);
    let dests = destination_ranges(text, &s);
    assert_eq!(dests.len(), 2, "one destination per reference: {dests:?}");

    let substrs: Vec<&str> = dests.iter().map(|r| &text[r.clone()]).collect();
    assert!(
        substrs.contains(&"<assets/relative wrold.png#frag>"),
        "relative path + space + fragment isolated whole: {substrs:?}"
    );
    assert!(
        substrs.contains(&"https://example.com/wrold.png"),
        "absolute URL isolated whole: {substrs:?}"
    );

    // Every "wrold" occurrence OUTSIDE a destination range (the two labels /
    // alt text) survives; only the ones INSIDE the two destinations above are
    // covered. Count total occurrences vs. how many a destination swallows.
    let total_wrold = text.matches("wrold").count();
    assert_eq!(total_wrold, 4, "sanity: two labels + two in-destination words");
    let covered = text
        .match_indices("wrold")
        .filter(|(i, w)| dests.iter().any(|r| r.start <= *i && i + w.len() <= r.end))
        .count();
    assert_eq!(covered, 2, "exactly the two IN-DESTINATION words are covered: {dests:?}");

    // A document with no link/image yields no destinations at all.
    let prose = "just some prose, no addresses here\n";
    assert!(destination_ranges(prose, &spans(prose)).is_empty());
}
