//! `OverlayState`'s QUERY/NAVIGATION half: the fuzzy refilter, the
//! type/scroll/move-selection primitives, and the row-display accessors the
//! renderer + sidecar read (`item_strings`, `empty_message`, …). Split out of
//! the former `overlay.rs` monolith (2026-07 code-organization pass); every
//! item's path is unchanged -- only the file it lives in moved.

use super::{OverlayKind, OverlayState};
use crate::fuzzy::{self, Tier};

impl OverlayState {
    /// Re-rank `corpus` against the current query into `items`, clamping the
    /// selection. Called after every query edit.
    pub fn refilter(&mut self) {
        let mut scored = fuzzy::rank(&self.query, &self.corpus, |i| {
            if self.open.contains(&i) {
                Tier::Open
            } else if self.recent.contains(&i) {
                Tier::Recent
            } else {
                Tier::Corpus
            }
        });
        // MRU TIEBREAK: `self.recent` is ordered MOST-RECENT-FIRST (the persisted
        // recently-opened MRU for Goto, the recently-run MRU for the Command palette).
        // Among rows with an EQUAL fuzzy+tier score, the more-recently-used one
        // (smaller position in `recent`) sorts first; non-recent rows fall to
        // `usize::MAX` and keep their original corpus order. `fuzzy::rank` already
        // sorted by (score desc, index asc); this stable re-sort inserts the MRU key
        // between them, so the Recent lens reads newest-first without any per-picker
        // code. Inert when `recent` is empty (the headless capture path) — every
        // position is `MAX`, so the order is byte-identical to the plain rank.
        let recent_rank = |ci: usize| self.recent.iter().position(|&x| x == ci).unwrap_or(usize::MAX);
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| recent_rank(a.index).cmp(&recent_rank(b.index)))
                .then_with(|| a.index.cmp(&b.index))
        });
        let mut ranked: Vec<usize> = scored.into_iter().map(|r| r.index).collect();
        // SPELL "Add to dictionary" EXEMPTION: the add row acts on the TARGETED
        // word, not the typed query, so a fuzzy filter that dropped it (its label
        // didn't match what you typed) re-appends it at the END — it is always
        // reachable while the spell picker is open. Inert for every other kind
        // (`spell_add` empty).
        if self.spell_add.iter().any(|&a| a) {
            for (ci, &is_add) in self.spell_add.iter().enumerate() {
                if is_add && !ranked.contains(&ci) {
                    ranked.push(ci);
                }
            }
        }
        // RUNTIME-GATED ROW FILTER (Command palette only, today): drop any corpus
        // entry marked `hidden` (e.g. "Finish file" with no daemon `--wait` client
        // actively waiting — see `commands::visible_hidden_mask`). `corpus` itself
        // stays untouched — only what's rankable/selectable shrinks — so the
        // row-index math `commands::visible_action_of` relies on stays valid. A
        // no-op (`hidden` empty) for every kind but the Command palette.
        if !self.hidden.is_empty() {
            ranked.retain(|&i| !self.hidden.get(i).copied().unwrap_or(false));
        }
        // DOTFILE DISPLAY FILTER (file pickers only, gated on `show_hidden`): drop any
        // corpus entry whose basename / ancestor component starts with `.` (except
        // `.env*`). The full corpus is untouched — this is purely what's SHOWN — so
        // flipping `show_hidden` and re-running `refilter` reveals them with no
        // filesystem re-read. A no-op for non-file pickers (theme/command/…) and when
        // dotfiles are revealed.
        // The Project explorer's synthetic "." accept-this-folder row is EXEMPT — it is
        // the "pick THIS folder" affordance, not a dotfile — so it survives the filter
        // (and is never revealed/re-hidden by the toggle either). Go-to HEADING rows are
        // likewise exempt (a heading title is prose, not a dotfile path).
        if !self.show_hidden && self.kind.hides_dotfiles() {
            ranked.retain(|&i| {
                self.corpus[i] == "."
                    || self.heading.get(i).copied().unwrap_or(false)
                    || !crate::index::is_hidden_entry(&self.corpus[i])
            });
        }
        // HEADINGS-LENS GATE (Go-to only): a REFINEMENT lens other than Headings
        // (Recent / This folder) still lists files only — the appended document-
        // heading rows are dropped there. The flat `All` home (`facet_lens == 0`)
        // is the UNIFIED DEFAULT (item 11): it keeps heading rows IN, mixed with
        // file rows and ranked together by the same fuzzy score, so one query can
        // reach either kind. The Headings lens re-admits them exclusively via its
        // own bucket ([`crate::index::goto_bucket`]). Inert when `heading` is empty
        // (every other picker, and a Go-to over a buffer with no headings).
        if !self.heading.is_empty() && self.facet_lens != 0 && self.active_facet_id() != Some("headings") {
            ranked.retain(|&i| !self.heading.get(i).copied().unwrap_or(false));
        }
        // FACETING picker under a real lens (strip index != 0, the All home): GROUP the
        // (fuzzy-matched) items into the lens's sections, in section order, preserving
        // the fuzzy rank WITHIN each section. `item_sections` records each row's section
        // (the faint header). The flat All home (and every non-faceting kind) keeps the
        // plain ranked list. GENERIC: the picker's own scheme supplies the sections +
        // the per-item bucketing — no picker-specific code here.
        let scheme = self.facet_scheme();
        if let Some(sc) = scheme.filter(|_| self.facet_lens != 0) {
            let mut items = Vec::with_capacity(ranked.len());
            let mut sections = Vec::with_capacity(ranked.len());
            for sect in sc.strip[self.facet_lens].sections {
                for &ci in &ranked {
                    // OPT-OUT faceting: an item with `None` on this lens yields `None`
                    // here, matching no section, so it is omitted from the lens (still
                    // reachable under All). Only `Some(section)` items are placed. The
                    // bucket sees the accept string PLUS the universal dir/git flags
                    // (the file pickers' Folders / Files / Git lenses key off them).
                    let fi = crate::facets::FacetItem {
                        accept: &self.corpus[ci],
                        is_dir: self.is_dir.get(ci).copied().unwrap_or(false),
                        is_git: self.git.get(ci).copied().unwrap_or(false),
                        // Command palette's Recent lens: reuse the recency tier vec.
                        recent: self.recent.contains(&ci),
                        // Go-to's Headings lens: this row is an appended doc heading.
                        heading: self.heading.get(ci).copied().unwrap_or(false),
                        // History's Session / Today lenses: the per-row stamp + the
                        // picker-global reference clocks (all `None` headless → inert).
                        ts: self.facet_ts.get(ci).copied(),
                        now: self.facet_now,
                        session_start: self.facet_session_start,
                    };
                    if (sc.bucket)(fi, self.facet_lens) == Some(*sect) {
                        items.push(ci);
                        sections.push((*sect).to_string());
                    }
                }
            }
            self.items = items;
            self.item_sections = sections;
        } else {
            self.item_sections = vec![String::new(); ranked.len()];
            self.items = ranked;
        }
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        self.scroll_to_selected();
        // DIFF-AS-PREVIEW: the row set changed under the highlight — whatever
        // version is now selected gets a fresh transcript, scrolled to its top.
        self.diff_scroll = 0;
    }

    /// THEME picker: the SECTION label for each filtered row, in the same order as
    /// [`Self::item_strings`] — the faint group header a row sits under (empty under
    /// All / for non-theme kinds). Surfaced to the render pipeline + sidecar so the
    /// grouping is drawable AND agent-verifiable.
    pub fn item_sections(&self) -> Vec<String> {
        self.item_sections.clone()
    }

    /// Append a char to the query and refilter. A query edit re-ranks the list, so the
    /// selection + scroll reset to the TOP (the best match).
    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
    }

    /// Remove the last query char and refilter.
    pub fn pop(&mut self) {
        self.query.pop();
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
    }

    /// ⌥⌫ (M-Backspace / DeleteWordBackward): remove the trailing WORD from the
    /// query — one whitespace/punct run + one word — then refilter. Shares the
    /// document buffer's exact word-delete boundary rule (the minibuffer caret is
    /// implicitly at the end of the append/pop query, so the boundary is computed
    /// from `query.len()`), so ⌥⌫ means the same thing in the palette as in the
    /// text. A NO-OP on an empty query (nothing to remove).
    pub fn pop_word(&mut self) {
        truncate_trailing_word(&mut self.query);
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
    }

    /// Cmd-Shift-. : REVEAL / re-hide dot-prefixed entries in THIS file picker (the
    /// Finder "show hidden files" convention). Flips `show_hidden` and re-runs the
    /// display filter (`refilter`) so the listing rebuilds with dotfiles shown/hidden
    /// — no filesystem re-read (the corpus already holds every entry). Resets the
    /// selection to the top (the row set changed under it). A calm NO-OP for a
    /// non-file picker (theme/command/…): those don't hide dotfiles, so there is
    /// nothing to reveal. Returns whether the flag actually flipped.
    pub fn toggle_hidden(&mut self) -> bool {
        if !self.kind.hides_dotfiles() {
            return false;
        }
        self.show_hidden = !self.show_hidden;
        self.selected = 0;
        self.scroll = 0;
        self.refilter();
        true
    }

    /// The per-kind visible ROW CAP (delegates to [`OverlayKind::window_rows`], the ONE
    /// owner). Both the scroll math here AND the pipeline's drawn window (via
    /// [`crate::render::ViewState::overlay_window_rows`]) read the same value, so the
    /// highlighted / hovered / drawn rows can never disagree.
    pub fn window_rows(&self) -> usize {
        self.kind.window_rows()
    }

    /// Scroll the window the MINIMUM needed so `selected` sits within
    /// `[scroll, scroll + window_rows)`, then clamp so the final page never shows a
    /// blank tail. Called after any keyboard move / refilter — NEVER on a hover.
    pub(super) fn scroll_to_selected(&mut self) {
        let window = self.window_rows();
        if window == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + window {
            self.scroll = self.selected + 1 - window;
        }
        let max_top = self.items.len().saturating_sub(window);
        if self.scroll > max_top {
            self.scroll = max_top;
        }
    }

    /// Move the selection by `delta` rows, clamped to the visible item range, then
    /// scroll the window to keep the new selection visible (the keyboard ↑/↓ + PgUp/Dn
    /// path). The WHEEL rides this too, so a wheel notch advances the list exactly like
    /// an arrow press.
    pub fn move_sel(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let n = self.items.len() as isize;
        let mut s = self.selected as isize + delta;
        if s < 0 {
            s = 0;
        }
        if s >= n {
            s = n - 1;
        }
        self.selected = s as usize;
        self.scroll_to_selected();
        // DIFF-AS-PREVIEW: a selection move means a NEW transcript — top it out.
        self.diff_scroll = 0;
    }

    /// JUMP the selection to the FIRST visible item (the Home/End-in-picker jump — see
    /// [`crate::actions::overlay_nav::overlay_intercept`]'s LineStart/BufferStart arm),
    /// then scroll the window to it. A saturating counterpart to [`Self::move_sel`] that
    /// can't over/underflow on a huge delta; an empty list floors at 0. The ONE owner of
    /// "go to the top row", so the keyboard jump and any future caller land identically.
    pub fn select_first(&mut self) {
        self.selected = 0;
        self.scroll_to_selected();
        self.diff_scroll = 0;
    }

    /// JUMP the selection to the LAST visible item (the End/Home-in-picker jump — the
    /// LineEnd/BufferEnd arm), then scroll the window to it. The ONE owner of "go to the
    /// bottom row"; an empty list floors at 0 (mirrors [`Self::move_sel`]'s empty guard).
    pub fn select_last(&mut self) {
        self.selected = self.items.len().saturating_sub(1);
        self.scroll_to_selected();
        self.diff_scroll = 0;
    }

    /// A HOVER re-highlights the row `target` ONLY when it is already within the current
    /// visible band `[scroll, scroll + window_rows)` (and is a real item). Returns whether
    /// the highlight moved. Crucially it NEVER touches `scroll`, so hovering the top /
    /// bottom edge — or anywhere off the visible rows — can't make the list auto-scroll:
    /// a hover highlights what's under the pointer, nothing more.
    pub fn hover_select(&mut self, target: usize) -> bool {
        let window = self.window_rows();
        let last = (self.scroll + window).min(self.items.len());
        if target >= self.scroll && target < last && target != self.selected {
            self.selected = target;
            // DIFF-AS-PREVIEW: hovering a different version previews ITS diff,
            // fresh from the top (the same reset every selection move takes).
            self.diff_scroll = 0;
            true
        } else {
            false
        }
    }

    /// ITEM 52 — RE-STAMP the card's frozen [`Self::align`] to the CURRENTLY-active
    /// world's own anchor. Called on a DELIBERATE selection crossing (keyboard nav,
    /// wheel, page/jump moves) AFTER [`crate::actions::preview_overlay`] has made the
    /// highlighted world active, so an open THEME picker SNAPS its card into the
    /// destination world's own left/center/right rail — choosing a world drops you
    /// inside it (the standing law; SUPERSEDES item 45's summon-time freeze for a
    /// deliberate move). PASSIVE pointer hover never calls this, so sweeping the
    /// pointer down the rows re-tints every world WITHOUT starting a spatial chase
    /// (the item-45 freeze still holds the card put through a hover). A NO-OP for
    /// every non-Theme picker: the active world can't move under them, so
    /// [`crate::render::effective_card_anchor`] returns the same anchor it froze at
    /// summon. It reads the SAME [`crate::render::effective_card_anchor`] owner the
    /// summon freeze does, so a keyboard crossing and a fresh summon into the same
    /// world resolve to the identical rail.
    pub fn reanchor(&mut self) {
        if self.kind == OverlayKind::Theme {
            self.align = crate::render::effective_card_anchor();
        }
    }

    /// The corpus index currently highlighted (into `corpus`/`git`/`is_dir`), or
    /// `None` when no item matches.
    pub fn selected_corpus_index(&self) -> Option<usize> {
        self.items.get(self.selected).copied()
    }

    /// The document LINE the highlighted HEADING row jumps to (Go-to's Headings lens),
    /// or `None` when no item matches / the row carries no line. Read only when the
    /// highlighted row IS a heading ([`Self::selected_is_heading`]).
    pub fn selected_line(&self) -> Option<usize> {
        self.selected_corpus_index()
            .and_then(|i| self.lines.get(i).copied())
    }

    /// True when the highlighted Go-to row is a document HEADING (the Headings lens),
    /// so the accept JUMPS to [`Self::selected_line`] instead of opening a file. `false`
    /// for an ordinary file row and every non-Go-to picker (empty `heading` vec).
    pub fn selected_is_heading(&self) -> bool {
        self.selected_corpus_index()
            .map(|i| self.heading.get(i).copied().unwrap_or(false))
            .unwrap_or(false)
    }

    /// True when the highlighted Spell row is the appended "Add '<word>' to
    /// dictionary" affordance (`spell_add[i]`), so the accept ADDS the word to the
    /// personal dictionary instead of replacing it with a suggestion. `false` for a
    /// suggestion row and every non-Spell picker (empty `spell_add`). The word to
    /// add is [`Self::add_word`].
    pub fn selected_is_add_to_dictionary(&self) -> bool {
        self.selected_corpus_index()
            .map(|i| self.spell_add.get(i).copied().unwrap_or(false))
            .unwrap_or(false)
    }

    /// The RESTORE id of the highlighted history row (History only), or `None` when
    /// no item matches / this isn't a history picker / an empty history (no rows to
    /// restore). Enter maps this to a restore.
    pub fn selected_history_id(&self) -> Option<&str> {
        self.selected_corpus_index()
            .and_then(|i| self.history_ids.get(i))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// The caret LOOK the highlighted row selects (Caret picker only), or `None`
    /// when no item matches or this isn't a caret picker. Maps the highlighted row's
    /// label back to its [`crate::caret::CaretMode`] via [`CaretMode::from_label`].
    pub fn selected_caret_mode(&self) -> Option<crate::caret::CaretMode> {
        if self.kind != OverlayKind::Caret {
            return None;
        }
        self.selected_value()
            .and_then(crate::caret::CaretMode::from_label)
    }

    /// The RAW corpus string currently highlighted (the accept value), or `None`
    /// when no item matches.
    pub fn selected_value(&self) -> Option<&str> {
        self.selected_corpus_index().map(|i| self.corpus[i].as_str())
    }

    /// True when the highlighted entry is a directory (Browse: Enter descends).
    pub fn selected_is_dir(&self) -> bool {
        self.selected_corpus_index()
            .map(|i| self.is_dir[i])
            .unwrap_or(false)
    }

    /// The DISPLAY string for corpus entry `i`: the raw value plus a trailing
    /// `/` for a directory. A git repo is marked NOT here but by a dim `"git"` tag
    /// in the row's SECONDARY (right) column (see [`Self::item_git_tags`]), so the
    /// primary cell stays the clean folder name; the accept value is always the raw
    /// corpus string.
    fn display_of(&self, i: usize) -> String {
        // ASSET CLEANER: the corpus holds the root-relative PATH (the accept/trash key
        // + fuzzy corpus, so typing a folder narrows), but the primary cell shows just
        // the leaf FILE NAME — its parent dir rides the secondary column. Every other
        // picker displays its raw corpus value.
        if self.kind == OverlayKind::Assets {
            let rel = &self.corpus[i];
            return rel.rsplit('/').next().unwrap_or(rel).to_string();
        }
        // THE UNION ROUND: a settings row (appended to the Command palette's
        // corpus by `attach_settings_rows`) draws the `§ ` marker glyph before its
        // name — `crate::overlay::row_split` recognizes the SAME prefix constant
        // and mutes it, exactly like a file row's directory prefix.
        if self.kind == OverlayKind::Command && self.is_setting.get(i).copied().unwrap_or(false) {
            return format!("{}{}", OverlayKind::SETTINGS_MARKER_PREFIX, self.corpus[i]);
        }
        // ITEM 11's UNIFIED LIST: a Go-to HEADING row (appended after the file rows
        // by `attach_headings`) draws the `❡ ` marker glyph before its (already
        // depth-indented) title — the mirror-image of the settings marker above —
        // so it reads apart from a file row at a glance once the default `All` list
        // mixes both kinds together.
        if self.kind == OverlayKind::Goto && self.heading.get(i).copied().unwrap_or(false) {
            return format!("{}{}", OverlayKind::HEADING_MARKER_PREFIX, self.corpus[i]);
        }
        let mut s = self.corpus[i].clone();
        if self.is_dir.get(i).copied().unwrap_or(false) {
            s.push('/');
        }
        s
    }

    /// The filtered DISPLAY strings, top-to-bottom (for rendering AND the
    /// sidecar). Directories carry a trailing `/`; a git repo's marker rides the
    /// SECONDARY column ([`Self::item_git_tags`]), not the name.
    pub fn item_strings(&self) -> Vec<String> {
        self.items.iter().map(|&i| self.display_of(i)).collect()
    }

    /// The filtered git-repo TAGS, in the same row order as [`Self::item_strings`]:
    /// a dim `"git"` for a row that is itself a git repo, `""` otherwise. This is
    /// the Project / Browse pickers' SECONDARY (right) column — the same recessive
    /// column the command palette uses for chords and go-to for edit times, so the
    /// tag YIELDS first under width pressure ([`crate::render::rowlayout`]). Returns
    /// an EMPTY vec when NO row is a git repo, so a git-free listing keeps no
    /// secondary column at all (byte-identical to a plain picker). For a picker kind
    /// that never marks git (theme / command / …) every flag is false → empty vec.
    pub fn item_git_tags(&self) -> Vec<String> {
        if !self.items.iter().any(|&i| self.git.get(i).copied().unwrap_or(false)) {
            return Vec::new();
        }
        self.items
            .iter()
            .map(|&i| {
                if self.git.get(i).copied().unwrap_or(false) {
                    "git".to_string()
                } else {
                    String::new()
                }
            })
            .collect()
    }

    /// The calm EMPTY-STATE line to show when NO rows match — a QUERY that filtered
    /// everything out reads the universal "no matches"; an empty CORPUS reads the
    /// per-kind [`OverlayKind::empty_corpus_message`] ("no history yet", "no
    /// suggestions", …). The ONE owner of the empty-state text, shared by the render
    /// message row AND the sidecar `overlay.empty` field so pixels + sidecar agree.
    pub fn empty_message(&self) -> String {
        if !self.query.is_empty() {
            return "no matches".to_string();
        }
        // A REFINEMENT lens (a strip index past the flat `All` home) that filtered
        // the corpus to empty reads its own calm line — e.g. the Go-to Recent lens's
        // "no recent files yet" — distinct from a genuinely empty corpus.
        if let Some(lens) = self.active_facet_id() {
            if let Some(msg) = self.kind.empty_lens_message(lens) {
                return msg.to_string();
            }
        }
        self.kind.empty_corpus_message().to_string()
    }

    /// The empty-state message to DRAW, or `None` when the picker has rows. `Some`
    /// exactly when `items` is empty — the render path then draws one dim,
    /// non-selectable message row (styled like the foot hint), and since `items` is
    /// empty every accept (`selected_value`/`selected_corpus_index`) already returns
    /// `None`, so Enter on the empty state is a calm no-op with no extra guard.
    pub fn empty_notice(&self) -> Option<String> {
        if self.items.is_empty() {
            Some(self.empty_message())
        } else {
            None
        }
    }

    /// The filtered BINDING labels, in the same row order as [`item_strings`]
    /// (Command palette only; empty/blank for every other kind). Lets the render
    /// + sidecar show each command's chord beside its name without re-deriving it.
    pub fn item_bindings(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|&i| self.bindings.get(i).cloned().unwrap_or_default())
            .collect()
    }

    /// The filtered relative-time LABELS, in the same row order as [`item_strings`]
    /// (go-to picker only; empty for every other kind and in headless capture). A
    /// HEADING row (see [`Self::heading`]) carries no mtime — since item 11's unified
    /// `All` list mixes heading rows in among file rows, its cell reads the constant
    /// `"heading"` KIND HINT instead, the rowlayout SECONDARY-cell disambiguator that
    /// tells a heading row apart from a file row at a glance (a file row's cell is
    /// its relative edit time live, or blank in headless where mtime is never read).
    pub fn item_times(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|&i| {
                if self.heading.get(i).copied().unwrap_or(false) {
                    "heading".to_string()
                } else {
                    self.times.get(i).cloned().unwrap_or_default()
                }
            })
            .collect()
    }
}

/// Remove the trailing word (its preceding non-word run + the word itself) from a
/// minibuffer input `s`, in place — the ⌥⌫ word-delete shared by EVERY overlay
/// input (the fuzzy query + the Rename / Link / Keep / Settings-value edits).
/// Routes through the document buffer's ONE word-delete boundary owner
/// ([`crate::buffer::word_delete_backward_boundary`]) so the minibuffer can never
/// disagree with the text about where a word ends. A NO-OP on an empty string.
pub(super) fn truncate_trailing_word(s: &mut String) {
    let chars: Vec<char> = s.chars().collect();
    let keep = crate::buffer::word_delete_backward_boundary(chars.len(), |i| chars[i]);
    if keep < chars.len() {
        *s = chars[..keep].iter().collect();
    }
}
