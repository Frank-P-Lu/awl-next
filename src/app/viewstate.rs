//! VIEW SNAPSHOT: build the [`ViewState`] from the current buffer + scroll +
//! zoom + selection + search + overlay and push it into the pipeline
//! ([`App::sync_view`]), plus anchoring the OS IME candidate window to the
//! caret. The one bridge from editor state to the render pipeline; lifted out
//! of `app.rs` verbatim.

use super::*;

impl App {
    /// Build the render snapshot from the current buffer + scroll + zoom +
    /// selection and push it into the pipeline. When `follow` is true (cursor
    /// moved / text edited), the scroll is clamped so the cursor stays on
    /// screen; when false (free wheel scroll), the scroll is left untouched so
    /// the viewport moves independently of the cursor.
    pub(super) fn sync_view(&mut self, follow: bool) {
        if self.gpu.is_none() {
            return;
        }
        // Any real view sync applies `self.zoom`, so it also consumes a queued
        // zoom reflow. Usually RedrawRequested took the gate immediately before
        // calling us; this second clear covers an unrelated input that needs a
        // sync before that redraw arrives.
        self.zoom_reflow.clear();
        let height = self.gpu.as_ref().unwrap().config.height as f32;
        let (cursor_line, cursor_col) = self.buffer.cursor_line_col();
        // Re-run spell detection only when the buffer text changed. We detect a
        // change via the cheap edit VERSION (a `u64` bump per content mutation)
        // instead of cloning + comparing the whole rope string each keystroke. The
        // preedit composition is deliberately NOT included, so composing text is
        // never flagged.
        //
        // EAGER (the completed-word-lag fix's first half): recompute SYNCHRONOUSLY,
        // right here, every time the version changed — no debounce window in which
        // an old verdict could paint over text it never judged. The former ~150ms
        // debounce existed so a word wasn't flagged while you were still typing it;
        // that job now belongs entirely to the DISPLAY-side caret-word suppression
        // (`word_at_caret` in `render/rects.rs`) — the word under the caret is
        // checked exactly as eagerly as every other word, just not SHOWN while the
        // caret sits in it. Every verdict is ALSO KEYED to the exact text it judged
        // (`spell::keyed`/`SpellVerdict::still_valid`, filtered below when building
        // `ViewState.misspelled`) as a second, independent guarantee: even within
        // this one call, a verdict this rescan is ABOUT to replace can never be
        // read as still describing the new text.
        if self.spell.is_some() && self.spell_checked_version != Some(self.buffer.version()) {
            self.recompute_spell_cache();
        }
        // SAVE-FEEDBACK round: the window-title EDITED marker + the native
        // macOS titlebar dot, kept live WITHOUT re-titling every keystroke —
        // `sync_view` already runs on nearly every edit/cursor-move (gated on
        // the gpu-present check above, the cheapest honest hook), so compare
        // against the cached `title_dirty` and only call `update_title` (a
        // string format + a `set_title`/`set_document_edited` OS call) on an
        // ACTUAL clean↔dirty flip.
        if self.is_document_dirty() != self.title_dirty {
            self.update_title();
        }
        // Schedule a debounced AUTO-SAVE for the active quick note when its text
        // changed. This lives ONLY here (the live windowed path, gated by the
        // gpu-present check above), so the headless capture/replay never auto-writes
        // — the determinism + no-fixture-mutation guarantee. The write fires in
        // `about_to_wait` after a quiet period.
        if self.buffer.is_note() && self.autosave_saved_version != Some(self.buffer.version()) {
            self.autosave_dirty_at = Some(self.clock.now());
        }
        // Arm the DOCUMENT AUTOSAVE idle timer (config-gated, default ON) when a
        // non-note buffer's text changed since its last write — a pathed document
        // tracks `doc_saved_version`, the no-path scratch its stash version.
        // Same determinism guarantee as the note arming above: this lives ONLY
        // under the gpu-present gate, so headless can never schedule a write.
        if self.config.autosave_on() && !self.buffer.is_note() {
            let unsaved = if self.buffer.path().is_some() {
                self.doc_saved_version != Some(self.buffer.version())
            } else {
                self.scratch_saved_version != Some(self.buffer.version())
            };
            if unsaved {
                self.doc_autosave_at = Some(self.clock.now());
            }
        }
        // DIFF-AS-PREVIEW: while the History picker is open, the page below the
        // card shows the WRITER'S DIFF of the current buffer vs the highlighted
        // row's version — derived here, at ViewState-build time, by overriding
        // the pushed text with the marked-up-manuscript transcript (the one owner
        // `crate::history::diff_preview`, cached per id). The BUFFER (its
        // content, version, undo history) is NEVER touched, so Esc just closes
        // the overlay and the next sync pushes the buffer's own text again —
        // "back to now exactly". `None` whenever the picker isn't open / the row
        // is the empty-state one. (The old plain-content preview and the separate
        // Compare takeover both retired into this one surface.)
        let preview = self.history_preview_text();
        // DIFF-AS-PREVIEW scroll: while the diff preview is up, the page shows the
        // OVERLAY's own `diff_scroll` (PgUp/PgDn / panel-focus ↑/↓ / the wheel over
        // the page all mutate it) — and `self.scroll_lines`, the DOCUMENT's
        // viewport, is deliberately never touched, so "Esc = back to now exactly"
        // includes the scroll by construction. Clamped against the shaped
        // transcript below (with the clamp written back, so the sidecar reports
        // the honest value).
        let diff_scroll = if preview.is_some() {
            self.overlay.as_ref().map(|o| o.diff_scroll)
        } else {
            None
        };
        // ROPE-CLONE SHORT-CIRCUIT: reuse the last materialised rope clone while the
        // buffer version is unchanged (see [`Self::view_text`]). A PREVIEW bypasses
        // `view_text` entirely — the version-keyed `sync_text_cache` must never hold
        // a previewed version's bytes (the cache-key discipline).
        let text = match &preview {
            Some(p) => p.clone(),
            None => self.view_text(),
        };
        // The follow branch chases the BUFFER cursor; a preview clamps that cursor
        // into a DIFFERENT text, so arrowing the rows must never scroll-chase it.
        let follow = follow && preview.is_none();

        // Did this sync follow a text EDIT? A bumped buffer version since the last
        // sync means the cursor moved because of typing/delete/paste/newline (vs.
        // pure navigation), so the caret slides as a plain block with no underline
        // however far it jumped (Enter, a wide glyph, a paste). Captured once per
        // sync so the re-push below reuses the same value.
        let version = self.buffer.version();
        // A delete-word edit DID bump the version, but its caret should still
        // streak like the equivalent navigation move (M-b): the removed word
        // collapses while the caret glides left across the gap, as ONE concurrent
        // motion. So when `caret_edit_streaks` was set for this sync, treat the
        // move as navigation (not an edit) for the underline-suppression test only.
        // One-shot: reset it so the next sync goes back to the default.
        let streak_override = std::mem::take(&mut self.caret_edit_streaks);
        let is_edit_move = version != self.caret_synced_version && !streak_override;
        self.caret_synced_version = version;
        // Was the keypress driving this sync an OS auto-repeat (a HELD arrow)?
        // One-shot, like `caret_edit_streaks`: consumed here so a following
        // non-keyboard sync (IME/wheel) doesn't inherit a stale held flag.
        let held = std::mem::take(&mut self.caret_held);

        // FORMAT POPOVER: recompute the lit/label model from the LIVE selection
        // each sync (so it reflects a format apply the instant it lands), gated on
        // the mouse-summoned flag + the config toggle + an actual selection + no
        // modal surface (overlay / search) owning the screen. A pure fn of the
        // buffer state (`actions::popover::plan`); `None` parks every popover quad.
        let popover = if self.popover_open
            && crate::popover::popover_on()
            && self.overlay.is_none()
            && self.search.is_none()
            && self.buffer.has_selection()
        {
            crate::actions::popover::plan(
                &self.buffer.text(),
                self.buffer.anchor_char(),
                self.buffer.cursor_char(),
                self.buffer.is_markdown(),
            )
        } else {
            None
        };

        // Map the active isearch state (if any) into render-facing fields: each
        // match CHAR range -> ((l,c),(l,c)) so highlight quads reuse the
        // selection-rect geometry; the current match is shown only by the real
        // amber caret (already moved onto it by handle_search_key).
        let (
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            search_replace_active,
            search_replacement,
            search_editing_replacement,
        ) = self.search_view_fields();

        // KEYED (the completed-word-lag fix's second half): filter the cache down
        // to the verdicts still valid against the text about to be pushed
        // (`spell::visible`, THE ONE reader) — a verdict whose exact word has
        // since changed underneath its span (an edit this same sync just made,
        // or — belt and suspenders — any future path that mutates `spell_cache`
        // without an immediate rescan) can never paint. A DIFF-AS-PREVIEW
        // substitutes a different transcript into `text` above; every verdict
        // stays keyed to the REAL buffer text, so key against that (a cached
        // `view_text()` clone, not a fresh rope walk) rather than the preview
        // transcript it would never match anyway.
        let misspelled = if preview.is_some() {
            let buffer_text = self.view_text();
            crate::spell::visible(&self.spell_cache, &buffer_text)
        } else {
            crate::spell::visible(&self.spell_cache, &text)
        };

        // Build the snapshot once and push it so the pipeline shapes the CURRENT
        // text/zoom. The scroll offset is counted in VISUAL ROWS; row geometry
        // (and thus the cursor's visual row + the document's total rows) does not
        // depend on the scroll value, so we can read those AFTER this first push
        // and only need to re-push if cursor-follow moves the scroll.
        let mut view = ViewState {
            text,
            cursor_line,
            cursor_col,
            // The caret's wrap affinity (Upstream only right after a visual line-END
            // motion) — the pipeline reads it to render the caret on the row it
            // visually belongs to at a shared soft-wrap boundary.
            caret_affinity: self.buffer.affinity(),
            scroll_lines: diff_scroll.unwrap_or(self.scroll_lines),
            zoom: self.zoom,
            selection: self.buffer.selection_line_col(),
            preedit: self.preedit.clone(),
            misspelled,
            is_edit_move,
            held,
            // DRAG-BAR: while a live text-selection drag is in progress the caret
            // melts to the thin insertion bar (see `ViewState::selecting_drag`),
            // returning to the configured look on release.
            selecting_drag: self.dragging,
            search_matches,
            search_current,
            search_query,
            search_active,
            search_case_sensitive,
            search_replace_active,
            search_replacement,
            search_editing_replacement,
            overlay_active: self.overlay.is_some(),
            // CRISP-BACKDROP exception: the THEME / CARET-STYLE / HISTORY pickers keep
            // the doc crisp behind them (live theme colours / caret preview / the
            // history version preview — the document IS the preview); every other
            // full overlay gets the frosted-blur backdrop.
            overlay_crisp: self
                .overlay
                .as_ref()
                .map(|o| {
                    matches!(
                        o.kind,
                        crate::overlay::OverlayKind::Theme
                            | crate::overlay::OverlayKind::Caret
                            | crate::overlay::OverlayKind::History
                    )
                })
                .unwrap_or(false),
            overlay_query: self
                .overlay
                .as_ref()
                .map(|o| o.query.clone())
                .unwrap_or_default(),
            overlay_title: self
                .overlay
                .as_ref()
                .filter(|o| o.kind.draws_title_prefix())
                .map(|o| o.kind.title())
                .unwrap_or(""),
            overlay_items: self
                .overlay
                .as_ref()
                .map(|o| o.item_strings())
                .unwrap_or_default(),
            // EMPTY STATE: the shared calm message when the overlay has no rows (empty
            // corpus / query matched nothing); `None` when there are rows or no overlay.
            overlay_empty: self.overlay.as_ref().and_then(|o| o.empty_notice()),
            overlay_bindings: self
                .overlay
                .as_ref()
                .map(|o| o.item_bindings())
                .unwrap_or_default(),
            overlay_times: self
                .overlay
                .as_ref()
                .map(|o| o.item_times())
                .unwrap_or_default(),
            overlay_git: self
                .overlay
                .as_ref()
                .map(|o| o.item_git_tags())
                .unwrap_or_default(),
            overlay_selected: self.overlay.as_ref().map(|o| o.selected).unwrap_or(0),
            overlay_scroll: self.overlay.as_ref().map(|o| o.scroll).unwrap_or(0),
            // The per-kind visible-row cap (8 spell / 12 flat+faceted / more for theme),
            // the ONE owner the pipeline windows against so the drawn rows match the
            // hover/keyboard item-window exactly.
            overlay_window_rows: self.overlay.as_ref().map(|o| o.window_rows()).unwrap_or(12),
            overlay_hint: self
                .overlay
                .as_ref()
                .map(|o| o.foot_hint())
                .unwrap_or_default(),
            // FACETED PICKERS (theme / go-to / browse): the lens strip + per-row
            // section labels (empty for every non-faceting kind, which renders flat).
            overlay_lens: self
                .overlay
                .as_ref()
                .map(|o| o.lens_strip())
                .unwrap_or_default(),
            overlay_sections: self
                .overlay
                .as_ref()
                .map(|o| o.item_sections())
                .unwrap_or_default(),
            // CARET-STYLE PICKER preview: while that picker is open, the look its
            // highlighted row selects (drives the live animated preview box). `None`
            // for every other state, so the preview loop runs ONLY while it is open.
            caret_preview: self
                .overlay
                .as_ref()
                .filter(|o| o.kind == crate::overlay::OverlayKind::Caret)
                .and_then(|o| o.selected_caret_mode()),
            // PAGE-MODE GUTTER: the buffer's display name (saved file name, or the
            // derived scratch/slug name for an unsaved note) over the project name.
            // (While the History picker's diff preview is up, the card itself names
            // the compared version — the gutter keeps its ordinary identity.)
            gutter_name: self.buffer.display_name(),
            gutter_project: self.project.name.clone(),
            // MARKDOWN STYLING gate: a buffer is "markdown" only once it has a
            // `.md`/`.markdown` path. An unnamed scratch / `.rs` / `.txt` buffer is
            // left untouched (no markup dimming of `#` comments etc.).
            is_markdown: self.buffer.is_markdown(),
            // INLINE IMAGES: the directory a relative `![alt](img.png)` path
            // resolves against — the open document's own parent dir (buffer path,
            // else the launch `file`). `None` for a no-path scratch/note buffer
            // (a relative image path then resolves against the process cwd).
            doc_dir: self
                .buffer
                .path()
                .or(self.file.as_deref())
                .and_then(|p| p.parent())
                .map(|d| d.to_path_buf()),
            syn_lang: self.buffer.syntax_lang(),
            // SPELL contextual panel: when the open overlay is the spell picker, its
            // target word span turns the overlay into a small floating panel anchored
            // at the word (no blur). `None` for every other overlay / no overlay.
            overlay_spell: self
                .overlay
                .as_ref()
                .filter(|o| o.kind == crate::overlay::OverlayKind::Spell)
                .and_then(|o| o.spell_target),
            // CALM NOTICE (live-only: today the autosave clobber guard). Empty
            // draws nothing — parked off-screen, like the empty word count.
            notice: self.notice.clone().unwrap_or_default(),
            // i18n: the Han-ambiguity tiebreak ladder, from the loaded config
            // (or the built-in default when absent/all-unrecognized).
            cjk_priority: self.config.cjk_priority_or_default(),
            // LINE ENDINGS: the active buffer's on-disk ending, for the held stats
            // HUD's LINE ENDINGS row (a pure buffer fact, not re-derivable from text).
            eol: self.buffer.eol(),
            // FORMAT POPOVER: the mouse-summoned format toolbar's model (computed
            // above), or `None` when down.
            popover,
            // DIFF-AS-PREVIEW: dress the page column as a CARD (border + elevation
            // + clipped content) while the History picker's diff preview is up;
            // the panel border strengthens one value step when Tab moved the
            // focus into it.
            diff_panel: preview.is_some(),
            diff_panel_focus: self
                .overlay
                .as_ref()
                .map(|o| o.diff_focus)
                .unwrap_or(false),
            // FOLDS: filled just below (with the buffer's fold set), after which the
            // hidden lines are dropped from `text` + coordinates remapped. Kept as
            // the conscious render decision this exhaustive site forces.
            folds: Vec::new(),
        };
        // HISTORY PREVIEW geometry safety: the pushed text is a DIFFERENT (possibly
        // shorter) version than the buffer, so every field whose line/col spans
        // index the BUFFER text must be re-bounded or cleared — the cursor clamps
        // into the previewed text (the shared `clamp_line_col`); selection /
        // preedit / squiggles / search highlights are dropped for the preview's
        // duration (they'd misalign, or panic in the glyph-span layer). All
        // restored automatically on close: the next sync rebuilds them from the
        // untouched buffer.
        if preview.is_some() {
            // DIFF-AS-PREVIEW: park the caret on the transcript's blank line 1
            // (between the `# title` and the first diff block) so NO line's WYSIWYG
            // conceal reveals — the reveal is caret-line-scoped and line 1 carries no
            // markup, so the title's `#` and every `==`/`>`/strike marker stay
            // concealed: the clean marked-up manuscript, never a revealed-raw line.
            // The ONE reveal-suppression rule, shared with the `AWL_DIFF_*` capture
            // harness (`main/run.rs` parks the same way), so live == capture.
            let (dl, dc) = crate::history::clamp_line_col(&view.text, 1, 0);
            view.cursor_line = dl;
            view.cursor_col = dc;
            view.selection = None;
            view.preedit = String::new();
            view.misspelled = Vec::new();
            view.search_matches = Vec::new();
            view.search_current = None;
            view.search_query = String::new();
            view.search_active = false;
            view.search_case_sensitive = false;
            view.search_replace_active = false;
            view.search_replacement = String::new();
            view.search_editing_replacement = false;
            // A history preview shows a DIFFERENT version's text; the popover's
            // spans would index the wrong bytes, so it never rides a preview frame.
            view.popover = None;
        }
        // FOLDS: collapse the folded sections out of the shaped text. `view.text`
        // is the full document; drop the hidden lines and remap the caret /
        // selection / search / spell coordinates into the filtered space the
        // pipeline shapes — a hidden line is never laid out, so it contributes ZERO
        // height. Recorded (unfiltered) for the sidecar. A no-op when nothing is
        // folded (byte-identical) and skipped during a history preview (its
        // substitute transcript owns the text). The action-seam auto-expand keeps
        // the caret + any selection on visible lines.
        view.folds = self.buffer.folds().iter().copied().collect();
        if preview.is_none() && self.buffer.has_folds() {
            crate::fold::apply_to_view(&mut view, &self.buffer.hidden_lines());
        }
        {
            let gpu = self.gpu.as_mut().unwrap();
            gpu.pipeline.set_view(&view);
        }

        // Cursor-follow (an edit / cursor move): adjust the VISUAL-ROW scroll so the
        // cursor's visual row sits in the viewport. TYPEWRITER SCROLL folds into
        // cursor-follow: while it is on, the cursor's row is CENTERED vertically (it
        // rests at the eye line); while it is off the minimal-adjust is kept EXACTLY
        // (only nudge the scroll enough to reveal the row). For a non-wrapped doc the
        // cursor's visual row == its logical line, so the off path is identical to the
        // previous logical-line cursor-follow.
        let prev_scroll = self.scroll_lines;
        if let Some(anchor) = self.zoom_anchor.take() {
            // ZOOM ANCHOR wins this sync: this `set_view` just reshaped to the newly
            // changed zoom, so re-solve the scroll that keeps the anchored document
            // point at its captured screen y (the ONE owner does the variable-row
            // math + clamp). Overrides cursor-follow — the anchored caret is on
            // screen by construction, and the off-screen fallback deliberately holds
            // the viewport centre rather than yanking to the caret.
            let pipeline = &self.gpu.as_ref().unwrap().pipeline;
            self.scroll_lines =
                pipeline.zoom_anchor_scroll(anchor.line, anchor.col, anchor.screen_y, height);
        } else if follow {
            let pipeline = &self.gpu.as_ref().unwrap().pipeline;
            // Affinity-aware so cursor-follow tracks the row the caret VISUALLY sits
            // on at a shared boundary (Upstream → the upper row).
            let cursor_row =
                pipeline.visual_row_of_aff(cursor_line, cursor_col, self.buffer.affinity());
            self.scroll_lines = match follow_scroll_strategy(
                crate::typewriter::typewriter_on(),
                self.dragging,
            ) {
                // Variable-row-height aware: scroll minimally so the cursor's row
                // (taller on a heading) is fully visible, summing real row heights.
                FollowScroll::ShowRow => {
                    pipeline.scroll_to_show_row(cursor_row, self.scroll_lines, height)
                }
                // TYPEWRITER: center the cursor's row (variable-height aware too).
                FollowScroll::CenterRow => pipeline.scroll_to_center_row(cursor_row, height),
                // A primary-button press is live: defer the recenter (leave the
                // scroll exactly where it is) rather than move the view under a
                // stationary pointer — see `follow_scroll_strategy`.
                FollowScroll::Deferred => self.scroll_lines,
            };
        }
        // Always keep scroll within document bounds (pixel-accurate "does it fit").
        let max = self.gpu.as_ref().unwrap().pipeline.max_scroll_rows(height);
        match diff_scroll {
            // DIFF-AS-PREVIEW: clamp the OVERLAY's diff scroll against the shaped
            // transcript and write the clamp back (state stays honest for the
            // sidecar + the next key). `self.scroll_lines` is untouched — the
            // document's own viewport survives the whole preview.
            Some(ds) => {
                let clamped = ds.min(max);
                if let Some(ov) = self.overlay.as_mut() {
                    ov.diff_scroll = clamped;
                }
                if view.scroll_lines != clamped {
                    view.scroll_lines = clamped;
                    self.gpu.as_mut().unwrap().pipeline.set_view(&view);
                }
            }
            None => {
                self.scroll_lines = self.scroll_lines.min(max);
                // Re-push only if the scroll actually changed (cheap; avoids a
                // redundant reshape on the common no-scroll-change path).
                if self.scroll_lines != prev_scroll {
                    view.scroll_lines = self.scroll_lines;
                    self.gpu.as_mut().unwrap().pipeline.set_view(&view);
                }
            }
        }
        // Keep the OS candidate window anchored to the (advance-aware) caret.
        self.update_ime_cursor_area();

        // Apply the one-shot caret IMPULSE queued by `apply` for this sync (edit
        // flinch / blocked-action recoil), AFTER the spring target is set above so it
        // rides on top and self-settles back to rest.
        self.apply_caret_impulses();

        // LIFETIME STATS: accumulate the caret's DOCUMENT-space travel now that the
        // pipeline's caret target reflects this sync's cursor. `sync_view` is the
        // one live bridge every caret move passes through; the hook adds distance
        // only when the logical cursor actually moved (never on a pure scroll /
        // re-layout), and no-ops when the odometer is off (config-gated inside).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_track_caret();

        // Push the odometer snapshot to the pipeline so a held HUD this frame reads
        // the current lifetime figures (live-only; a capture never calls `sync_view`,
        // so its odometer rows stay the "—" placeholder).
        #[cfg(not(target_arch = "wasm32"))]
        self.stats_sync_hud();

        // WRITING STREAKS: push the live year-view so a summoned card this frame
        // reads the real heatmap (live-only; a capture shows the placeholder).
        #[cfg(not(target_arch = "wasm32"))]
        self.streaks_sync_card();

        // NOTES VERBS round: push the SAVED stat's live state (dirty, or clean +
        // elapsed seconds since the last successful write) — live-only, mirroring
        // `stats_sync_hud` exactly.
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_hud_saved();

        // CHECK FOR UPDATES round: push the About card's "checked … ago" figure
        // (the LOCAL marker's elapsed time) — live-only, mirroring
        // `sync_hud_saved` exactly.
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_update_checked();

        // DISCOVERABILITY (phase 2): push the hold-⌘ peek's personalized rows + the
        // Keybindings footer's tips from the ledger (live-only; a capture never calls
        // `sync_view`, so the peek falls back to the starter six and the footer hides).
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_discoverability();
    }

