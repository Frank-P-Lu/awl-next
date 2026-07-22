//! TEXT / SHAPING SEAM — the `set_text` family + its supporting layout machinery:
//! the incremental-vs-full reshape decision, the per-line `AttrsList` assembly
//! (base doc attrs + markdown / syntax / CJK / heading-size layers), the IME
//! preedit composition, and the wrap-width / shape-height / heading-presence
//! queries that feed it.
//!
//! These are all inherent methods on [`super::TextPipeline`]: they shape into its
//! glyphon `GlyphBuffer` through its `FontSystem`, reading + mutating its line /
//! attrs / metrics state heavily, so they CANNOT become `&self`-free free functions
//! the way the pure span/attrs helpers in [`super::spans`] already are. This module
//! is purely a physical home for that cohesive shaping cluster, carved out of
//! `render.rs` verbatim. Because a child module sees its ancestor's private items,
//! the methods keep their full access to `TextPipeline`'s private fields/helpers and
//! to the `spans` / `geometry` free helpers with NO behaviour change — the shaped
//! glyphs are byte-identical.

use super::*;

/// Pre-resolved per-script `(family, weight)` faces for ONE reshape — see
/// [`TextPipeline::resolve_script_fonts`]. `None` for a script with NEITHER a
/// bundled nor an installed system candidate (the documented degenerate case:
/// no span is added for that script and shaping falls through to
/// cosmic-text's neutral platform fallback).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ScriptFonts {
    pub ja: Option<(&'static str, glyphon::Weight)>,
    pub zh_hans: Option<(&'static str, glyphon::Weight)>,
    pub zh_hant: Option<(&'static str, glyphon::Weight)>,
    pub ko: Option<(&'static str, glyphon::Weight)>,
}

impl ScriptFonts {
    /// The resolved face for `id`, or `None` when `id` is [`theme::FontId::Latin`]
    /// (never overridden — the base doc attrs already shape in it) or when that
    /// script's ladder resolved to nothing on this machine.
    pub(super) fn get(&self, id: theme::FontId) -> Option<(&'static str, glyphon::Weight)> {
        match id {
            theme::FontId::Latin => None,
            theme::FontId::Ja => self.ja,
            theme::FontId::ZhHans => self.zh_hans,
            theme::FontId::ZhHant => self.zh_hant,
            theme::FontId::Ko => self.ko,
        }
    }
}

impl TextPipeline {
    /// The document BASE family for the current buffer: the active world's
    /// [`Theme::mono`] when this is a CODE buffer (`self.syn_lang.is_some()` — a
    /// recognized `.rs`/`.py`/… file), else its proportional [`Theme::font`]. Code
    /// needs a true fixed grid the display serif/sans can't give, so a code buffer
    /// shapes in the world's monospace companion while prose / markdown / the
    /// no-path scratch buffer keep the world's display face. A world whose display
    /// face is already mono has `mono == font`, so those worlds are unaffected.
    /// Both are `&'static str`, so the borrowed `Family::Name` outlives the caller.
    pub(super) fn doc_family(&self) -> &'static str {
        if self.syn_lang.is_some() {
            theme::active().mono
        } else {
            theme::active().font
        }
    }

    /// The glyphon `Attrs` used to shape the DOCUMENT/body text: the ACTIVE
    /// world's display face, selected by its exact registered family name via
    /// `Family::Name`. This is the one knob that makes a theme switch reskin the
    /// GLYPH SHAPES — a mono world shapes in IBM Plex Mono, a serif world in
    /// Literata/Newsreader, etc. The chosen family is a registered embedded face
    /// (see FONT_THEME_FACES); any glyph it lacks falls back to the registered
    /// monospace (IBM Plex Mono) under Advanced shaping. The returned advances are
    /// real (proportional for the non-mono faces), and every horizontal call site
    /// (caret, hit-test, selection) reads those advances via `line_glyph_xs`, so
    /// the caret tracks each glyph's true advance on every world.
    ///
    /// A CODE buffer instead shapes in the world's monospace companion
    /// ([`Self::doc_family`] → [`Theme::mono`]) — the true fixed grid code wants —
    /// while prose / markdown keep the display face; both are `&'static str`, so the
    /// borrowed `Family::Name` outlives any caller's shaping call.
    pub(super) fn doc_attrs(&self) -> Attrs<'static> {
        // The font features are the THREE-WAY LIGATURE SPLIT (prose fi/fl vs code
        // programming ligatures vs discretionary-off), owned in ONE pure place so
        // this document path and the panel path (`render.rs` `panel_attrs`) can
        // never drift — see [`font_features`].
        let fam = self.doc_family();
        let ff = font_features(
            self.syn_lang.is_some(),
            fam,
            crate::render::code_ligatures_on(),
        );
        Attrs::new()
            .family(Family::Name(fam))
            .weight(mono_safe_weight(fam))
            .font_features(ff)
    }

    /// Resolve the ACTIVE world's CJK (Japanese) fallback face to a concrete
    /// `(family, weight)` the font DB actually has, or `None` if NEITHER a
    /// bundled nor a system candidate is installed (see `theme::CJK_MINCHO`/
    /// `CJK_GOTHIC`'s bundled-first priority order). Walks `theme::cjk` in
    /// order and returns the FIRST family present, paired with the registered
    /// weight of that family's face nearest 400. Since the Japanese-bundle
    /// round the FIRST candidate is always a bundled embedded face (Noto Serif
    /// JP / Noto Sans JP, registered in [`build_font_system`] — see
    /// [`FONT_CJK_FACES`]), so on every machine `resolve_cjk` deterministically
    /// resolves there UNLESS `AWL_CJK_FORCE=system` (the jp-compare dev knob)
    /// pruned it; only then does it fall to the trailing system Hiragino/Noto-
    /// CJK candidates.
    ///
    /// Returning the concrete weight is essential — see [`add_cjk_spans`]: naming
    /// the family at the default 400 would be dropped by cosmic-text's
    /// `weight_diff == 0` fallback filter (Hiragino has no Weight-400 face; the
    /// bundled Noto JP faces register exactly at 400, so they need no such
    /// correction). When this is `None`, the renderer adds no CJK span and
    /// Japanese falls through to cosmic-text's neutral platform fallback (the
    /// documented degenerate case — today only reachable via `AWL_CJK_FORCE` on
    /// a system with no Hiragino/Noto CJK, since the bundled faces are always
    /// registered in a normal run).
    pub(super) fn resolve_cjk(&self) -> Option<(&'static str, glyphon::Weight)> {
        self.resolve_font_id(theme::FontId::Ja)
    }

    /// THE font-ID resolver: walk the active world's [`theme::Theme::candidates`]
    /// ladder for `id` and return the FIRST family actually registered in the
    /// font DB, paired with its concrete registered weight nearest 400 (the
    /// same weight-trap correction [`Self::resolve_cjk`] always needed —
    /// system faces like Hiragino/PingFang don't register at a clean 400).
    /// `None` when NEITHER a bundled nor any system candidate is installed —
    /// the documented degenerate case: no span is added for that script and
    /// shaping falls through to cosmic-text's neutral platform fallback.
    /// [`theme::FontId::Latin`] is the one ID that can never return `None` in
    /// a normal run: its sole candidate is the world's own embedded display
    /// face, always registered (see `render::FONT_THEME_FACES`).
    pub(super) fn resolve_font_id(
        &self,
        id: theme::FontId,
    ) -> Option<(&'static str, glyphon::Weight)> {
        let db = self.font_system.db();
        for fam in theme::active().candidates(id) {
            let nearest = db
                .faces()
                .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(fam)))
                .map(|f| f.weight.0)
                .min_by_key(|w| (*w as i32 - 400).abs());
            if let Some(w) = nearest {
                return Some((fam, glyphon::Weight(w)));
            }
        }
        None
    }

    /// Pre-resolved per-script `(family, weight)` faces for ONE reshape —
    /// resolved ONCE (four small font-DB walks, the same cost class
    /// `resolve_cjk` always paid for one script) rather than per RUN, mirroring
    /// the existing "resolve once, apply per line" shape. `latin` has no
    /// entry: a Latin-classified run never needs an override span (the base
    /// doc attrs already shape in the world's own display face).
    pub(super) fn resolve_script_fonts(&self) -> ScriptFonts {
        ScriptFonts {
            ja: self.resolve_font_id(theme::FontId::Ja),
            zh_hans: self.resolve_font_id(theme::FontId::ZhHans),
            zh_hant: self.resolve_font_id(theme::FontId::ZhHant),
            ko: self.resolve_font_id(theme::FontId::Ko),
        }
    }

    /// [`Self::resolve_cjk`]'s family name plus whether it's a BUNDLED Noto
    /// Serif/Sans JP face (as opposed to a trailing system Hiragino/Noto-CJK
    /// candidate) — the capture sidecar's `font.cjk` block. Deterministic on
    /// every machine in a normal run: the bundled face is always registered
    /// and listed first (see `theme::CJK_MINCHO`/`CJK_GOTHIC`), so an agent can
    /// assert `bundled == true` for a JP fixture with NO dependency on which
    /// system CJK fonts happen to be installed — the first genuinely
    /// machine-independent JP-rendering assertion.
    pub fn cjk_report(&self) -> Option<(&'static str, bool)> {
        self.script_font_report(theme::FontId::Ja)
    }

    /// [`Self::cjk_report`]'s generalization: the resolved family + whether
    /// it's a bundled/embedded face, for ANY [`theme::FontId`] — the i18n
    /// round's sidecar `font.scripts` block (`capture/sidecar.rs`). `None`
    /// when that script's ladder resolved to nothing on this machine (the
    /// documented degenerate case; genuinely machine-dependent for zh/ko
    /// since v1 ships no bundled asset for them).
    pub fn script_font_report(&self, id: theme::FontId) -> Option<(&'static str, bool)> {
        self.resolve_font_id(id).map(|(family, _)| {
            let bundled = theme::EMBEDDED_CJK_FAMILIES.iter().any(|b| b.eq_ignore_ascii_case(family));
            (family, bundled)
        })
    }

    /// i18n: the document's OWN frontmatter `lang:` tag (`None` for an
    /// untagged or non-markdown document) — the sidecar's top-level `doc_lang`
    /// field. A pure function of the currently-shaped text, re-derived on
    /// every reshape (see [`Self::set_text_incremental`]).
    pub fn doc_lang_report(&self) -> Option<crate::frontmatter::Lang> {
        self.doc_lang
    }

    /// The document BYTE offset of buffer line `li`'s first byte (sum of the
    /// earlier lines' text lengths, each plus one for its `\n`). Maps the
    /// document-byte markdown/syntax spans into a single line's local byte range
    /// when rebuilding that line's `AttrsList` (the caret-driven conceal refresh +
    /// full restyle here), the `line_is_inline_image` image gate, and the capture
    /// reports (see `render/reports.rs`). O(li); the callers touch only a handful
    /// of lines, so this stays cheap.
    pub(super) fn line_doc_byte_start(&self, li: usize) -> usize {
        self.buffer
            .lines
            .iter()
            .take(li)
            .map(|l| l.text().len() + 1)
            .sum()
    }

    /// Re-apply the per-theme CJK family spans to EVERY buffer line in place.
    /// Used after a whole-buffer `Buffer::set_text` (which only carries the single
    /// Latin doc family) — the full-reshape path (`set_text_full`) and the live
    /// theme-switch reshape (`sync_theme`) — so CJK runs pick up the world's
    /// mincho/gothic face. No-op when [`Self::resolve_cjk`] is `None`. Must run
    /// BEFORE the following `shape_until_scroll`, since `set_attrs_list` resets a
    /// line's cached shaping.
    pub(super) fn apply_cjk_spans_all(&mut self) {
        let Some(cjk) = self.resolve_cjk() else { return };
        let attrs = self.doc_attrs();
        for line in self.buffer.lines.iter_mut() {
            let runs = cjk_runs(line.text());
            if runs.is_empty() {
                continue;
            }
            let mut al = glyphon::cosmic_text::AttrsList::new(&attrs);
            for run in runs {
                let a = attrs
                    .clone()
                    .family(Family::Name(cjk.0))
                    .weight(cjk.1);
                al.add_span(run, &a);
            }
            line.set_attrs_list(al);
        }
    }

    /// Replace document text and reshape. Active-theme display family + Advanced
    /// shaping: Advanced is required so cosmic-text performs font fallback for
    /// glyphs the theme face lacks (e.g. CJK -> a system Japanese face, or a glyph
    /// missing from a proportional face -> the mono default) AND so glyph advances
    /// are correct (full-width CJK cells are ~2x a Latin advance; proportional
    /// faces vary per glyph). All horizontal layout (caret, hit-test, selection) is
    /// then driven by the REAL shaped advances via [`Self::line_glyph_xs`], not a
    /// fixed CHAR_WIDTH — so the caret tracks each glyph on proportional worlds too.
    pub fn set_text(&mut self, text: &str) {
        self.reshape_count += 1;
        // Track the EFFECTIVE shaped face (mono for a code buffer, else the display
        // face) so a later theme switch reshapes iff that face actually changes.
        // `syn_lang` is set upstream (in `set_view`) before this runs.
        self.shaped_font = self.doc_family();
        // A full reshape bakes every per-span color under the active world; record
        // it so a later same-face theme switch can detect the palette change and
        // re-bake (see `shaped_theme` / `sync_theme_font`).
        self.shaped_theme = theme::active_index();
        self.set_text_incremental(text);
        // Grow the buffer's shaping HEIGHT so the WHOLE new document shapes (every
        // visual row appears in `layout_runs()`), which the visual-row scroll
        // count + overlay placement + hit-test all depend on. `set_size` may have
        // been called when the buffer still held placeholder text (so its height
        // budget was for the wrong line count); recompute it here against the text
        // we just set. Width (wrap) is preserved. cosmic-text no-ops if unchanged.
        // Wrap at the PAGE-MODE column width (recomputed from the current zoom /
        // measure), not the buffer's stale size — a zoom or measure change alters
        // the column, so re-feeding the old width would keep the wrong wrap.
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The shaped geometry just changed: the cached total-visual-row count is
        // stale. Recomputed lazily on the next `total_visual_rows` read.
        self.row_geom.invalidate();
    }

    /// BEFORE-style whole-buffer reshape: the original code path that called
    /// cosmic-text's `Buffer::set_text` (which clears + rebuilds EVERY line,
    /// discarding all per-line shaping caches and forcing a whole-document Advanced
    /// reshape). Retained ONLY so the typing micro-benchmark can measure the old
    /// O(document) cost against the new incremental path on the same pipeline; the
    /// live editor never calls this.
    pub fn set_text_full(&mut self, text: &str) {
        self.reshape_count += 1;
        let attrs = self.doc_attrs();
        self.buffer.set_text(
            &mut self.font_system,
            text,
            &attrs,
            Shaping::Advanced,
            None,
        );
        // `Buffer::set_text` shaped every line in the single Latin doc family;
        // overlay the per-theme CJK family spans so Japanese resolves to the
        // world's mincho/gothic face (before the shape below re-lays the lines).
        self.apply_cjk_spans_all();
        // Wrap at the PAGE-MODE column width (recomputed from the current zoom /
        // measure), not the buffer's stale size — a zoom or measure change alters
        // the column, so re-feeding the old width would keep the wrong wrap.
        let width = Some(self.text_wrap_width());
        let shape_h = self.full_shape_height();
        self.buffer
            .set_size(&mut self.font_system, width, Some(shape_h));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.row_geom.invalidate();
        self.shaped_key = Some(text.to_string());
    }

    /// Reconcile the glyphon buffer's per-line text with `text`, mutating ONLY the
    /// `BufferLine`s that actually differ so cosmic-text reuses cached per-line
    /// shaping for every UNCHANGED line. This is the core of the typing fix:
    /// `Buffer::set_text` clears + rebuilds every line (discarding all shaping
    /// caches, forcing a whole-document Advanced reshape), whereas here a single
    /// typed character invalidates exactly one `BufferLine`, so the next
    /// `shape_until_scroll` re-shapes just that line and the rest stay cached.
    ///
    /// Line splits/joins (newline insert / backspace-merge) shift only the lines
    /// at and after the edit; we splice the glyphon `lines` vector to match the
    /// new line list and let `BufferLine::set_text` no-op (return `false`, keeping
    /// the cache) for any line whose text is byte-identical after the shift. So a
    /// newline in a huge document still only reshapes the two touched lines, not
    /// the thousands of identical lines below it.
    /// Parse the WHOLE document `text` into its base styling-span layers in document
    /// byte coords: the MARKDOWN spans (gated to markdown buffers) and the SYNTAX
    /// role spans (gated to recognized CODE buffers). Markdown + syntax are mutually
    /// exclusive, so at most one of the two lists is ever non-empty; a non-styled
    /// buffer yields two empty lists, which makes the per-line attrs pass a no-op so
    /// the render stays byte-identical. Computed from the shaped text (preedit-spliced
    /// and all), so the span byte offsets line up with the buffer lines.
    #[allow(clippy::type_complexity)]
    fn parse_doc_spans(
        &self,
        text: &str,
    ) -> (
        Vec<(std::ops::Range<usize>, crate::markdown::MdKind)>,
        Vec<(std::ops::Range<usize>, crate::syntax::SynKind)>,
    ) {
        let md_spans = if self.md_enabled {
            crate::markdown::spans(text)
        } else {
            Vec::new()
        };
        let syn_spans = match self.syn_lang {
            Some(lang) => crate::syntax::spans(lang, text),
            None => Vec::new(),
        };
        (md_spans, syn_spans)
    }

    /// INLINE IMAGES: resolve a doc-relative image path against the open
    /// document's directory ([`Self::image_base_dir`], set from
    /// [`ViewState::doc_dir`]). An ABSOLUTE path is used verbatim; a relative one
    /// with no base dir (a scratch/no-path buffer) resolves against the cwd.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn resolve_image_path(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        match &self.image_base_dir {
            Some(d) => d.join(p),
            None => p.to_path_buf(),
        }
    }

    /// True when logical line `li` is an inline-image reference (`![alt](path)`) —
    /// i.e. it carries a `ConcealMarkup(Image)` span. The ONE owner of "is this an
    /// image line" for the caret-scale + focus-recolor paths (the pure
    /// [`super::spans::line_has_image_span`] over `self.md_spans`), so a WRAPPED
    /// TABLE row — which also reserves an `image_heights` slot but follows the pure
    /// heading model — is never mistaken for one. `false` for every line when
    /// there are no md spans (non-markdown / images-off).
    pub(super) fn line_is_inline_image(&self, li: usize) -> bool {
        if self.md_spans.is_empty() {
            return false;
        }
        let start = self.line_doc_byte_start(li);
        let end = start + self.buffer.lines.get(li).map_or(0, |l| l.text().len());
        super::spans::line_has_image_span(&self.md_spans, start, end)
    }

    /// INLINE IMAGES: compute the per-LOGICAL-LINE image display HEIGHT table (the
    /// value [`build_line_attrs`] uses to reserve a tall row) AND refill the
    /// deterministic [`Self::image_report`] (the sidecar + next-phase draw source).
    /// Reads each `ConcealMarkup(Image)` md_span's `![alt](path)` source with
    /// [`crate::markdown::parse_image_source`], then reads ONLY the image file's
    /// header dimensions (`into_dimensions` — no full decode) to fit-to-column,
    /// VIEWPORT-CAPPED (never past [`super::spans::IMAGE_MAX_VIEWPORT_FRAC`] of the
    /// window height) via the pure [`super::spans::image_display_size`]. A
    /// missing/unreadable file reserves a placeholder-height row (the placeholder
    /// GLYPH is the next phase).
    /// Returns an all-`None` table when the feature is off / not markdown / on wasm,
    /// so the render stays byte-identical (no tall row is ever reserved). ALSO
    /// populates [`Self::image_force`] (item 5 rework — see its field doc) for
    /// MIXED off-cursor lines; that table is a separate, PARALLEL mechanism, never
    /// a value in this returned `heights` table (a mixed line's OWN row is never
    /// inflated any more — see the field doc for why).
    ///
    /// `selection_touch` (SELECTION REVEAL regression fix, item 16 follow-up) is
    /// the caller's already-computed [`super::spans::selection_touch_bytes`]
    /// extent: each image's `revealed_now` is true when the caret's line OR the
    /// active selection touches its span, via [`super::spans::selection_touches`]
    /// — the SAME test [`super::spans::wysiwyg_reveals`] uses for the raw markup
    /// itself — so a selected image line PARKS (dims/skips the tall reservation)
    /// exactly like a caret-revealed one, never a bright image under revealed
    /// source text.
    fn compute_image_layout(
        &mut self,
        text: &str,
        md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
        selection_touch: Option<&std::ops::Range<usize>>,
    ) -> Vec<Option<f32>> {
        let mut report = self.image_report.borrow_mut();
        report.clear();
        let line_count = text.split('\n').count().max(1);
        #[allow(unused_mut)]
        let mut heights = vec![None; line_count];
        #[allow(unused_mut)]
        let mut force: Vec<Option<(f32, f32)>> = vec![None; line_count];
        #[cfg(not(target_arch = "wasm32"))]
        if crate::markdown::inline_images_on() && self.md_enabled {
            use crate::markdown::{ConcealKind, MdKind};
            let wrap = self.text_wrap_width();
            let base_fs = self.metrics.font_size;
            let base_lh = self.metrics.line_height;
            let cursor_line = self.cursor_line;
            // The REAL document family/weight (see `Self::doc_attrs`'s doc) — the
            // forcing measurement below MUST shape the prefix in the SAME face the
            // caption itself renders in (a mono/serif/sans world's own display
            // face), or it under/over-measures against a generic fallback and the
            // forcing glyph lands at the WRONG remaining-space estimate (confirmed
            // empirically: a plain `Attrs::new()` measurement under-measured a real
            // theme's wider face, so the forcing glyph fired while the caption's
            // OWN natural wrap still had a row left, stranding the image mid-text
            // again — the very bug this round fixed).
            let doc_attrs = self.doc_attrs();
            // Collect the pure per-image facts (no `&mut self` needed) FIRST — the
            // forcing measurement below needs `&mut self.font_system`, so it can't
            // run interleaved with a borrow of `md_spans`/`text` inside the SAME
            // loop as a method call on `&mut self` (the existing `self.resolve_
            // image_path`/`self.image_preview` reads here are all `&self`, fine).
            struct Found {
                r: std::ops::Range<usize>,
                img: crate::markdown::ImageRef,
                line: usize,
                dw: f32,
                dh: f32,
                missing: bool,
                mixed: bool,
                revealed_now: bool,
                prefix: String,
            }
            let mut found = Vec::new();
            for (r, k) in md_spans {
                if !matches!(k, MdKind::ConcealMarkup(ConcealKind::Image)) {
                    continue;
                }
                let Some(img) = text
                    .get(r.clone())
                    .and_then(crate::markdown::parse_image_source)
                else {
                    continue;
                };
                let line = text[..r.start].bytes().filter(|&b| b == b'\n').count();
                let resolved = self.resolve_image_path(&img.path);
                let dims = image::ImageReader::open(&resolved)
                    .ok()
                    .and_then(|rd| rd.with_guessed_format().ok())
                    .and_then(|rd| rd.into_dimensions().ok());
                // DRAG-RESIZE live preview: while THIS image is being dragged, its
                // fit-to-column width is overridden by the live preview width (the
                // buffer's `|NNN` hint is only written on release), re-fitting the
                // height from the intrinsic aspect. Feeding the preview width as the
                // effective `width_hint` reuses `image_display_size`'s exact
                // fit/clamp math, so a preview looks byte-identical to the committed
                // hint it will become.
                let effective_hint = match self.image_preview {
                    Some((ps, pe, pw)) if ps == r.start && pe == r.end => {
                        Some(pw.round().max(1.0) as u32)
                    }
                    _ => img.width_hint,
                };
                let max_h = self.window_h * super::spans::IMAGE_MAX_VIEWPORT_FRAC;
                let (dw, dh, missing) = match dims {
                    Some((w, h)) => {
                        let (dw, dh) =
                            super::spans::image_display_size(w, h, effective_hint, wrap, max_h);
                        (dw, dh, false)
                    }
                    None => (
                        wrap.max(1.0),
                        base_lh * super::spans::IMAGE_MISSING_ROW_LINES,
                        true,
                    ),
                };
                // ITEM 5 REWORK — "list item with text AND an image": a BARE image
                // line (`- ![alt](p)`, list marker aside) reserves exactly `dh` —
                // the image IS the row, unchanged since images-v1 (`heights[line]`
                // below). A MIXED line (`- caption text ![alt](p)`) does NOT touch
                // `heights[line]` at all any more — see [`Self::image_force`]'s
                // field doc for the forced-trailing-row mechanism that replaces the
                // prior round's `base_lh + 2*dh` whole-row inflation (which centred
                // the caption away from its own marker — the reported bug).
                let line_start = text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_end = text[r.end..].find('\n').map(|i| r.end + i).unwrap_or(text.len());
                let local = (r.start - line_start)..(r.end - line_start);
                let mixed =
                    super::spans::image_line_has_other_content(&text[line_start..line_end], local);
                // SELECTION REVEAL (regression fix, item 16 follow-up): an
                // image line PARKS (see below) when the caret is on it OR the
                // active selection touches its own span `r` â the SAME overlap
                // test `wysiwyg_reveals` applies to this exact span for the raw-
                // markup conceal decision (`super::spans::selection_touches`,
                // never re-derived), so a selected image line's layout/draw
                // state can't disagree with its own revealed markup.
                let revealed_now =
                    line == cursor_line || super::spans::selection_touches(selection_touch, r);
                if mixed {
                    // REVEALED MIXED LINE (caret on it): reserves nothing at all —
                    // the ordinary un-scaled row model, exactly like plain prose —
                    // and the draw side parks the image for that one frame. Same
                    // "reflow while actively editing is accepted" trade already
                    // priced into fence/rule conceal elsewhere (docs/markdown.md).
                    if !revealed_now {
                        found.push(Found {
                            r: r.clone(),
                            img: img.clone(),
                            line,
                            dw,
                            dh,
                            missing,
                            mixed: true,
                            revealed_now,
                            prefix: text[line_start..r.start].to_string(),
                        });
                        continue;
                    }
                } else if let Some(slot) = heights.get_mut(line) {
                    *slot = Some(dh);
                }
                found.push(Found {
                    r: r.clone(),
                    img: img.clone(),
                    line,
                    dw,
                    dh,
                    missing,
                    mixed,
                    revealed_now,
                    prefix: String::new(),
                });
            }
            for f in found {
                if f.mixed && !f.revealed_now {
                    // The forcing span's target advance: enough to overflow the
                    // marker+caption's own LAST wrapped row (so it lands on a
                    // fresh trailing row of THIS line, never the caption's own —
                    // see `Self::image_force`'s field doc), plus a small safety
                    // margin so float/measurement drift can't leave it short. A
                    // caption that already wraps on its own is handled too: we
                    // measure the LAST row of ITS OWN natural wrap, not the whole
                    // (unwrapped) text.
                    let last_row_w = Self::measure_last_row_width(
                        &mut self.font_system,
                        &f.prefix,
                        &doc_attrs,
                        base_fs,
                        wrap,
                    );
                    let remaining = (wrap - last_row_w).max(0.0);
                    let target_advance = remaining + Self::IMAGE_FORCE_MARGIN_PX;
                    if let Some(slot) = force.get_mut(f.line) {
                        *slot = Some((f.dh, target_advance));
                    }
                }
                report.push(crate::render::ImageReport {
                    range: (f.r.start, f.r.end),
                    line: f.line,
                    path: f.img.path,
                    alt: f.img.alt,
                    width_hint: f.img.width_hint,
                    display_w: f.dw,
                    display_h: f.dh,
                    missing: f.missing,
                    revealed: f.revealed_now,
                });
            }
        }
        let _ = md_spans;
        let _ = selection_touch;
        self.image_force = force;
        heights
    }

    /// ITEM 5 REWORK: the safety margin (px) added on top of the measured
    /// remaining space when sizing a mixed image line's forcing `letter_spacing`
    /// (see [`Self::image_force`]'s field doc) — absorbs float rounding and the
    /// small residual inaccuracy of measuring the prefix in ISOLATION (matched
    /// family/weight via `doc_attrs`, but not per-span CJK/markdown overrides the
    /// real line may also carry). Small relative to any real wrap column, so it
    /// never itself risks overflowing a fresh row.
    const IMAGE_FORCE_MARGIN_PX: f32 = 4.0;

    /// ITEM 5 REWORK: the rendered pixel width of `text`'s own LAST visual row when
    /// word-wrapped at `wrap_width` — i.e. how much of a fresh `wrap_width`-wide row
    /// is already used by `text` (a mixed image line's marker+caption PREFIX, up to
    /// but not including the image markup) if it were laid out alone. Shapes `text`
    /// in an ISOLATED, throwaway `Buffer`, in the SAME `attrs` (family/weight — see
    /// the call site's doc: MUST match the real document face, or this under/over-
    /// measures) at `Wrap::WordOrGlyph` and the SAME wrap width the real line uses,
    /// so a caption that wraps on its own is measured correctly too (the LAST run's
    /// width, not the whole unwrapped sum). `0.0` for empty text. A one-line-shape
    /// cost, paid once per mixed image line per RESHAPE (not per frame) —
    /// comparable to `compute_image_layout`'s existing per-image disk read.
    fn measure_last_row_width(
        font_system: &mut FontSystem,
        text: &str,
        attrs: &Attrs<'static>,
        font_size: f32,
        wrap_width: f32,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let mut buf = GlyphBuffer::new_empty(GlyphMetrics::new(font_size, font_size * 1.2));
        buf.set_wrap(font_system, Wrap::WordOrGlyph);
        buf.set_size(font_system, Some(wrap_width.max(1.0)), None);
        let al = glyphon::cosmic_text::AttrsList::new(attrs);
        buf.lines.push(glyphon::cosmic_text::BufferLine::new(
            text,
            glyphon::cosmic_text::LineEnding::None,
            al,
            Shaping::Advanced,
        ));
        buf.shape_until_scroll(font_system, false);
        buf.layout_runs().last().map(|r| r.line_w).unwrap_or(0.0)
    }

    /// ITEM 5 REWORK: the absolute screen y (px) at which THIS line's image quad
    /// draws/hit-tests. A BARE image line (or any line without a current
    /// [`Self::image_force`] entry) is byte-identical to before: the row top
    /// ([`Self::line_ornament_top`], offset `0.0` — the image IS the row). A MIXED
    /// OFF-CURSOR line instead reads its LAST visual row's own top — where the
    /// forcing glyph actually landed (real cosmic-text layout, see
    /// [`Self::image_force`]'s field doc) — so the quad sits directly below the
    /// (untouched, base-height) marker+caption row, never overlapping it, with NO
    /// separate offset arithmetic to keep in sync with the layout.
    pub(super) fn image_draw_top(&self, line: usize) -> f32 {
        if self.image_force.get(line).copied().flatten().is_some() {
            let rows = self.visual_rows(line);
            if let Some(last) = rows.last() {
                return self.doc_top() + last.line_top;
            }
        }
        self.line_ornament_top(line)
    }

    /// ITEM 5 REWORK: true when logical `line` currently has a well-defined image
    /// draw position — a BARE line's `image_heights` reservation OR a MIXED
    /// off-cursor line's [`Self::image_force`] entry. `false` for a REVEALED MIXED
    /// line (both tables are `None` while the caret is on it — see
    /// `compute_image_layout`'s doc comment), the caller's
    /// (`layers::prepare_images` / `Self::image_hit_rects`) signal to skip
    /// drawing/arming the image for that one frame rather than draw it at an
    /// undefined position.
    pub(super) fn image_row_reserved(&self, line: usize) -> bool {
        self.image_heights.get(line).copied().flatten().is_some()
            || self.image_force.get(line).copied().flatten().is_some()
    }

    /// ITEM 5c (theme-switch slowdown probe): the running count of actual image
    /// DECODES this pipeline has performed (never a cache hit — see
    /// `image_cache::ImageCache::decode_count`'s doc). A theme switch
    /// (`sync_theme`/`sync_theme_colors`/`sync_theme_font`) never touches the
    /// decode cache, so this stays flat across repeated switches with the same
    /// images on screen; the render/tests witness asserts exactly that. Test-only.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn image_decode_count(&self) -> usize {
        self.image_cache.decode_count()
    }

    /// INLINE-IMAGE DRAG-RESIZE (v2, live app only): set (or clear with `None`) the
    /// live-preview width override — `(byte_start, byte_end, display_w)` keyed by the
    /// dragged image's `![alt](path)` document byte range. Marks the override dirty so
    /// the next `set_view` forces the reshape that re-runs `compute_image_layout` and
    /// re-fits that image live (the buffer is untouched until the drag's release
    /// write-back). A no-op when the override is unchanged, so a redundant set costs
    /// nothing. Never reached headlessly (no MouseInput in a capture).
    pub fn set_image_preview(&mut self, preview: Option<(usize, usize, f32)>) {
        if self.image_preview != preview {
            self.image_preview = preview;
            self.image_preview_dirty = true;
        }
    }

    /// INLINE-IMAGE DRAG-RESIZE (v2, live app only): the on-screen resize-HANDLE
    /// targets — for each DRAWN image (visible, not a missing placeholder) its
    /// document byte `range` + its on-screen rect `[left, top, w, h]`, computed
    /// with the IDENTICAL geometry `prepare_images` draws at (centered in the
    /// writing column, reserved tall row). The app loops these through the pure
    /// `geometry::image_handle_hit` to decide an edge/corner grab — no parallel
    /// geometry. Reads through [`Self::images_report`] (NOT the stored
    /// `image_report` directly) so `im.revealed` is the ALWAYS-FRESH override, not
    /// the last reshape's stored flag — otherwise a pure caret/selection move onto
    /// an off-cursor MIXED image line (no reshape, so `compute_image_layout` never
    /// re-runs) would leave a stale `revealed: false`, and the skip below would arm
    /// a handle for a line that no longer reserves a draw position (item 27).
    /// Empty when the feature is off / no drawn images.
    ///
    /// REVEALED images ARM TOO (no blanket `im.revealed` exclusion): the caption
    /// model (`df773ba`) draws the image on EVERY line now — caret-on-line only
    /// floats the raw source text as a caption overlay, it no longer hides the
    /// drawn BARE image — so the resize handles at its edges/corners stay live
    /// regardless of caret position. The caption text sits CENTERED mid-image while
    /// the handles live at the edges/corners, so the two affordances never overlap.
    /// A REVEALED MIXED line is the one exception (it reserves nothing this frame —
    /// see the skip below), so its handle correctly drops out.
    pub fn image_hit_rects(&self) -> Vec<((usize, usize), [f32; 4])> {
        let report = self.images_report();
        let text_left = self.text_left();
        let wrap = self.text_wrap_width();
        let mut out = Vec::new();
        for im in report.iter() {
            // ITEM 5 REWORK: a REVEALED MIXED line has no reservation this frame
            // (`compute_image_layout`'s doc comment) — no well-defined position
            // to arm a handle at, so it's skipped like `im.missing`.
            if im.missing
                || !self.line_ornament_visible(im.line)
                || (im.revealed && !self.image_row_reserved(im.line))
            {
                continue;
            }
            let dw = im.display_w.max(1.0);
            let dh = im.display_h.max(1.0);
            let top = self.image_draw_top(im.line);
            let left = text_left + (wrap - dw).max(0.0) * 0.5;
            out.push((im.range, [left, top, dw, dh]));
        }
        out
    }

    /// INLINE-IMAGE DRAG-RESIZE (v2, live app only): if `(pointer_x, pointer_y)` is
    /// over a DRAWN image's resize EDGE/CORNER, the hit image's document byte `range`,
    /// the grabbed [`ImageHandle`](super::geometry::ImageHandle), and its PRESS-TIME
    /// on-screen `rect` `[left, top, w, h]` (the anchors the drag math reads).
    /// Encapsulates [`Self::image_hit_rects`] + the pure
    /// [`super::geometry::image_handle_hit`] so no raw geometry leaks to the app — the
    /// same shape as `page_resize_hover`. `None` when the pointer is over no border /
    /// the feature is off.
    pub fn image_handle_at(
        &self,
        pointer_x: f32,
        pointer_y: f32,
    ) -> Option<((usize, usize), super::geometry::ImageHandle, [f32; 4])> {
        let tol = super::geometry::IMAGE_RESIZE_GRAB_PX;
        let pointer = (pointer_x, pointer_y);
        self.image_hit_rects().into_iter().find_map(|(range, rect)| {
            super::geometry::image_handle_hit(pointer, rect, tol).map(|handle| (range, handle, rect))
        })
    }

    pub(super) fn set_text_incremental(&mut self, text: &str) {
        let attrs = self.doc_attrs();
        // Resolve every per-script fallback face ONCE (depends on the active
        // theme + font DB, not the per-line text), then overlay the resolved
        // face on each changed line below via the per-run script ladder
        // (`build_line_attrs` -> `add_script_spans`).
        let fonts = self.resolve_script_fonts();
        // i18n: the document's OWN frontmatter `lang:` tag, re-derived here
        // (the one place fresh `text` is in hand) and cached in `self.doc_lang`
        // for the caret-driven passes below (`restyle_all_lines` /
        // `refresh_rule_conceal`) that only ever run on UNCHANGED text, so the
        // cached value is always current for them.
        self.doc_lang = crate::frontmatter::detect(text).and_then(|fm| fm.lang);
        // Parse the whole document into its markdown + syntax styling spans (both in
        // document byte coords, gated per buffer kind). Pulled into [`parse_doc_spans`]
        // so this stays the diff/splice orchestrator; an empty list makes the per-line
        // pass below a byte-identical no-op.
        let (md_spans, syn_spans) = self.parse_doc_spans(text);
        // Split into lines WITHOUT the line terminators (cosmic-text stores the
        // ending separately). `str::lines()` drops a single trailing newline, which
        // matches cosmic-text's "trailing empty line" handling: we re-add an empty
        // final line below so an end-of-buffer caret has a line to sit on. Computed
        // BEFORE `compute_image_layout` below (reordered this round) so its
        // selection-touch extent can feed the image reveal decision.
        let new_lines: Vec<&str> = text.split('\n').collect();
        // Prefix-sum each line's FIRST byte offset in the document (each line is
        // its text + one `\n`), so the markdown span pass can map a document-byte
        // span into a line's local byte range.
        let mut line_starts: Vec<usize> = Vec::with_capacity(new_lines.len());
        let mut acc = 0usize;
        for l in &new_lines {
            line_starts.push(acc);
            acc += l.len() + 1;
        }
        // REVEAL-ON-CURSOR: a markdown horizontal-rule line conceals its raw `---`
        // (transparent ink, fleuron alone) UNLESS the caret is on it, in which case
        // the dashes reveal for editing. `conceal_rule` is keyed off the line index
        // vs `self.cursor_line` (read here so the closure stays a plain capture).
        let cursor_line = self.cursor_line;
        // WYSIWYG fence conceal is BLOCK-scoped: it needs the caret's own line's
        // first document byte (not just its line index) to test containment in a
        // fenced block's whole byte range. `line_starts` is already built above.
        let cursor_byte = line_starts.get(cursor_line).copied().unwrap_or(0);
        // SELECTION REVEAL: the byte extent of every line the active selection
        // touches (`None` with no selection), computed ONCE from the freshly-
        // diffed `line_starts`/`new_lines` (not `self.buffer.lines`, which is
        // still the STALE pre-edit text at this point in the splice) — see
        // `selection_touch_bytes`'s own doc comment for why this is the ONE
        // owner every reveal decision below reads (now including
        // `compute_image_layout`'s inline-image reveal, item 16 follow-up).
        let selection_touch = selection_touch_bytes(
            self.selection,
            |i| line_starts.get(i).copied().unwrap_or(0),
            |i| new_lines.get(i).map_or(0, |l| l.len()),
        );
        // INLINE IMAGES: per-line reserved display heights (+ the sidecar/draw
        // report), read from the just-parsed `ConcealMarkup(Image)` spans and each
        // image's header dimensions. All-`None` (no tall rows) when the feature is
        // off / non-markdown / wasm, so the render below stays byte-identical.
        let mut image_heights = self.compute_image_layout(text, &md_spans, selection_touch.as_ref());
        // ITEM 5 REWORK: the forced-trailing-row table `compute_image_layout` just
        // populated on `self` (see `Self::image_force`'s field doc) — pulled out to
        // a local so the `line_attrs` closure below can capture it without also
        // entangling a borrow of `self` (which the loop that calls it mutates
        // elsewhere, e.g. `self.buffer.lines[..]`). A small per-line `Vec`, cloned
        // once per reshape — the same cost class as `image_heights` itself.
        let image_force = self.image_force.clone();
        // WRAP-NOT-CLIP TABLES: a too-wide GFM table wraps its cells and each grown
        // row RESERVES a tall document row here (the SAME `image_heights` slot the
        // images use, since a line is never both an image ref and a table row), so
        // the off-cursor grid never overlaps the following content. `None` for every
        // line that isn't a wrapped table row → byte-identical for a fitting table
        // and every non-table doc.
        {
            let table_heights = self.compute_table_layout(text, &md_spans);
            for (li, th) in table_heights.iter().enumerate() {
                if let (Some(h), Some(slot)) = (th, image_heights.get_mut(li)) {
                    if slot.is_none() {
                        *slot = Some(*h);
                    }
                }
            }
        }
        // Build a per-line attrs list = base doc attrs + MARKDOWN spans + CJK
        // family spans (CJK family wins on CJK runs; markdown weight/color/style
        // win elsewhere). `start` is the line's document byte offset. A HEADING
        // line scales its base metrics (bigger font + taller row) via
        // [`scaled_base_attrs`]; every span on that line is built from the scaled
        // base so the glyphs grow with the row. Non-heading lines get scale 1.0,
        // i.e. the byte-identical plain base.
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let doc_lang = self.doc_lang;
        let cjk_priority = &self.cjk_priority;
        let line_attrs = |lt: &str, start: usize, li: usize| {
            let conceal_off_cursor = li != cursor_line;
            build_line_attrs(
                &attrs, base_fs, base_lh, md, lt, start, &md_spans, &syn_spans, doc_lang,
                cjk_priority, &fonts, conceal_off_cursor, cursor_byte,
                image_heights.get(li).copied().flatten(),
                image_force.get(li).copied().flatten(),
                selection_touch.as_ref(),
            )
        };
        // `split('\n')` on "a\n" yields ["a", ""] — exactly the trailing-empty-line
        // shape cosmic-text wants. On "" it yields [""], one empty line. Good.

        // Diff against the live buffer to find the changed middle band.
        let (prefix, old_end, new_end) = self.unchanged_band(&new_lines);

        // Rebuild changed lines, reusing existing BufferLine slots where they line
        // up so an in-place edit (same line count) only resets the edited line.
        let mut replacement: Vec<glyphon::cosmic_text::BufferLine> =
            Vec::with_capacity(new_end - prefix);
        for (k, &lt) in new_lines[prefix..new_end].iter().enumerate() {
            let old_idx = prefix + k;
            if old_idx < old_end {
                // Reuse the slot: `set_text` no-ops (keeps cache) if text unchanged,
                // else resets just this line's shaping.
                let mut line = std::mem::replace(
                    &mut self.buffer.lines[old_idx],
                    glyphon::cosmic_text::BufferLine::new(
                        "",
                        glyphon::cosmic_text::LineEnding::None,
                        glyphon::cosmic_text::AttrsList::new(&attrs),
                        Shaping::Advanced,
                    ),
                );
                line.set_text(
                    lt,
                    glyphon::cosmic_text::LineEnding::Lf,
                    line_attrs(lt, line_starts[old_idx], old_idx),
                );
                replacement.push(line);
            } else {
                replacement.push(glyphon::cosmic_text::BufferLine::new(
                    lt,
                    glyphon::cosmic_text::LineEnding::Lf,
                    line_attrs(lt, line_starts[old_idx], old_idx),
                    Shaping::Advanced,
                ));
            }
        }

        // Splice the changed band into the glyphon line vector. The unchanged
        // prefix lines (0..prefix) and suffix lines (old_end..old_len) keep their
        // identity and cached shaping.
        //
        // MARKDOWN STYLING NOTE: only the CHANGED band is re-styled here; an
        // unchanged-TEXT prefix/suffix line keeps its prior md attrs. Markdown is
        // overwhelmingly line-local (bold/italic/code/heading/link), so this is
        // correct for the typing-fast common case. A multi-line construct toggled
        // ABOVE unchanged lines (opening a ``` fence or `>` quote) could leave a
        // few cached lines styled by the OLD parse until they are themselves
        // touched — accepted to preserve the incremental single-line reshape. The
        // freshly-parsed `self.md_spans` (below) always reflects the whole doc, so
        // the sidecar + focus compositing stay accurate.
        self.buffer.lines.splice(prefix..old_end, replacement);
        // PERSISTENT MARGIN OUTLINE: distill the document's headings from the SAME
        // freshly-parsed markdown spans (no second pulldown parse) — before the move
        // below, while both `text` and `md_spans` are in hand. Empty for a
        // non-markdown buffer (the outline is a markdown/notes surface), so a
        // `.rs`/`.txt` buffer keeps an empty list. Recompute the CURRENT heading
        // (nearest at/above the caret) off the fresh list; the render phase gates its
        // re-upload on `last_outline_current` crossing.
        self.outline_headings = if md {
            crate::markdown::headings_from_spans(text, &md_spans)
        } else {
            Vec::new()
        };
        self.last_outline_current = self.outline_current();
        // Store the fresh whole-document span list (used by focus compositing and
        // the capture sidecar). Moved out of the closure now that it is done.
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        // Stash the per-line image heights so the caret-driven restyle passes
        // (`restyle_all_lines` / `refresh_rule_conceal`), which run on UNCHANGED
        // text, keep the same tall rows without re-reading image headers.
        self.image_heights = image_heights;

        self.finalize_buffer_lines(&attrs);
    }

    /// Diff the freshly split `new_lines` against the live buffer: the common
    /// unchanged prefix + suffix bound the changed middle band — `[prefix, old_end)`
    /// in the old buffer, `[prefix, new_end)` in the new text — whose lines outside
    /// the band keep their cached shaping (we never even visit them).
    pub(super) fn unchanged_band(&self, new_lines: &[&str]) -> (usize, usize, usize) {
        // Find the common UNCHANGED prefix of lines (the typical edit touches a
        // line in the middle/end, so everything above it is identical and keeps
        // its cached shaping untouched — we don't even visit those).
        let old_len = self.buffer.lines.len();
        let new_len = new_lines.len();
        let mut prefix = 0usize;
        while prefix < old_len
            && prefix < new_len
            && self.buffer.lines[prefix].text() == new_lines[prefix]
        {
            prefix += 1;
        }
        // Find the common UNCHANGED suffix (below the edit), not overlapping the
        // prefix. Lines here are byte-identical and keep their cached shaping.
        let mut suffix = 0usize;
        while suffix < old_len.saturating_sub(prefix)
            && suffix < new_len.saturating_sub(prefix)
            && self.buffer.lines[old_len - 1 - suffix].text() == new_lines[new_len - 1 - suffix]
        {
            suffix += 1;
        }
        // The changed middle band is [prefix, old_len-suffix) in the old buffer and
        // [prefix, new_len-suffix) in the new text. Replace exactly that band; the
        // prefix and suffix `BufferLine`s (with their cached shaping) are reused.
        let old_end = old_len - suffix;
        let new_end = new_len - suffix;
        (prefix, old_end, new_end)
    }

    /// Enforce cosmic-text's BufferLine invariants after a splice: the last line
    /// must end `None`, the buffer must never be empty, then flag a redraw.
    pub(super) fn finalize_buffer_lines(&mut self, attrs: &Attrs<'static>) {
        // cosmic-text requires the LAST line to carry `LineEnding::None`. Our lines
        // all got `Lf`; fix up the final one (a no-op reset when it's already None).
        if let Some(last) = self.buffer.lines.last_mut() {
            last.set_ending(glyphon::cosmic_text::LineEnding::None);
        }
        // Defensive: never leave the buffer with zero lines (cosmic-text invariant).
        if self.buffer.lines.is_empty() {
            self.buffer.lines.push(glyphon::cosmic_text::BufferLine::new(
                "",
                glyphon::cosmic_text::LineEnding::None,
                glyphon::cosmic_text::AttrsList::new(attrs),
                Shaping::Advanced,
            ));
        }
        self.buffer.set_redraw(true);
    }

    /// Rebuild EVERY line's `AttrsList` (markdown + CJK spans) at the CURRENT
    /// metrics, then re-shape. Heading lines carry ABSOLUTE per-span `metrics` (a
    /// fixed pixel size), and the incremental text path only rebuilds lines whose
    /// TEXT changed — so on a pure ZOOM/DPI change the (unchanged) heading lines
    /// would keep their stale pixel size and fail to scale with the body. Callers
    /// gate this on "a markdown buffer that actually has a heading" so the common
    /// case never pays for it.
    pub(super) fn restyle_all_lines(&mut self) {
        let attrs = self.doc_attrs();
        let fonts = self.resolve_script_fonts();
        let doc_lang = self.doc_lang;
        let cjk_priority = self.cjk_priority.clone();
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let md_spans = std::mem::take(&mut self.md_spans);
        let syn_spans = std::mem::take(&mut self.syn_spans);
        // INLINE IMAGES: reuse the per-line heights computed at the last reshape so
        // an image line keeps its tall row through a zoom/DPI restyle. NOTE (logged
        // scope trim): the row is NOT re-fit to the zoomed column here (no image
        // header is re-read on a pure restyle) — it re-fits on the next text
        // edit/reshape, exactly like the caret-driven conceal path below.
        let image_heights = std::mem::take(&mut self.image_heights);
        // ITEM 5 REWORK: same "reuse the last reshape's table" treatment as
        // `image_heights` above — a zoom/DPI restyle doesn't re-measure the
        // forcing `letter_spacing` (no text/wrap change), it just re-applies it.
        let image_force = std::mem::take(&mut self.image_force);
        // REVEAL-ON-CURSOR: conceal every hr line's `---` EXCEPT the caret's (mirrors
        // the incremental path so a zoom/DPI restyle keeps the same conceal/reveal).
        // `cursor_byte` additionally drives the WYSIWYG fence conceal's BLOCK scope.
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
        // SELECTION REVEAL: same extent `set_text_incremental` computes, from
        // the CURRENT `self.buffer.lines` (valid here, unlike mid-splice).
        let selection_touch = selection_touch_bytes(
            self.selection,
            |i| self.line_doc_byte_start(i),
            |i| self.buffer.lines.get(i).map(|l| l.text().len()).unwrap_or(0),
        );
        let mut start = 0usize;
        for li in 0..self.buffer.lines.len() {
            let tlen = self.buffer.lines[li].text().len();
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let al = build_line_attrs(
                    &attrs, base_fs, base_lh, md, line.text(), start, &md_spans, &syn_spans,
                    doc_lang, &cjk_priority, &fonts, li != cursor_line, cursor_byte,
                    image_heights.get(li).copied().flatten(),
                    image_force.get(li).copied().flatten(),
                    selection_touch.as_ref(),
                );
                line.set_attrs_list(al);
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.image_heights = image_heights;
        self.image_force = image_force;
        self.row_geom.invalidate();
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.buffer.set_redraw(true);
    }

    /// REVEAL-ON-CURSOR upkeep: re-lay every line whose conceal state depends on the
    /// CARET OR ACTIVE SELECTION — the markdown horizontal-rule / bullet-marker
    /// conceal (each keyed to its OWN line) AND every WYSIWYG-concealable
    /// [`crate::markdown::MdKind::ConcealMarkup`] span (heading/emphasis/inline-code/
    /// highlight, each line-scoped like the hr/bullet; a fenced block's marker lines,
    /// block-scoped) — so it all matches the CURRENT caret line/position AND the
    /// current selection's touched lines (2026-07-22, "selection reveals raw
    /// markdown"). The incremental text path only rebuilds lines whose TEXT changed,
    /// so a PURE cursor/selection move (no edit) would otherwise leave a stale
    /// conceal/reveal; this closes that gap. Called from `set_view`'s sync (which
    /// runs on every `set_view`), so the toggle tracks the caret AND the selection
    /// with no new state threaded through `render.rs`.
    ///
    /// Cheap + idempotent: only lines carrying a concealable span are visited, and
    /// rebuilding the SAME attrs no-ops in `set_attrs_list` (it resets shaping only
    /// when the attrs differ), so a move that crosses no concealable boundary reshapes
    /// nothing.
    pub(super) fn refresh_rule_conceal(&mut self, force: bool) {
        if self.md_spans.is_empty() {
            self.last_conceal_cursor_line = Some(self.cursor_line);
            self.last_conceal_selection = self.selection;
            return;
        }
        // GATE (byte-identical): the conceal only toggles on a caret-LINE change OR
        // a SELECTION change (which line set now selection-reveals), so a pure
        // scroll / same-line-same-selection move / idle redraw would re-lay the SAME
        // attrs and no-op. Skip the O(lines × md_spans) rescan in that case. `force`
        // (a reshape / text edit / restyle just happened) always runs it, because the
        // reshape drops the per-line attrs and a newly-typed `---`/bullet/heading/etc.
        // must (re)conceal. TRIPWIRE: comparing the WHOLE selection (not just its
        // touched-line extent) means a selection that starts/ends/clears without
        // changing which LINES it touches still re-runs the (idempotent) rescan below
        // — a harmless no-op, never a missed reveal/conceal transition.
        if !force
            && self.last_conceal_cursor_line == Some(self.cursor_line)
            && self.last_conceal_selection == self.selection
        {
            return;
        }
        self.last_conceal_cursor_line = Some(self.cursor_line);
        self.last_conceal_selection = self.selection;
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
        // SELECTION REVEAL: same extent `set_text_incremental`/`restyle_all_lines`
        // compute (see `selection_touch_bytes`), from the CURRENT `self.buffer.lines`.
        let selection_touch = selection_touch_bytes(
            self.selection,
            |i| self.line_doc_byte_start(i),
            |i| self.buffer.lines.get(i).map(|l| l.text().len()).unwrap_or(0),
        );
        let attrs = self.doc_attrs();
        let fonts = self.resolve_script_fonts();
        let doc_lang = self.doc_lang;
        let cjk_priority = self.cjk_priority.clone();
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        let md = self.md_enabled;
        let md_spans = std::mem::take(&mut self.md_spans);
        let syn_spans = std::mem::take(&mut self.syn_spans);
        // INLINE IMAGES: keep the image line's tall row when the caret enters/leaves
        // it (a pure conceal toggle must NOT collapse the reserved height).
        let mut image_heights = std::mem::take(&mut self.image_heights);
        // ITEM 5 REWORK: the forced-trailing-row table (see `Self::image_force`'s
        // field doc) is JUST as cursor-dependent as `image_heights` used to be for
        // a mixed line — re-derived below alongside it.
        let mut image_force = std::mem::take(&mut self.image_force);
        let wrap = self.text_wrap_width();
        let base_font_size = self.metrics.font_size;
        let mut changed = false;
        let mut start = 0usize;
        for li in 0..self.buffer.lines.len() {
            let tlen = self.buffer.lines[li].text().len();
            let is_rule = md_spans.iter().any(|(r, k)| {
                *k == crate::markdown::MdKind::Rule && r.start < start + tlen + 1 && r.end > start
            });
            // A bullet line also toggles its conceal on caret move (reveal the raw `-`
            // when the caret lands on it, re-hide it under the glyph when it leaves) —
            // the SAME reveal-on-cursor upkeep the hr lines get, via the shared
            // [`crate::markdown::list_item`] detection.
            let is_bullet = crate::markdown::list_item(self.buffer.lines[li].text())
                .is_some_and(|it| !it.ordered);
            // A WYSIWYG-concealable line (heading/emphasis/code/highlight, or a
            // fenced block's marker lines) toggles too — any `ConcealMarkup` span
            // touching this line means its reveal state may depend on the caret.
            let is_concealable = md_spans.iter().any(|(r, k)| {
                matches!(k, crate::markdown::MdKind::ConcealMarkup(_))
                    && r.start < start + tlen + 1
                    && r.end > start
            });
            // ITEM 5 REWORK: a MIXED image line's forced-trailing-row entry is
            // CURSOR-DEPENDENT (unlike a bare line's — the caption model
            // deliberately holds THAT one fixed, see `Self::image_force`'s field
            // doc for why a revealed mixed line reserves nothing at all). A pure
            // cursor move (no text change, so `compute_image_layout` itself never
            // re-runs) must still re-derive that decision here — otherwise
            // entering/leaving the line via arrow keys would leave the STALE
            // reservation from whichever cursor position last triggered a full
            // reshape. `dh` is read back from the already-populated `image_report`
            // (no re-decode: the image + its display size are unchanged by a
            // cursor move). Gated on `is_concealable` (an image line always
            // carries its own `ConcealMarkup(Image)` span, so this never runs the
            // extra `image_report` scan on an ordinary line) and NOT wasm-gated:
            // on wasm `image_report` is always empty (inline-image decode is
            // native-only), so it is a harmless no-op there.
            if is_concealable {
                if let Some(dh) = self
                    .image_report
                    .borrow()
                    .iter()
                    .find(|im| im.line == li)
                    .map(|im| im.display_h)
                {
                    let line_text = self.buffer.lines[li].text().to_string();
                    // This line's own image span (its doc range minus `start`).
                    if let Some((img_start, img_end)) = md_spans.iter().find_map(|(r, k)| {
                        matches!(k, crate::markdown::MdKind::ConcealMarkup(crate::markdown::ConcealKind::Image))
                            .then(|| (r.start.max(start), r.end.min(start + tlen)))
                            .filter(|(s, e)| s < e)
                    }) {
                        let local_range = (img_start - start)..(img_end - start);
                        let mixed = super::spans::image_line_has_other_content(
                            &line_text,
                            local_range.clone(),
                        );
                        if mixed {
                            // `heights[line]` is never touched for a mixed line
                            // (bare/table rows own that slot exclusively now).
                            if let Some(slot) = image_heights.get_mut(li) {
                                *slot = None;
                            }
                            // SELECTION REVEAL (regression fix, item 16 follow-up):
                            // this caret-move-only rescan re-derives `image_force`
                            // on EVERY caret/selection tick (not just a reshape),
                            // so it must widen the SAME `revealed_now` test
                            // `compute_image_layout` now applies — caret line OR
                            // active selection touching this image's own span
                            // (`img_start..img_end`, via `selection_touches`, the
                            // SAME predicate, never re-derived) — or it would
                            // immediately re-force the row on the very next tick
                            // and undo a selection-driven park.
                            let revealed_now = li == cursor_line
                                || selection_touches(selection_touch.as_ref(), &(img_start..img_end));
                            let want = if revealed_now {
                                None
                            } else {
                                let prefix = &line_text[..local_range.start];
                                let last_row_w = Self::measure_last_row_width(
                                    &mut self.font_system,
                                    prefix,
                                    &attrs,
                                    base_font_size,
                                    wrap,
                                );
                                let remaining = (wrap - last_row_w).max(0.0);
                                Some((dh, remaining + Self::IMAGE_FORCE_MARGIN_PX))
                            };
                            if let Some(slot) = image_force.get_mut(li) {
                                *slot = want;
                            }
                        } else {
                            if let Some(slot) = image_heights.get_mut(li) {
                                *slot = Some(dh);
                            }
                            if let Some(slot) = image_force.get_mut(li) {
                                *slot = None;
                            }
                        }
                    }
                }
            }
            if is_rule || is_bullet || is_concealable {
                if let Some(line) = self.buffer.lines.get_mut(li) {
                    let al = build_line_attrs(
                        &attrs, base_fs, base_lh, md, line.text(), start, &md_spans, &syn_spans,
                        doc_lang, &cjk_priority, &fonts, li != cursor_line, cursor_byte,
                        image_heights.get(li).copied().flatten(),
                        image_force.get(li).copied().flatten(),
                        selection_touch.as_ref(),
                    );
                    changed |= line.set_attrs_list(al);
                }
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.image_heights = image_heights;
        self.image_force = image_force;
        if changed {
            // WYSIWYG v1.1: a reveal/conceal toggle can now change actual GLYPH
            // GEOMETRY, not just color (the zero-width metrics override — see
            // `add_wysiwyg_conceal_spans`), so the row-geometry memo
            // (`visual_rows`'s single-slot per-line cache) MUST invalidate here
            // too, exactly like `restyle_all_lines` already does — otherwise a
            // PURE cursor move (no full reshape) leaves `visual_rows` serving
            // the OLD (still-concealed or still-revealed) cached x-advances even
            // though the underlying buffer was correctly reshaped a line below.
            // Before this round every toggle here was COLOR-only, so the stale
            // memo was harmless; it is not anymore.
            self.row_geom.invalidate();
            // A crossed hr boundary reset those lines' shaping; re-shape so they lay
            // out with the new conceal/reveal before the next `prepare`.
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            self.buffer.set_redraw(true);
        }
    }

    /// Compose the document `text` with any active preedit spliced in at the cursor
    /// (the string actually handed to the shaper) and the preedit's char count (by
    /// which the effective cursor column is advanced so the caret sits at the
    /// preedit's end). With no preedit the composed text is `text` verbatim.
    pub(super) fn compose(&self, text: &str) -> (String, usize) {
        if self.preedit.is_empty() {
            return (text.to_string(), 0);
        }
        // Find the cursor's absolute char index in `text`, then its byte offset,
        // and splice the preedit there. Preedit strings carry no newlines (IME
        // composition is a single run), so it stays on the cursor line.
        let insert_char = line_col_to_char_index(text, self.cursor_line, self.cursor_col);
        let byte_at = text
            .char_indices()
            .nth(insert_char)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        let mut composed = String::with_capacity(text.len() + self.preedit.len());
        composed.push_str(&text[..byte_at]);
        composed.push_str(&self.preedit);
        composed.push_str(&text[byte_at..]);
        (composed, self.preedit.chars().count())
    }

    /// Splice the active preedit (if any) into `text`, then RESHAPE ONLY IF the
    /// composed string differs from what is already shaped (or `force` is set for a
    /// zoom change). Advances the effective cursor column to the preedit's end
    /// either way (a no-reshape cursor move still needs the caret placed correctly).
    ///
    /// The composed-string compare is the lever that makes every non-typing event
    /// free: a cursor move / scroll / selection change produces the SAME composed
    /// text, so `set_text` (and the whole shaping path) is skipped entirely.
    pub(super) fn shape_with_preedit(&mut self, text: &str, force: bool) {
        if self.preedit.is_empty() {
            // COMMON PATH (every non-composing frame): the composed text IS `text`
            // verbatim, so compare the shaped key against `text` DIRECTLY — no
            // `compose` allocation to clone-then-discard on a pure move / scroll /
            // selection change. Only allocate the owned `shaped_key` when we actually
            // (re)shape. No preedit chars to advance the caret past. Byte-identical to
            // the old `compose`-then-compare (which returned `text.to_string()` here).
            let unchanged = !force && self.shaped_key.as_deref() == Some(text);
            if !unchanged {
                self.set_text(text);
                self.shaped_key = Some(text.to_string());
            }
            return;
        }
        let (composed, preedit_chars) = self.compose(text);
        let unchanged = !force && self.shaped_key.as_deref() == Some(composed.as_str());
        if !unchanged {
            self.set_text(&composed);
            self.shaped_key = Some(composed);
        }
        // Caret lands after the preedit on the same logical line, shaped or not.
        self.cursor_col += preedit_chars;
    }

    /// Re-wrap the document buffer to the live [`Self::text_wrap_width`] if it has
    /// drifted from it. The single enforcement point for the invariant "buffer wrap
    /// width == text_wrap_width()", called once per frame from [`Self::prepare`] so NO
    /// state change can leave the buffer wrapped at a stale width (see the comment at
    /// the top of `prepare`). Cheap: skipped entirely when already in sync.
    pub(super) fn sync_wrap_width(&mut self) {
        let want = self.text_wrap_width();
        let have = self.buffer.size().0.unwrap_or(f32::NAN);
        if (have - want).abs() > 0.5 {
            let shape_h = self.full_shape_height();
            self.buffer
                .set_size(&mut self.font_system, Some(want), Some(shape_h));
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            self.row_geom.invalidate();
            // TABLES: a width-only drift (page-mode toggle / measure edit /
            // page-width drag) never bumps `reshape_count` on its own, so
            // `compute_table_layout` — the ONE table shape site — would
            // otherwise stay unrun here and both the reservation and the
            // cached drawn geometry would sit pinned to the LAST real
            // reshape's (now stale) width. Resync them together, from the
            // SAME call, so they catch up to the new width WITHOUT ever
            // disagreeing with each other even transiently.
            self.resync_table_layout_for_width();
        }
    }

    /// TABLES-ONLY companion to [`Self::sync_wrap_width`] (see its call site's
    /// doc comment for why this exists): re-run the ONE table shape site,
    /// [`Self::compute_table_layout`] — which refreshes [`layers::TableGridCache`]
    /// unconditionally — against the text `self.md_spans` was already parsed
    /// from (safe to reuse verbatim: this seam only ever fires when NO real
    /// `set_text` reshape ran this frame, i.e. the text itself is UNCHANGED),
    /// then merges the fresh per-line heights into the TABLE-OWNED lines of
    /// `self.image_heights` and rebuilds every line's attrs via
    /// [`Self::restyle_all_lines`] so the RESERVATION is baked in promptly too
    /// — never just the cache. "Table-owned" is decided by byte-range
    /// containment in [`Self::table_blocks`] (never an image line's slot: a
    /// line is never both an image reference and a table row). Cheap + a
    /// no-op the moment there is no table on the document at all (checked
    /// right after the (already-required) cache refresh, so that refresh
    /// always happens regardless).
    pub(super) fn resync_table_layout_for_width(&mut self) {
        if !self.md_enabled {
            return;
        }
        let Some(text) = self.shaped_key.clone() else { return };
        let md_spans = self.md_spans.clone();
        let table_heights = self.compute_table_layout(&text, &md_spans);
        if table_heights.iter().all(Option::is_none) {
            // No wrapped-table row anywhere (either no table at all, or every
            // table fits) — the cache refresh above already covers a fitting
            // table's column widths; there is no reservation to (re)bake.
            return;
        }
        let blocks = self.table_blocks();
        if blocks.is_empty() {
            return;
        }
        let mut start = 0usize;
        for (li, l) in text.split('\n').enumerate() {
            let in_table = blocks.iter().any(|(_, r)| r.start <= start && start < r.end);
            if in_table {
                if let Some(slot) = self.image_heights.get_mut(li) {
                    *slot = table_heights.get(li).copied().flatten();
                }
            }
            start += l.len() + 1;
        }
        self.restyle_all_lines();
    }

    /// A buffer height tall enough to shape EVERY visual row of the document, so
    /// `layout_runs()` covers the whole doc (not just one window). Soft-wrap can
    /// turn each logical line into several rows, so we budget a few rows per
    /// logical line plus a floor, all at the (zoomed) line height. Generous on
    /// purpose; cosmic-text simply lays out all rows that fit and these documents
    /// are small.
    pub(super) fn full_shape_height(&self) -> f32 {
        let logical = self.buffer.lines.len().max(1);
        // Allow up to ~8 wrapped rows per logical line before we'd undercount —
        // far more than realistic prose wrap — plus a fixed floor so a tiny doc
        // still shapes comfortably.
        let rows = (logical.saturating_mul(8)).max(64) as f32;
        TEXT_TOP + rows * self.metrics.line_height + self.metrics.line_height
    }

    /// True when the buffer has at least one heading LINE (a leading-`#` run that
    /// scales) — the only thing that introduces a non-uniform (larger) row, and so
    /// the only reason a zoom/DPI change needs a full attrs rebuild
    /// ([`Self::restyle_all_lines`]). Scans line text (cheap; awl docs are small)
    /// rather than the pulldown spans, so an in-progress `#foo` still counts.
    pub(super) fn has_heading_lines(&self) -> bool {
        if !self.md_enabled {
            return false;
        }
        self.buffer
            .lines
            .iter()
            .any(|l| md_line_scale(l.text(), true) != 1.0)
    }
}

/// The ONE owner of "which OpenType font features apply to a shaping context" —
/// a PURE function of (is this a CODE buffer, the concrete FACE being shaped,
/// the sticky `code_ligatures` toggle) → the `FontFeatures` set. BOTH
/// feature-setting sites consult it — the document body ([`TextPipeline::doc_attrs`]
/// here) and the summoned panels ([`super::panel_attrs`] in `render.rs`) — so the
/// two can never drift on ligatures.
///
/// THE THREE-WAY LIGATURE SPLIT (settled 2026-07, per the per-mono pitch probe):
///   * **PROSE** (a non-code buffer / the proportional display face): STANDARD
///     ligatures ON (the Butterick-approved fi/fl collision-fixers) + contextual
///     ligatures; DISCRETIONARY off. `code_ligatures` does NOT gate prose —
///     standard fi/fl is uncontroversial and always on. A true `liga` fi→single-
///     glyph REDUCES glyph count, which `assemble_glyph_xs` handles (linear split
///     → uniform pitch); the only cost is sub-glyph caret/selection granularity
///     inside a rare fi/fl (flagged for the next phase's measured verification).
///     `calt` (contextual ALTERNATES — Monaspace's programming-ligature engine,
///     e.g. `!=`→`≠`) is explicitly OFF here too, unconditionally, regardless of
///     face — LIVE BUG this round fixed: a mono-display world's (Tawny/Potoroo)
///     prose buffer left `calt` untouched, so the shaper fell through to the
///     font's own default (usually ON), and ordinary prose punctuation
///     landing next to unrelated punctuation across a word/markup boundary
///     (`==highlight!!==` — the `!!` emphasis marker's trailing `!` sitting next
///     to the `==` highlight delimiter's leading `=`) got silently read as a
///     programming ligature and fused into `≠`. TASTE CALL, logged: this
///     forfeits Monaspace's prose "texture healing" (its `calt` also nudges a
///     few ordinary glyph pairs for visual smoothing) in exchange for prose
///     punctuation never being misread as code — `calt` stays a CODE-buffer-only
///     feature (below), never a prose one, on any face.
///   * **CODE on a PITCH-SAFE mono** (JetBrains Mono, Iosevka): the PROGRAMMING
///     ligatures those monos ship, which ride `calt` (contextual alternates) and
///     substitute glyph SHAPES while keeping 1 glyph per source char + per-char
///     clusters (probe: maxdev 0.0) — so the uniform mono grid holds. `liga` /
///     `clig` / `dlig` stay OFF (irrelevant to the programming set, and a true
///     `liga` substitution could merge clusters); `calt` ON is the whole
///     mechanism. This is exactly what the shipping build already rendered (it
///     never disabled `calt`), now made explicit + gated.
///   * **CODE on an UNSAFE / inert / unknown mono** (Monaspace Xenon, IBM Plex
///     Mono, anything unclassified): LIGATURE-FREE. Monaspace's texture-healing
///     ligatures ride `rclt` (Required Contextual Alternates) + `ccmp` and MERGE
///     glyph clusters (several glyphs share one source cluster), which breaks
///     `assemble_glyph_xs`'s byte→x map → non-uniform `line_glyph_xs` →
///     caret/hit-test/selection column math breaks on any `->`/`=>`/`::` line (a
///     LATENT bug in the shipping build, which never disabled `rclt`). There is
///     NO "ligatures + clean per-char columns" option for Monaspace via font
///     features, so it is deliberately ligature-free: `calt` + `rclt` + `ccmp`
///     OFF restores uniform pitch (probe: maxdev 0.0). IBM Plex Mono has no
///     programming ligatures at all, so this set is a harmless no-op there; an
///     UNKNOWN mono defaults here too (conservative — guarantees uniform pitch
///     until a mono is explicitly classified pitch-safe via [`mono_is_pitch_safe`]).
///   * **DISCRETIONARY** ligatures (the quaint st/ct) are OFF in EVERY context.
///
/// `code_ligatures == false` forces the LIGATURE-FREE code set for every mono
/// (prose is unaffected — it never rode the toggle). For Monaspace / IBM Plex the
/// toggle is a no-op (they are ligature-free either way); it meaningfully flips
/// only the pitch-safe monos' `calt`.
pub(super) fn font_features(
    is_code: bool,
    face: &str,
    code_ligatures: bool,
) -> glyphon::cosmic_text::FontFeatures {
    use glyphon::cosmic_text::{FeatureTag, FontFeatures};
    let mut ff = FontFeatures::new();
    // DISCRETIONARY off in EVERY context (the quaint st/ct) — the universal rule.
    ff.disable(FeatureTag::DISCRETIONARY_LIGATURES);
    if !is_code {
        // PROSE: standard + contextual ligatures ON (fi/fl collision-fixers).
        // `calt` OFF unconditionally — see the module doc's PROSE bullet: a
        // mono-display world would otherwise inherit the font's own default
        // `calt` state and risk fusing unrelated adjacent prose punctuation
        // (e.g. `==highlight!!==`) into a programming ligature (`!=`→`≠`).
        ff.enable(FeatureTag::STANDARD_LIGATURES);
        ff.enable(FeatureTag::CONTEXTUAL_LIGATURES);
        ff.disable(FeatureTag::CONTEXTUAL_ALTERNATES);
        return ff;
    }
    // CODE: never standard/contextual ligatures — on a mono they'd merge clusters.
    ff.disable(FeatureTag::STANDARD_LIGATURES);
    ff.disable(FeatureTag::CONTEXTUAL_LIGATURES);
    if code_ligatures && mono_is_pitch_safe(face) {
        // PITCH-SAFE mono (JBM / Iosevka): programming ligatures ride `calt`,
        // 1 glyph per source char, uniform pitch preserved.
        ff.enable(FeatureTag::CONTEXTUAL_ALTERNATES);
    } else {
        // UNSAFE / inert / unknown mono, OR the toggle is OFF → LIGATURE-FREE.
        // `calt` off (no contextual substitution); `rclt` + `ccmp` off to stop
        // Monaspace's texture-healing cluster merge (the per-mono probe's fix),
        // which is what would otherwise break `line_glyph_xs` on a `-> => ::` line.
        ff.disable(FeatureTag::CONTEXTUAL_ALTERNATES);
        ff.disable(FeatureTag::new(b"rclt"));
        ff.disable(FeatureTag::new(b"ccmp"));
    }
    ff
}

/// Whether a mono FACE keeps STRICT uniform advance under its programming
/// ligatures (so `calt` can be left ON for a code buffer) — the per-mono safety
/// verdict from the pitch probe. Only the two monos the probe MEASURED as safe
/// (their ligatures ride `calt`: 1 glyph per source char, per-char clusters,
/// maxdev 0.0) are listed. Every OTHER mono is treated as UNSAFE and rendered
/// ligature-free, so the uniform grid can never silently break:
///   * **Monaspace Xenon** — its texture-healing ligatures ride `rclt` + `ccmp`
///     and MERGE glyph clusters → non-uniform `line_glyph_xs` (there is no clean
///     per-char option for it via font features).
///   * **IBM Plex Mono** — ships no programming ligatures at all, so ligature-free
///     is a no-op (nothing to enable).
///   * any **future / unknown** mono — conservative default until measured.
///
/// Add a mono here ONLY after the pitch probe (`mono_world_shapes_uniform_pitch`,
/// extended to shape real `-> => !=` content, not repeated single chars) confirms
/// it holds strict advance with `calt` on.
pub(super) fn mono_is_pitch_safe(face: &str) -> bool {
    matches!(face, "JetBrains Mono" | "Iosevka")
}
