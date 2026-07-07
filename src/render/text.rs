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
        // Disable shaping LIGATURES (liga/clig/dlig) for the document body. On a
        // proportional face like Literata, "Th"/"fi"/"ffi" otherwise shape into a
        // SINGLE ligature glyph spanning TWO source chars — which makes the per-char
        // model break down: the morph caret on 'T' would silhouette the whole "Th"
        // (covering the H), and a selection of just 'T' would highlight the whole
        // ligature. Turning ligatures off makes 1 char = 1 glyph everywhere, so the
        // caret mask, hit-test, and selection x-slices are all genuinely
        // per-character with zero special-casing downstream. This is also standard
        // text-EDITOR behaviour (editing inside a ligature is confusing). Kerning
        // and contextual alternates stay on, so non-ligature spacing is unaffected.
        let mut ff = glyphon::cosmic_text::FontFeatures::new();
        ff.disable(glyphon::cosmic_text::FeatureTag::STANDARD_LIGATURES);
        ff.disable(glyphon::cosmic_text::FeatureTag::CONTEXTUAL_LIGATURES);
        ff.disable(glyphon::cosmic_text::FeatureTag::DISCRETIONARY_LIGATURES);
        let fam = self.doc_family();
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

    /// The per-line BASE attrs + effective row LINE-HEIGHT for logical line `li`,
    /// accounting for BOTH a heading's size scale AND an inline image's reserved
    /// tall row — the SAME decision [`build_line_attrs`] makes from its
    /// `image_row_height` argument, shared here so the focus-recolor paths
    /// ([`Self::clear_focus_spans`] / [`Self::color_char_range`]), which assemble a
    /// line's attrs OUTSIDE `build_line_attrs`, can never drift on the row height.
    /// An image line uses a NORMAL font size over its tall display line-height; a
    /// non-image line uses the heading size scale (`1.0` for body).
    pub(super) fn line_metric_base(
        &self,
        li: usize,
        base: &Attrs<'static>,
    ) -> (Attrs<'static>, f32) {
        let base_fs = self.metrics.font_size;
        let base_lh = self.metrics.line_height;
        match self.image_heights.get(li).copied().flatten() {
            Some(h) => (
                base.clone().metrics(GlyphMetrics::new(base_fs, h)),
                h,
            ),
            None => {
                let scale =
                    super::spans::md_line_scale(self.buffer.lines[li].text(), self.md_enabled);
                (
                    super::spans::scaled_base_attrs(base, base_fs, base_lh, scale),
                    base_lh * scale,
                )
            }
        }
    }

    /// INLINE IMAGES: compute the per-LOGICAL-LINE image display HEIGHT table (the
    /// value [`build_line_attrs`] uses to reserve a tall row) AND refill the
    /// deterministic [`Self::image_report`] (the sidecar + next-phase draw source).
    /// Reads each `ConcealMarkup(Image)` md_span's `![alt](path)` source with
    /// [`crate::markdown::parse_image_source`], then reads ONLY the image file's
    /// header dimensions (`into_dimensions` — no full decode) to fit-to-column via
    /// the pure [`super::spans::image_display_size`]. A missing/unreadable file
    /// reserves a placeholder-height row (the placeholder GLYPH is the next phase).
    /// Returns an all-`None` table when the feature is off / not markdown / on wasm,
    /// so the render stays byte-identical (no tall row is ever reserved).
    fn compute_image_layout(
        &self,
        text: &str,
        md_spans: &[(std::ops::Range<usize>, crate::markdown::MdKind)],
    ) -> Vec<Option<f32>> {
        let mut report = self.image_report.borrow_mut();
        report.clear();
        let line_count = text.split('\n').count().max(1);
        #[allow(unused_mut)]
        let mut heights = vec![None; line_count];
        #[cfg(not(target_arch = "wasm32"))]
        if crate::markdown::inline_images_on() && self.md_enabled {
            use crate::markdown::{ConcealKind, MdKind};
            let wrap = self.text_wrap_width();
            let base_lh = self.metrics.line_height;
            let cursor_line = self.cursor_line;
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
                let (dw, dh, missing) = match dims {
                    Some((w, h)) => {
                        let (dw, dh) =
                            super::spans::image_display_size(w, h, img.width_hint, wrap);
                        (dw, dh, false)
                    }
                    None => (
                        wrap.max(1.0),
                        base_lh * super::spans::IMAGE_MISSING_ROW_LINES,
                        true,
                    ),
                };
                if let Some(slot) = heights.get_mut(line) {
                    *slot = Some(dh);
                }
                report.push(crate::render::ImageReport {
                    range: (r.start, r.end),
                    line,
                    path: img.path,
                    alt: img.alt,
                    width_hint: img.width_hint,
                    display_w: dw,
                    display_h: dh,
                    missing,
                    revealed: line == cursor_line,
                });
            }
        }
        let _ = md_spans;
        heights
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
        // INLINE IMAGES: per-line reserved display heights (+ the sidecar/draw
        // report), read from the just-parsed `ConcealMarkup(Image)` spans and each
        // image's header dimensions. All-`None` (no tall rows) when the feature is
        // off / non-markdown / wasm, so the render below stays byte-identical.
        let image_heights = self.compute_image_layout(text, &md_spans);
        // Split into lines WITHOUT the line terminators (cosmic-text stores the
        // ending separately). `str::lines()` drops a single trailing newline, which
        // matches cosmic-text's "trailing empty line" handling: we re-add an empty
        // final line below so an end-of-buffer caret has a line to sit on.
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
        // REVEAL-ON-CURSOR: a markdown horizontal-rule line conceals its raw `---`
        // (transparent ink, fleuron alone) UNLESS the caret is on it, in which case
        // the dashes reveal for editing. `conceal_rule` is keyed off the line index
        // vs `self.cursor_line` (read here so the closure stays a plain capture).
        let cursor_line = self.cursor_line;
        // WYSIWYG fence conceal is BLOCK-scoped: it needs the caret's own line's
        // first document byte (not just its line index) to test containment in a
        // fenced block's whole byte range. `line_starts` is already built above.
        let cursor_byte = line_starts.get(cursor_line).copied().unwrap_or(0);
        let doc_lang = self.doc_lang;
        let cjk_priority = &self.cjk_priority;
        let line_attrs = |lt: &str, start: usize, li: usize| {
            let conceal_off_cursor = li != cursor_line;
            build_line_attrs(
                &attrs, base_fs, base_lh, md, lt, start, &md_spans, &syn_spans, doc_lang,
                cjk_priority, &fonts, conceal_off_cursor, cursor_byte,
                image_heights.get(li).copied().flatten(),
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
    /// case never pays for it. Leaves focus coloring to the caller's `update_focus`
    /// (the rebuilt attrs drop the per-line focus spans, mirroring a reshape).
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
        // REVEAL-ON-CURSOR: conceal every hr line's `---` EXCEPT the caret's (mirrors
        // the incremental path so a zoom/DPI restyle keeps the same conceal/reveal).
        // `cursor_byte` additionally drives the WYSIWYG fence conceal's BLOCK scope.
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
        let mut start = 0usize;
        for li in 0..self.buffer.lines.len() {
            let tlen = self.buffer.lines[li].text().len();
            if let Some(line) = self.buffer.lines.get_mut(li) {
                let al = build_line_attrs(
                    &attrs, base_fs, base_lh, md, line.text(), start, &md_spans, &syn_spans,
                    doc_lang, &cjk_priority, &fonts, li != cursor_line, cursor_byte,
                    image_heights.get(li).copied().flatten(),
                );
                line.set_attrs_list(al);
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.image_heights = image_heights;
        self.row_geom.invalidate();
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.buffer.set_redraw(true);
    }

    /// REVEAL-ON-CURSOR upkeep: re-lay every line whose conceal state depends on the
    /// CARET — the markdown horizontal-rule / bullet-marker conceal (each keyed to its
    /// OWN line) AND, since this round, every WYSIWYG-concealable
    /// [`crate::markdown::MdKind::ConcealMarkup`] span (heading/emphasis/inline-code/
    /// highlight, each line-scoped like the hr/bullet; a fenced block's marker lines,
    /// block-scoped) — so it all matches the CURRENT caret line/position. The
    /// incremental text path only rebuilds lines whose TEXT changed, so a PURE cursor
    /// move (no edit) would otherwise leave a stale conceal/reveal; this closes that
    /// gap. Called from [`Self::update_focus`] (which runs on every `set_view`), so the
    /// toggle tracks the caret with no new state threaded through `render.rs`.
    ///
    /// Cheap + idempotent: only lines carrying a concealable span are visited, and
    /// rebuilding the SAME attrs no-ops in `set_attrs_list` (it resets shaping only
    /// when the attrs differ), so a move that crosses no concealable boundary reshapes
    /// nothing. Lines currently carrying a focus color span are SKIPPED — the focus
    /// pass owns their attrs and applies the same conceal — so this never fights the
    /// typewriter/paragraph coloring.
    pub(super) fn refresh_rule_conceal(&mut self, force: bool) {
        if self.md_spans.is_empty() {
            self.last_conceal_cursor_line = Some(self.cursor_line);
            return;
        }
        // GATE (byte-identical): the conceal only toggles on a caret-LINE change, so a
        // pure scroll / same-line move / idle redraw would re-lay the SAME attrs and
        // no-op. Skip the O(lines × md_spans) rescan in that case. `force` (a reshape /
        // text edit / restyle just happened) always runs it, because the reshape drops
        // the per-line attrs and a newly-typed `---`/bullet/heading/etc. must (re)conceal.
        if !force && self.last_conceal_cursor_line == Some(self.cursor_line) {
            return;
        }
        self.last_conceal_cursor_line = Some(self.cursor_line);
        let cursor_line = self.cursor_line;
        let cursor_byte = self.line_doc_byte_start(cursor_line);
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
        let image_heights = std::mem::take(&mut self.image_heights);
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
            if (is_rule || is_bullet || is_concealable) && !self.focus_lines.contains(&li) {
                if let Some(line) = self.buffer.lines.get_mut(li) {
                    let al = build_line_attrs(
                        &attrs, base_fs, base_lh, md, line.text(), start, &md_spans, &syn_spans,
                        doc_lang, &cjk_priority, &fonts, li != cursor_line, cursor_byte,
                        image_heights.get(li).copied().flatten(),
                    );
                    changed |= line.set_attrs_list(al);
                }
            }
            start += tlen + 1;
        }
        self.md_spans = md_spans;
        self.syn_spans = syn_spans;
        self.image_heights = image_heights;
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
        }
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