    /// The document text for this sync — the ROPE-CLONE SHORT-CIRCUIT. `sync_view`
    /// runs on every cursor move / scroll / selection change, none of which bump the
    /// buffer version — yet each would otherwise walk the whole rope into a fresh
    /// `String`. Reuse the last clone (a memcpy) while the version is unchanged;
    /// re-materialise the rope only after a real edit. The resulting bytes are
    /// identical either way. The cache is keyed by the buffer VERSION alone, so a
    /// BUFFER SWAP (open / new note — a fresh buffer restarting at version 0) must
    /// drop it at the swap site (`load_path` / `new_note`): an un-edited previous
    /// buffer also sits at version 0, and its stale entry would otherwise be served
    /// as the NEW document's text (the live "open a file and nothing appears" bug).
    pub(super) fn view_text(&mut self) -> String {
        let text_version = self.buffer.version();
        match &self.sync_text_cache {
            Some((v, t)) if *v == text_version => t.clone(),
            _ => {
                let t = self.buffer.text();
                self.sync_text_cache = Some((text_version, t.clone()));
                t
            }
        }
    }

    /// DIFF-AS-PREVIEW: the History picker's live-preview TRANSCRIPT (the writer's
    /// diff of the current buffer vs the highlighted version — see the one owner
    /// [`crate::history::diff_preview`]), or `None` when no preview applies (other
    /// overlays / no overlay / the empty-state row / an unresolvable id — the
    /// document then just shows the buffer, a calm degrade). Rendered ONCE per id
    /// into the `history_preview` cache, so an arrow/hover/wheel burst re-diffs
    /// nothing. Reads only; the buffer is never touched.
    ///
    /// SYNCHRONOUS (no per-arrow debounce): the round's release perf probe measured
    /// ~1-2 ms per diff at SCOPE.md scale — the diff FOLDS unchanged regions, so the
    /// transcript stays tiny and the reshape stays cheap even against a large draft
    /// (~15 ms of compute at 6k lines, still well inside a single stepped selection).
    /// So no measured demand for the theme-font-style debounce; the cost is paid
    /// straight, and live == the deterministic headless capture (`main/run.rs`).
    pub(super) fn history_preview_text(&mut self) -> Option<String> {
        let ov = self
            .overlay
            .as_ref()
            .filter(|o| o.kind == crate::overlay::OverlayKind::History)?;
        let id = ov.selected_history_id()?.to_string();
        if let Some((cached_id, transcript)) = &self.history_preview {
            if *cached_id == id {
                return Some(transcript.clone());
            }
        }
        let current = self.view_text();
        let ov = self
            .overlay
            .as_ref()
            .filter(|o| o.kind == crate::overlay::OverlayKind::History)?;
        let (id, transcript, _counts) = crate::history::diff_preview(
            ov,
            self.buffer.path(),
            self.file.as_deref(),
            self.buffer.is_note(),
            &current,
        )?;
        self.history_preview = Some((id, transcript.clone()));
        Some(transcript)
    }

    /// Map the active isearch state (if any) into the render-facing snapshot fields:
    /// each match CHAR range -> ((l,c),(l,c)) so highlight quads reuse the
    /// selection-rect geometry; the current match is shown only by the real amber
    /// caret (already moved onto it by `handle_search_key`). `None` search -> empty.
    fn search_view_fields(
        &self,
    ) -> (
        Vec<((usize, usize), (usize, usize))>,
        Option<usize>,
        String,
        bool,
        bool,
        bool,
        String,
        bool,
    ) {
        if let Some(st) = self.search.as_ref() {
            let matches = st
                .matches()
                .iter()
                .map(|m| {
                    (
                        self.buffer.char_to_line_col(m.start),
                        self.buffer.char_to_line_col(m.end),
                    )
                })
                .collect();
            (
                matches,
                st.current_index(),
                st.query().to_string(),
                true,
                st.is_case_sensitive(),
                st.is_replace_active(),
                st.replacement().to_string(),
                st.is_editing_replacement(),
            )
        } else {
            (
                Vec::new(),
                None,
                String::new(),
                false,
                false,
                false,
                String::new(),
                false,
            )
        }
    }

    /// Apply the one-shot caret IMPULSE `apply` queued for this sync — the edit
    /// FLINCH (a successful typed char / delete / kill-line / Enter / copy) OR the
    /// blocked-action RECOIL — fired in EVERY caret look AFTER `sync_view` set the
    /// spring target, so it rides on top and the spring self-settles it back to
    /// rest. One-shot: cleared on consume. The caller already requested a redraw;
    /// the breathe loop plays it out.
    fn apply_caret_impulses(&mut self) {
        // Edit FLINCH: a SUCCESSFUL typed char / delete / kill-line / Enter / copy
        // flinches the visual caret (squash-pop + back-kick / inward squash / gulp /
        // landing / a gentle copy pulse — the last one ALSO brightens the selection
        // quad's own tint via the same `TextPipeline::copy_pulse` call).
        if let Some(imp) = self.caret_impact.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                match imp {
                    CaretImpact::Type => gpu.pipeline.caret_type_impact(),
                    CaretImpact::Delete => gpu.pipeline.caret_delete_squash(),
                    CaretImpact::Gulp => gpu.pipeline.caret_gulp(),
                    CaretImpact::Land => gpu.pipeline.caret_line_land(),
                    CaretImpact::Copy => gpu.pipeline.copy_pulse(),
                }
            }
        }
        // BLOCKED-ACTION RECOIL: a motion/scroll/undo/delete that couldn't proceed
        // bumps the visual caret away from the wall (every caret look).
        if let Some(dir) = self.caret_recoil.take() {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.pipeline.caret_recoil(dir);
            }
        }
    }

    /// Tell winit where the composition caret is (in physical pixels) so the
    /// platform IME floats its candidate list by the caret. Reads the pipeline's
    /// real caret rect (which already accounts for any active preedit end).
    pub(super) fn update_ime_cursor_area(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (x, y, w, h) = gpu.pipeline.caret_pixel_rect();
        gpu.window.set_ime_cursor_area(
            winit::dpi::PhysicalPosition::new(x as f64, y as f64),
            winit::dpi::PhysicalSize::new(w.max(1.0) as f64, h.max(1.0) as f64),
        );
    }
}

/// Which vertical-scroll strategy `sync_view`'s cursor-follow applies — a PURE
/// function of typewriter scroll + whether a primary-button press is currently
/// live, extracted so the DEFERRAL DECISION is unit-testable without a GPU pipeline.
///
/// The sticky TYPEWRITER SCROLL toggle (`crate::typewriter`) asks for the CENTERED
/// pin (the caret row rests at the eye line). When it is off, the minimal-adjust
/// cursor-follow (`ShowRow`) is kept EXACTLY, so a default (typewriter off) launch
/// is byte-identical.
///
/// The bug the `Deferred` arm exists to prevent: the typewriter recenter used to
/// fire on EVERY `sync_view` — including the one a mouse PRESS triggers (hit-test
/// -> place cursor -> sync). Recentering moves the document under a pointer that
/// hasn't moved, so the very next `CursorMoved` is read as a big relative drag ->
/// phantom selection -> recenters again: a runaway feedback loop ("scroll really
/// quickly"). The fix keeps the auto-jump (it's the point of typewriter scroll) but
/// never lets it move the view while a press is down; the deferred recenter applies
/// on release, since `MouseInput::Released` already calls `sync_view(true)` after
/// `dragging` flips back to `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FollowScroll {
    /// Typewriter scroll off: nudge minimally so the cursor's row stays visible
    /// (the byte-identical default cursor-follow).
    ShowRow,
    /// Typewriter scroll wants the caret row centered — the pin, applied normally.
    CenterRow,
    /// Centering would apply, but a primary-button press is live right now: defer
    /// it — leave the scroll exactly where it is. The view must never move under a
    /// stationary pointer.
    Deferred,
}

pub(super) fn follow_scroll_strategy(typewriter: bool, dragging: bool) -> FollowScroll {
    if !typewriter {
        FollowScroll::ShowRow
    } else if dragging {
        FollowScroll::Deferred
    } else {
        FollowScroll::CenterRow
    }
}

#[cfg(test)]
mod follow_scroll_tests {
    use super::*;

    #[test]
    fn typewriter_off_always_shows_row_regardless_of_dragging() {
        // Centering door closed: the drag/press state can't matter — always the
        // minimal-adjust cursor-follow (byte-identical default).
        assert_eq!(follow_scroll_strategy(false, false), FollowScroll::ShowRow);
        assert_eq!(follow_scroll_strategy(false, true), FollowScroll::ShowRow);
    }

    #[test]
    fn typewriter_on_centers_when_no_press_is_live() {
        assert_eq!(follow_scroll_strategy(true, false), FollowScroll::CenterRow);
    }

    #[test]
    fn centering_defers_the_recenter_while_a_press_is_live() {
        // THE REGRESSION THIS GUARDS: a mouse press must never move the view
        // underneath the stationary pointer. While `dragging` is true, centering
        // must defer rather than recenter — the caller then leaves `scroll_lines`
        // untouched (see `sync_view`'s `Deferred` arm).
        assert_eq!(follow_scroll_strategy(true, true), FollowScroll::Deferred);
    }
}
