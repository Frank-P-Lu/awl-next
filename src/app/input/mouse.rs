//! src/app/input/mouse.rs — the MOUSE input path: the pixel->char hit test,
//! click-count/drag-slop arming, left-press + text-selection drag, the
//! outline/link/overlay/panel/menu-bar click surfaces, right-click
//! spellcheck, the context-aware cursor icon, wheel scroll + wheel-zoom
//! (incl. the horizontal table-pan gesture), and
//! `WindowEvent::CursorMoved`/`MouseInput`/`MouseWheel` dispatch itself.
//! Split out of the former `app/input.rs` monolith (2026-07
//! code-organization pass); see `keys` for the keyboard/IME path and
//! `drags` for the page/image resize state machines (this file ARMS
//! those drags from `on_mouse_input`, but their own lifecycle lives there).

use crate::app::*;

impl App {
    /// Is the pointer on the page surface that owns direct document gestures?
    /// Reuses the renderer's column geometry: page margins (including the gutter
    /// and right readout) are orientation chrome, not clamped aliases for the first
    /// / last character on a row.
    fn pointer_over_writing_column(&self) -> bool {
        self.gpu
            .as_ref()
            .is_some_and(|gpu| gpu.pipeline.over_writing_column(self.cursor_px.0))
    }

    /// Map the current mouse pixel position to a buffer char index, accounting
    /// for scroll + zoom, then clamp to the document. Returns the char index.
    pub(in crate::app) fn hit_test_char(&self) -> usize {
        let (px, py) = self.cursor_px;
        // Advance-aware hit test: walk the REAL shaped glyph advances so a click
        // lands on the right glyph for mixed CJK + Latin lines. Falls back to the
        // fixed-pitch free function only if the pipeline is not yet up.
        let (line, col) = match self.gpu.as_ref() {
            Some(gpu) => gpu.pipeline.hit_test(px, py, self.scroll_lines),
            None => render::hit_test(
                px,
                py,
                self.scroll_lines,
                &render::Metrics::with_dpi(self.zoom, self.dpi),
                render::TEXT_LEFT,
            ),
        };
        self.buffer.line_col_to_char(line, col)
    }

    /// Multi-click detection: same spot, within the time window (`MULTICLICK_MS`) —
    /// bump the running click count (wrapping 1/2/3) and stamp `last_click_time` /
    /// `last_click_px` for the NEXT press, then return the now-current count.
    /// Shared by a normal document press ([`Self::on_press`]) and a press on the
    /// draggable page-column edge ([`Self::begin_page_resize_if_hovering`]) so a
    /// double-click reads the same wherever the pointer lands — one owner, so the
    /// two can't drift apart on what counts as "a double-click".
    pub(in crate::app) fn bump_click_count(&mut self) -> u32 {
        let now = Instant::now();
        let near = {
            let (lx, ly) = self.last_click_px;
            (self.cursor_px.0 - lx).abs() < 4.0 && (self.cursor_px.1 - ly).abs() < 4.0
        };
        let recent = self
            .last_click_time
            .map(|t| now.duration_since(t) < Duration::from_millis(MULTICLICK_MS))
            .unwrap_or(false);
        self.click_count = if recent && near { (self.click_count % 3) + 1 } else { 1 };
        self.last_click_time = Some(now);
        self.last_click_px = self.cursor_px;
        self.click_count
    }

    /// THE PHANTOM-SELECTION-CLICK FIX: whether the pointer has traveled far
    /// enough from the press position (`press`) to the current position
    /// (`current`, both PHYSICAL px like `cursor_px`) to treat this `CursorMoved`
    /// as the start of a REAL text-selection drag, rather than pointer jitter or a
    /// WYSIWYG reveal reflow relocating glyphs under an otherwise-stationary
    /// pointer (concealed markup regaining its real advance the instant the caret
    /// lands on that line — which used to look identical to a drag because the old
    /// code re-hit-tested on every move regardless of actual travel, so the
    /// hit-test RESULT drifting was mistaken for pointer motion). Pure
    /// squared-distance compare against [`DRAG_ARM_SLOP_PX`] (no sqrt needed).
    /// Deliberately answers ONLY from pixel geometry — never from a hit-test
    /// result — so it can never be fooled by content reflowing under a still
    /// pointer. See `App::drag_armed`'s doc in `app.rs` for the wiring.
    pub(in crate::app) fn exceeds_drag_slop(press: (f32, f32), current: (f32, f32)) -> bool {
        let dx = current.0 - press.0;
        let dy = current.1 - press.1;
        dx * dx + dy * dy > DRAG_ARM_SLOP_PX * DRAG_ARM_SLOP_PX
    }

    /// Handle a primary-button press inside the writing column: hit-test, set the
    /// anchor, and (for double / triple clicks) select the word / line under the
    /// cursor. A press in either PAGE MARGIN is swallowed before hit-testing — the
    /// gutter is orientation, not document text, and `hit_test` deliberately clamps
    /// out-of-column x positions to a line endpoint. Without this gate, clicking the
    /// gutter therefore selected text at the page's left edge. A drag that STARTED in
    /// the column may still extend into a margin through [`Self::on_drag`].
    ///
    /// `shift` is
    /// whether Shift was held at press time: a SHIFT-CLICK extends the existing
    /// selection (the standard gesture everywhere — TextEdit/Xcode/browsers/…)
    /// instead of starting a fresh one, so it must never `clear_mark`.
    pub(in crate::app) fn on_press(&mut self, shift: bool, over_writing_column: bool) {
        if !over_writing_column {
            return;
        }
        let click_count = self.bump_click_count();
        // A click is a non-edit gesture: seal the open undo group so text typed
        // after relocating the cursor is its own undo step.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = true;
        // Fresh gesture: neither armed nor traveled yet. `drag_press_px` anchors
        // the slop measurement `on_cursor_moved` runs on every subsequent move —
        // reset on EVERY press (including a repeated same-spot multi-click), so
        // each gesture measures its own travel from its own press point. See
        // `exceeds_drag_slop` / the phantom-selection-click fix.
        self.drag_press_px = self.cursor_px;
        self.drag_armed = false;
        match click_count {
            1 if shift => {
                // SHIFT-CLICK: keep the mark if one is already active, else drop
                // it at the cursor's CURRENT position (before this click moves
                // it) — then move only the cursor to the hit point. Never
                // `clear_mark`; that's what a plain click is for. Double/triple
                // click arms are unaffected (shift only modifies the single-click
                // arm — a shift+double-click still lands here as click_count 1
                // relative to the NEW spot, since a shift-click is usually a
                // fresh spot rather than a same-spot repeat).
                self.drag_granularity = DragGranularity::Char;
                if self.buffer.anchor_char().is_none() {
                    self.buffer.set_anchor(self.buffer.cursor_char());
                }
                self.buffer.set_cursor(idx);
                self.shift_selecting = true;
            }
            1 => {
                // Single click: place the cursor, clear any selection.
                self.drag_granularity = DragGranularity::Char;
                self.buffer.set_cursor(idx);
                self.buffer.clear_mark();
                self.buffer.set_anchor(idx);
                self.shift_selecting = false;
            }
            2 => {
                // Double click: select the word under the cursor.
                self.drag_granularity = DragGranularity::Word;
                let (s, e) = self.buffer.word_bounds(idx);
                self.buffer.select_range(s, e);
            }
            _ => {
                // Triple click: select the whole line.
                self.drag_granularity = DragGranularity::Line;
                let (s, e) = self.buffer.line_bounds(idx);
                self.buffer.select_range(s, e);
            }
        }
    }

    /// CLICK-TO-JUMP on a persistent MARGIN OUTLINE row: hit-test the pointer against
    /// the outline's OWN row geometry (`TextPipeline::outline_hit_line`, which folds in
    /// the whole shown/hidden gate — off / non-page / non-md / too-narrow all return
    /// `None`) and, on a hit, jump the caret to that heading's line — the same
    /// `jump_to_line` the retired summoned Outline picker used. Returns whether the
    /// press landed on a row (so the caller skips the document press). A benign,
    /// user-approved navigation affordance (DESIGN.md outline amendment: "click-to-jump
    /// only") — NOT a resizable/focusable sidebar. Never fires while an overlay is open
    /// (its scrim owns the click first, handled upstream in `on_mouse_input`).
    pub(in crate::app) fn outline_click(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        let line = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.outline_hit_line(px, py, g.config.height));
        if let Some(line) = line {
            self.jump_to_line(&line.to_string());
            true
        } else {
            false
        }
    }

    /// CMD-CLICK follow-link: hit-test the char under the pointer and, if a markdown
    /// link sits there, hand its URL to the OS browser through the SAME
    /// [`App::follow_link`] owner the `C-c C-o` keyboard path uses (so the two can't
    /// drift). Returns whether a link was followed, so the caller can SWALLOW the
    /// press — never moving the caret / starting a selection. Reads only. The
    /// mouse-affordance half of the identity round's "⌘-click Follow link" (the
    /// keyboard chord stays too).
    pub(in crate::app) fn follow_link_at_pointer(&self) -> bool {
        let byte = self.buffer.char_to_byte(self.hit_test_char());
        if let Some(url) = crate::markdown::link_at(&self.buffer.text(), byte) {
            self.follow_link(&url);
            true
        } else {
            false
        }
    }

    /// A pointer HOVER over an open picker: hit-test the row under the cursor and move
    /// the selection onto it — the mouse twin of an arrow-key move. It applies the SAME
    /// live preview a keyboard move does (`actions::preview_overlay`: the Theme picker
    /// re-tints to the hovered world, the Caret picker swaps the look; every flat picker
    /// is inert), so hovering previews exactly like arrowing. A calm no-op when the
    /// pointer is off the rows or already on the highlighted one. Uniform across EVERY
    /// picker kind — the row geometry comes from the one `overlay_row_at` hit-test.
    pub(in crate::app) fn overlay_hover(&mut self) {
        let hit = self
            .gpu
            .as_ref()
            .and_then(|g| g.pipeline.overlay_row_at(self.cursor_px.0, self.cursor_px.1));
        let Some(idx) = hit else { return };
        // Re-highlight ONLY the row genuinely under the pointer AMONG THE VISIBLE ROWS.
        // `hover_select` never moves the scroll window (and rejects a row outside the
        // visible band / already-selected), so hovering the top/bottom edge can't make
        // the list auto-scroll — a hover highlights, it never scrolls.
        let kind = match self.overlay.as_mut() {
            Some(ov) => {
                if !ov.hover_select(idx) {
                    return;
                }
                ov.kind
            }
            None => return,
        };
        // LIVE PREVIEW, identical to the keyboard nav path.
        if let Some(ov) = self.overlay.as_ref() {
            crate::actions::preview_overlay(ov);
        }
        // A Theme preview mutated the process-global active world: re-tint the baked GPU
        // pipelines + window title so the hover previews it live, mirroring the theme
        // branch of `post_apply_effects` — colors instantly, the font reshape deferred
        // to the settle (`retint_theme_preview`), so sweeping the pointer down the
        // list costs one recolor per row, not one reshape storm per row.
        if kind == crate::overlay::OverlayKind::Theme {
            self.retint_theme_preview();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// The mouse WHEEL while a picker is OPEN: it OWNS the wheel (the doc behind it does
    /// NOT scroll), advancing the SELECTION like ↑/↓ — wheel DOWN moves the highlight
    /// down, wheel UP moves it up — and the scroll window follows (`move_sel`). `lines` is
    /// the wheel delta in rows (positive = wheel up); a fractional notch rounds. Applies
    /// the same LIVE PREVIEW the keyboard nav does, so wheeling the Theme picker previews
    /// each world exactly like arrowing.
    pub(in crate::app) fn overlay_wheel(&mut self, lines: f32) {
        let delta = -(lines.round() as isize); // wheel DOWN (lines < 0) advances (↓)
        if delta == 0 {
            return;
        }
        let kind = match self.overlay.as_mut() {
            Some(ov) => {
                ov.move_sel(delta);
                ov.kind
            }
            None => return,
        };
        if let Some(ov) = self.overlay.as_ref() {
            crate::actions::preview_overlay(ov);
        }
        if kind == crate::overlay::OverlayKind::Theme {
            // Wheel preview: colors now, font reshape on settle (see overlay_hover).
            self.retint_theme_preview();
        }
        self.sync_view(false);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// A LEFT-CLICK while a picker is open, resolved against the overlay card:
    ///   * ON a candidate ROW → move the selection there and ACCEPT it — the exact
    ///     `Action::Newline` the keyboard's Enter runs, so a click opens the file /
    ///     runs the command / commits the theme / descends the folder identically
    ///     (one path, every kind).
    ///   * OUTSIDE the card rect → DISMISS the overlay, routed through the SAME
    ///     `Action::Cancel` Esc / C-g uses (so a Theme / Caret live preview reverts
    ///     too). Click-away-to-dismiss is GENERAL across every summoned overlay
    ///     (palette / pickers / spell / history / …) — the card rect + row hit-test
    ///     both come from the one kind-agnostic `overlay_geometry`.
    ///   * INSIDE the card but off a row (query line / foot hint) → SWALLOWED (the
    ///     picker stays modal; it never falls through to `on_press`, which would place
    ///     the document cursor beneath the card).
    /// Always consumes the click while an overlay is open.
    pub(in crate::app) fn overlay_click(&mut self, event_loop: &ActiveEventLoop) {
        let (px, py) = self.cursor_px;
        let (row_hit, lens_hit, card) = self
            .gpu
            .as_ref()
            .map(|g| {
                (
                    g.pipeline.overlay_row_at(px, py),
                    g.pipeline.overlay_lens_at(px, py),
                    g.pipeline.overlay_card_rect(),
                )
            })
            .unwrap_or((None, None, None));

        // FACETED PICKER: a click on a LENS label switches the facet (keeping the
        // selection), then previews + re-tints — the pointing counterpart to LEFT/RIGHT.
        // Handled before the row hit-test (the strip sits above the rows, never overlaps).
        if let Some(lens_idx) = lens_hit {
            if let Some(ov) = self.overlay.as_mut() {
                ov.set_facet_lens(lens_idx);
            }
            if let Some(ov) = self.overlay.as_ref() {
                crate::actions::preview_overlay(ov);
            }
            // Lens-click preview: colors now, font reshape on settle (see overlay_hover).
            self.retint_theme_preview();
            self.sync_view(false);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }

        if let Some(idx) = row_hit {
            // ON a row: ACCEPT through the shared apply path — byte-for-byte the same
            // as Enter on the highlighted row (open / run / commit / descend / replace).
            if let Some(ov) = self.overlay.as_mut() {
                if idx < ov.items.len() {
                    ov.selected = idx;
                }
            }
            // A row-click accept dispatches a plain `Newline` (not a catalog command),
            // so the ledger door is inert here; a direct gesture is the fast path.
            self.apply(Action::Newline, false, event_loop, crate::stats::Door::Chord);
        } else {
            // Off the rows. A click INSIDE the card (query line / foot hint) is
            // swallowed to keep the picker modal; a click OUTSIDE the card dismisses it.
            let inside = card
                .map(|[x, y, w, h]| px >= x && px <= x + w && py >= y && py <= y + h)
                .unwrap_or(false);
            if inside {
                return;
            }
            self.apply(Action::Cancel, false, event_loop, crate::stats::Door::Chord);
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// A LEFT-CLICK inside the summoned find/replace panel: CLICK-TO-SWITCH-FIELD
    /// (plus the `Aa` case toggle). A press on the `Aa` cell flips case sensitivity
    /// and re-anchors the caret; a press on the FIND row (off `Aa`) focuses the
    /// query (`editing_replacement = false`); a press on the REPLACE row focuses
    /// the replacement (`editing_replacement = true`) — the amber caret then rides
    /// the clicked field (Batch-1 fixed the replace caret-x, so focusing via click
    /// places it correctly). A press ELSEWHERE inside the card (the key-hint line,
    /// inter-row gaps) is a calm no-op — swallowed, never dismissing the search or
    /// moving the document cursor beneath the panel. Returns `true` when the press landed on/in the panel and
    /// was handled; `false` (off the card / panel down) lets the caller fall
    /// through to the normal document press. The find↔replace decision is the pure
    /// `TextPipeline::panel_hit` (unit-tested); this only wires the field state +
    /// redraw, mirroring the two focus doors `handle_search_key` already uses.
    pub(in crate::app) fn panel_click(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        let hit = self.gpu.as_ref().and_then(|g| g.pipeline.panel_hit(px, py));
        match hit {
            // A press on the `Aa` cell toggles case sensitivity + re-anchors the
            // caret on the recomputed current match — the CLICK driver for the
            // affordance whose only keyboard door is ⌘⌥C (bare ⌥c composes to 'ç'
            // on macOS). Recompute needs the haystack, so read it before borrowing.
            Some(crate::render::PanelHit::CaseToggle) => {
                let hay = self.buffer.text();
                let target = self.search.as_mut().map(|st| {
                    st.toggle_case(&hay);
                    st.current_match()
                });
                if let Some(Some(m)) = target {
                    self.buffer.set_cursor(m.start);
                }
            }
            Some(crate::render::PanelHit::Find) => {
                if let Some(st) = self.search.as_mut() {
                    st.focus_query();
                }
            }
            Some(crate::render::PanelHit::Replace) => {
                if let Some(st) = self.search.as_mut() {
                    st.focus_replacement();
                }
            }
            // In the card but off an editable row: swallow (a calm no-op).
            Some(crate::render::PanelHit::Elsewhere) => {}
            // Off the panel: let the press fall through to the document.
            None => return false,
        }
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
        true
    }

    /// WEB/LINUX MENU BAR press handling — the click half of the awl-rendered menu bar
    /// (`menubar.rs` + `render/chrome/menubar.rs`). Returns `true` when it CLAIMED the
    /// press (the caller then repaints + swallows it), `false` to fall through to the
    /// normal overlay/search/document chain. The design law (shared with the macOS
    /// NSMenu bar): an item fires its catalog `Action` through the SAME `App::apply`
    /// seam a keypress uses — never new behaviour. Behaviour:
    ///   * a press on a clickable dropdown ITEM resolves + fires its `Action`, closes
    ///     the dropdown;
    ///   * a press on a TITLE toggles that menu's dropdown (re-click closes), and closes
    ///     any conflicting summoned overlay/search (the bar draws over them, and a
    ///     dropdown + overlay must not both own input);
    ///   * a press ANYWHERE else while a dropdown is open closes it (click-away);
    ///   * a press on the bar's dead strip (no dropdown) is swallowed, so it never moves
    ///     the caret in the document beneath the bar.
    pub(in crate::app) fn menubar_press(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if !crate::menubar::menu_bar_on() {
            return false;
        }
        let (px, py) = self.cursor_px;
        // Read the three hit-tests, then drop the pipeline borrow so `self.apply` can
        // take `&mut self` below.
        let (item_hit, title_hit, over_surface) = {
            let Some(gpu) = self.gpu.as_ref() else { return false };
            (
                gpu.pipeline.menubar_item_at(px, py),
                gpu.pipeline.menubar_title_at(px, py),
                gpu.pipeline.over_menu_surface(px, py),
            )
        };
        // 1. A clickable dropdown ITEM: resolve its catalog Action + fire it, then close.
        if let Some((menu, item)) = item_hit {
            crate::menubar::set_open(None);
            let action = {
                let menus = crate::menu::roster();
                menus.get(menu).and_then(|m| m.items.get(item)).and_then(|it| match it {
                    crate::menu::RosterItem::Routed { id, .. } => crate::menu::resolve(id),
                    // A Predefined item (Window ▸ Minimize/Zoom) has no catalog Action —
                    // an inert no-op in the awl-rendered bar (a v1 scope trim; a real
                    // winit minimize/maximize wiring is a follow-up).
                    _ => None,
                })
            };
            if let Some(action) = action {
                // MENU door (a slow discovery surface) — attributed to `Door::Menu` in
                // the usage ledger, exactly like the macOS NSMenu handler.
                let exited = self.apply(action, false, event_loop, crate::stats::Door::Menu);
                if exited {
                    return true;
                }
            }
            self.sync_view(true);
            return true;
        }
        // 2. A TITLE: toggle its dropdown; close any conflicting summoned surface.
        if let Some(i) = title_hit {
            crate::menubar::toggle_open(i);
            self.overlay = None;
            self.search = None;
            self.sync_view(true);
            return true;
        }
        // 3. A click AWAY while a dropdown is open: close it.
        if crate::menubar::open_menu().is_some() {
            crate::menubar::set_open(None);
            self.sync_view(true);
            return true;
        }
        // 4. A press on the bar's own dead strip: swallow (never a caret move beneath it).
        over_surface
    }

    /// Handle a SECONDARY-button (right-click) press: hit-test + place the cursor at
    /// the word under the pointer exactly like a single left-click (no drag, no
    /// selection), then summon the EXISTING spell-suggestion picker for that word.
    /// Misspelled → suggestions; otherwise `OpenSpellSuggest` no-ops (calm). Zero new
    /// spell logic — it reuses the same `suggest_at` path Cmd-`;` uses.
    pub(in crate::app) fn on_right_press(
        &mut self,
        event_loop: &ActiveEventLoop,
        over_writing_column: bool,
    ) {
        // RE-TARGET: a right press ALWAYS dismisses any open overlay FIRST (through the
        // same `Action::Cancel` Esc uses, so a Theme/Caret preview reverts), then hit-tests
        // the word now under the pointer and opens ITS suggestions. So right-clicking a
        // SECOND misspelling while the first spell menu is open swaps the menu to the new
        // word instead of being swallowed by the modal overlay.
        if self.overlay.is_some() {
            let _ = self.apply(Action::Cancel, false, event_loop, crate::stats::Door::Chord);
        }
        // A margin right-click may dismiss an open spell picker, but it never
        // retargets the caret/selection to the clamped edge of document text.
        if !over_writing_column {
            self.sync_view(true);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }
        // A click is a non-edit gesture: seal the open undo group first.
        self.buffer.seal_undo_group();
        let idx = self.hit_test_char();
        self.dragging = false;
        self.buffer.set_cursor(idx);
        self.buffer.clear_mark();
        self.buffer.set_anchor(idx);
        self.shift_selecting = false;
        // Fire the spell picker for the word now under the cursor (same Action the
        // Cmd-`;` chord runs, so the overlay + sidecar behave identically). A right-click
        // is a direct, learned gesture — the FAST path, not a discovery browse — so the
        // ledger attributes it to `Door::Chord` (see `crate::stats::Door`), never
        // inflating the slow-door count the discoverability surfacing keys on.
        let _ = self.apply(Action::OpenSpellSuggest, false, event_loop, crate::stats::Door::Chord);
        self.sync_view(true);
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// Handle mouse motion while the button is held: extend the selection to the
    /// current pixel position, by the drag's granularity (char/word/line).
    pub(in crate::app) fn on_drag(&mut self) {
        if !self.dragging {
            return;
        }
        let idx = self.hit_test_char();
        match self.drag_granularity {
            DragGranularity::Char => self.buffer.set_cursor(idx),
            DragGranularity::Word => {
                // Extend by whole words: keep the original anchor word, move the
                // cursor to the far edge of the word under the pointer.
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ws, we) = self.buffer.word_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(we);
                } else {
                    self.buffer.set_cursor(ws);
                }
            }
            DragGranularity::Line => {
                let anchor = self.buffer.anchor_char().unwrap_or(idx);
                let (ls, le) = self.buffer.line_bounds(idx);
                if idx >= anchor {
                    self.buffer.set_cursor(le);
                } else {
                    self.buffer.set_cursor(ls);
                }
            }
        }
    }

    /// LIVE-ONLY: recompute the CONTEXT-AWARE OS cursor shape (`cursor_shape.rs`) for
    /// the current mouse position + interaction state, and flip `Window::set_cursor`
    /// ONLY when it actually changed (`cursor_shape::cursor_icon_change` — no per-move
    /// winit chatter). Every context flag reads an EXISTING hit-test — `page_resizing`
    /// (the live drag flag), `self.overlay.is_some()`, `page_resize_hover` (the same
    /// proximity test the page-edge press/hover already uses), and
    /// `over_writing_column` (the same column bounds `page_resize_hover` reads) — so
    /// this never invents parallel geometry, it only arbitrates priority among the
    /// existing regions (`cursor_shape::cursor_icon_for`).
    ///
    /// Called on every `CursorMoved`, and again from the two doors that change this
    /// context WITHOUT any mouse motion: a page-edge drag beginning/ending
    /// (`begin_page_resize_if_hovering` / `end_page_resize`) and a summoned overlay
    /// opening/closing (`App::apply`'s one `self.overlay = overlay` assignment).
    ///
    /// COMPOSES with pointer auto-hide: while the OS pointer is `Hidden`
    /// (`pointer_hide::PointerHide`), the `set_cursor` call is skipped outright (there
    /// is nothing visible to update) and the cache is left untouched, so the very next
    /// un-hide — always a `CursorMoved`, which recomputes context before anything else
    /// — compares the fresh icon against the still-accurate cache and lands directly on
    /// the context-correct shape instead of a stale one from before the hide.
    pub(in crate::app) fn sync_cursor_icon(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let (px, py) = self.cursor_px;
        // The pointing-hand affordance now covers EVERY summoned picker's clickable
        // rows (Command-P / go-to / browse / theme / history / keybindings / spell /
        // …), not just spell — reuses the SAME kind-agnostic `overlay_row_at`
        // hit-test the pickers' own click handling uses (`overlay_click`), so a
        // hovered row can never disagree with a clickable one. `overlay_row_at`
        // already returns `None` off a row (the query line, foot hint, scrim, empty
        // gaps), so this lights up only on a real actionable row.
        let overlay_open = self.overlay.is_some();
        let over_clickable_overlay_row =
            overlay_open && gpu.pipeline.overlay_row_at(px, py).is_some();
        // A clickable LENS-STRIP facet (Time/Register/… of a FACETING picker) earns
        // the same pointing hand as a clickable row — reuses the SAME `overlay_lens_at`
        // hit-test the strip's click handling uses (`overlay_click`), so a hovered
        // facet can never disagree with a clickable one. `None` for a non-faceting
        // picker (no strip drawn) or off the strip row.
        let over_clickable_lens = overlay_open && gpu.pipeline.overlay_lens_at(px, py).is_some();
        // The overlay's editable query-filter line reads as a text field (I-beam) —
        // same `overlay_geometry` the field renders from, via `over_overlay_query`.
        let over_query_input = overlay_open && gpu.pipeline.over_overlay_query(px, py);
        // A clickable MARGIN-OUTLINE row reads as click-to-jump (the pointing hand),
        // reusing the outline's OWN row geometry (`outline_hit_line`, which folds in
        // the whole hidden/off gate). Only while no overlay is open — an overlay's
        // scrim covers the outline, so the outline never claims the hand behind it.
        let over_outline_row = !overlay_open
            && gpu
                .pipeline
                .outline_hit_line(px, py, gpu.config.height)
                .is_some();
        // An inline image's resize EDGE/CORNER reads as that handle's own glyph (↔
        // side, ↕ top/bottom, ⤡/⤢ corner), exactly like the page edge — reuses the
        // SAME `image_handle_at` hit-test the press path uses, over the SAME images
        // layout the `ImageQuadPipeline` draws (no parallel geometry). Only a hover
        // matters here (`.map(|(_, handle, _)| handle)`); the active-drag handle rides
        // `self.image_resizing`.
        let image_hover = gpu.pipeline.image_handle_at(px, py).map(|(_, handle, _)| handle);
        // WEB/LINUX MENU BAR: a clickable title / dropdown item earns the pointing
        // hand; dead bar/dropdown space reads as the plain arrow (over the doc it
        // covers). Both `false` when the bar is hidden (default off on macOS).
        let over_menu_hand = gpu.pipeline.menubar_hand_at(px, py);
        let over_menu_bar = gpu.pipeline.over_menu_surface(px, py);
        let ctx = crate::cursor_shape::CursorContext {
            dragging_edge: self.page_resizing,
            overlay_open,
            over_edge: gpu.pipeline.page_resize_hover(px),
            over_text: gpu.pipeline.over_writing_column(px),
            over_clickable_overlay_row,
            over_clickable_lens,
            over_query_input,
            over_outline_row,
            over_menu_hand,
            over_menu_bar,
            image_drag: self.image_resizing.map(|d| d.handle),
            image_hover,
        };
        let desired = crate::cursor_shape::cursor_icon_for(ctx);
        let hidden = self.pointer_hide == crate::pointer_hide::PointerHide::Hidden;
        if let Some(icon) = crate::cursor_shape::cursor_icon_change(self.cursor_icon, desired, hidden)
        {
            gpu.window.set_cursor(icon);
            self.cursor_icon = icon;
        }
    }

    /// Apply a wheel scroll of `lines` (positive = content moves up / scroll
    /// down). Free scroll: moves the viewport WITHOUT moving the cursor.
    pub(in crate::app) fn wheel_scroll(&mut self, lines: f32) {
        // The scroll unit is a VISUAL ROW. The wheel delta is already in rows
        // (line notches / accumulated pixels per row), so just clamp to the
        // document's total-visual-row max so a wrapped doc can scroll all the way
        // to its last visual row.
        let max = if let Some(gpu) = self.gpu.as_ref() {
            gpu.pipeline.max_scroll_rows(gpu.config.height as f32)
        } else {
            0
        };
        // Round toward the scroll direction so small notches still move.
        let delta = lines.round() as isize;
        let cur = self.scroll_lines as isize;
        let next = (cur + delta).clamp(0, max as isize);
        self.scroll_lines = next as usize;
    }

    /// `WindowEvent::CursorMoved`: track the pointer, un-hide the auto-hidden OS
    /// pointer, drive whichever pointer OWNER is active (overlay hover / live
    /// page-resize drag / text-selection drag), then recompute the context-aware
    /// cursor shape once for the move regardless of which branch fired.
    pub(in crate::app) fn on_cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        self.cursor_px = (position.x as f32, position.y as f32);
        // POINTER AUTO-HIDE: ANY mouse motion snaps back to Visible instantly —
        // cancels a pending typing-hide countdown and un-hides an already-hidden
        // pointer in the same move (`pointer_hide::on_mouse_move` is always
        // `-> Visible`). `os_visibility_change` decides whether that crossed the
        // hidden/visible boundary, so `set_cursor_visible` is only ever called on
        // an actual change.
        let prev_pointer_hide = self.pointer_hide;
        self.pointer_hide = crate::pointer_hide::on_mouse_move(prev_pointer_hide);
        if let Some(visible) =
            crate::pointer_hide::os_visibility_change(prev_pointer_hide, self.pointer_hide)
        {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.set_cursor_visible(visible);
            }
        }
        // A summoned picker OWNS the pointer (it is modal, the doc receding
        // behind it): a hover moves + previews the row under the cursor, exactly
        // like an arrow move. A live PAGE-WIDTH resize drag owns the pointer next
        // (the grabbed column edge tracks it, re-wrapping live); otherwise a live
        // text selection extends.
        if self.overlay.is_some() {
            self.overlay_hover();
        } else if self.page_resizing {
            self.on_page_resize_drag();
        } else if self.image_resizing.is_some() {
            // A live INLINE-IMAGE drag-resize owns the pointer: the image's width
            // tracks it (previewed live in pipeline state, no buffer edit yet).
            self.on_image_resize_drag();
        } else if self.dragging {
            // THE PHANTOM-SELECTION-CLICK FIX: only extend the selection once the
            // pointer has genuinely traveled past the drag-arm slop from the press
            // position (`exceeds_drag_slop`, pure pixel-distance geometry) — never
            // merely because the hit-test RESULT changed. A WYSIWYG reveal reflow
            // can relocate glyphs under an otherwise-stationary pointer between
            // press and release; without this gate that reflow alone used to read
            // as a real drag. `drag_armed` is sticky for the rest of the gesture
            // once tripped, so a fast real drag keeps extending normally.
            if !self.drag_armed {
                self.drag_armed = Self::exceeds_drag_slop(self.drag_press_px, self.cursor_px);
            }
            if self.drag_armed {
                self.on_drag();
                self.sync_view(true);
                if let Some(gpu) = self.gpu.as_ref() {
                    gpu.window.request_redraw();
                }
            }
        }
        // CONTEXT-AWARE CURSOR SHAPE: recompute on every move regardless of which
        // branch above fired (a text-selection drag still reads as "over text",
        // an overlay hover still reads as the plain arrow, …) — one decision, not
        // a per-branch special case. See `cursor_shape.rs`.
        self.sync_cursor_icon();
    }

    /// `WindowEvent::MouseInput`: the left/right press+release surface — input
    /// stamping, the summoned-about-card dismiss, right-click spell suggestions,
    /// and the left-button press/drag/resize/release state machine.
    pub(in crate::app) fn on_mouse_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        state: ElementState,
        button: MouseButton,
    ) {
        // DEBUG key→px: a mouse press is input awaiting pixels too — it
        // shares the request_redraw path (left falls through to it below;
        // right redraws inside `on_right_press`). Other buttons return
        // without a frame, so they are not stamped.
        if state == ElementState::Pressed && matches!(button, MouseButton::Left | MouseButton::Right)
        {
            self.stamp_input();
            // HOLD-⌘ SHORTCUT PEEK: a mouse press (a ⌘-click is Follow link) interrupts
            // the hold — cancel a pending peek / close an open one. Inert unless a peek
            // is pending/open.
            self.feed_peek(crate::peek::PeekStimulus::Interrupt);
        }
        // SUMMONED ABOUT / LIFETIME STATS CARDS: like `apply_core`'s own
        // top-of-function key intercept (`actions.rs`), ANY mouse press while
        // either modal card is open dismisses it and is otherwise fully swallowed
        // — never falls through to spell-suggest, an overlay click, or a document
        // press/selection. Routes through the SAME owner (`card::dismiss_summoned_card`)
        // apply_core uses, so the key and click paths can't drift. See `card.rs`.
        if state == ElementState::Pressed
            && matches!(button, MouseButton::Left | MouseButton::Right)
            && crate::card::dismiss_summoned_card()
        {
            self.sync_view(true);
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.window.request_redraw();
            }
            return;
        }
        // RIGHT-CLICK → spell suggestions: hit-test + place the cursor at the
        // word under the pointer (same hit_test as a left-click), then fire the
        // EXISTING spell-suggestion picker. On a misspelled word it lists
        // corrections; elsewhere it's a calm no-op. Reuses suggest_at /
        // OpenSpellSuggest wholesale — no new spell logic.
        if button == MouseButton::Right {
            if state == ElementState::Pressed {
                let over_writing_column = self.pointer_over_writing_column();
                self.on_right_press(event_loop, over_writing_column);
            }
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        match state {
            ElementState::Pressed => {
                // WEB/LINUX MENU BAR owns a press on its strip / open dropdown FIRST — a
                // title toggles its menu, an item fires its Action (through the SAME
                // apply seam), a click-away closes it — before the overlay/search/
                // document chain, since the bar draws OVER them. Returns true when it
                // claimed the press (then repaint + swallow). Inert when the bar is off.
                if self.menubar_press(event_loop) {
                    self.sync_cursor_icon();
                    if let Some(gpu) = self.gpu.as_ref() {
                        gpu.window.request_redraw();
                    }
                    return;
                }
                // CMD-CLICK → follow link: a Super-held left press on a markdown link
                // opens it in the browser (the mouse twin of C-c C-o), swallowing the
                // click so it never moves the caret / starts a selection. Off a link
                // it falls through to the normal press. Only on the bare document — a
                // summoned picker / search panel owns the click first (the chain
                // below), so this is gated on neither being open.
                if self.mods.state().contains(ModifiersState::SUPER)
                    && self.overlay.is_none()
                    && self.search.is_none()
                    && self.pointer_over_writing_column()
                    && self.follow_link_at_pointer()
                {
                    return;
                }
                // A summoned picker OWNS the click (modal): a click ON a row
                // ACCEPTS it (same as Enter), a click OUTSIDE the card DISMISSES
                // it (same as Esc), a click inside but off a row is swallowed —
                // it never falls through to move the document cursor beneath the
                // card. Otherwise: a press ON a page-column edge begins a DIRECT
                // width resize (symmetric about center) instead of a text
                // selection; else it's a normal click / selection start.
                if self.overlay.is_some() {
                    self.overlay_click(event_loop);
                } else if self.search.is_some() && self.panel_click() {
                    // CLICK-TO-SWITCH-FIELD: a press on the find/replace panel
                    // focused a field (or was an in-card no-op); it never falls
                    // through to a document press. A press OFF the panel returns
                    // false and continues to the page-resize / doc-click path.
                } else if self.begin_image_resize_if_hovering() {
                    // A press ON an inline image's resize EDGE/CORNER begins a DIRECT
                    // drag-resize (its width tracks the pointer, previewed live)
                    // instead of a text selection — checked AHEAD of the page-column
                    // edge + the document press, since a handle sits inside the column.
                } else if !self.begin_page_resize_if_hovering(event_loop) {
                    // A press on a persistent MARGIN OUTLINE row jumps the caret to
                    // that heading (click-to-jump) instead of a document press; a press
                    // anywhere else is a normal click / selection start.
                    if !self.outline_click() {
                        let shift = self.mods.state().contains(ModifiersState::SHIFT);
                        // The SAME column-membership geometry that gives the gutter
                        // its arrow cursor owns press admission too. Margin x values
                        // must never reach the document hit-test (which correctly
                        // clamps drags to line endpoints, but is the wrong behavior for
                        // a gesture that STARTS outside the page).
                        let over_writing_column = self.pointer_over_writing_column();
                        self.on_press(shift, over_writing_column);
                        if over_writing_column {
                            self.sync_view(true);
                        }
                    }
                }
            }
            ElementState::Released if self.image_resizing.is_some() => {
                // Commit the settled image width: write the `|NNN` hint back as ONE
                // undoable edit (mutually exclusive with a page-resize / selection).
                self.end_image_resize();
            }
            ElementState::Released if self.page_resizing => {
                // Commit + persist the settled page width (sticky).
                self.end_page_resize();
            }
            ElementState::Released => {
                self.dragging = false;
                self.drag_armed = false;
                // A plain click (press + release with no drag) leaves the
                // press-time anchor lingering at the cursor. Collapse it so
                // a subsequent bare motion (C-p, C-n, …) just moves the
                // cursor and does NOT extend a phantom selection. A real
                // drag (or double/triple-click) leaves cursor != anchor,
                // i.e. has_selection(), so its mark is preserved.
                if !self.buffer.has_selection() {
                    self.buffer.clear_mark();
                }
                self.sync_view(true);
            }
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

    /// `WindowEvent::MouseWheel`: an overlay owns the wheel (drives the list),
    /// else Cmd/Super+wheel zooms, else free scroll. Converts the LineDelta /
    /// PixelDelta into a whole-row count first.
    pub(in crate::app) fn on_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        // DEBUG key→px: scroll is input awaiting pixels — every wheel
        // path below ends in the arm's request_redraw.
        self.stamp_input();
        // Zoom modifier: Cmd/Super only. (Ctrl must NOT zoom on mac.)
        let zoom_mod = scroll_zoom_intent(self.mods.state());
        // HORIZONTAL TABLE PAN (a live-only reading gesture): a clearly-horizontal
        // two-finger scroll over an OVERFLOWING table pans its grid rather than
        // scrolling the document. Only when no picker owns the wheel and Cmd/Super
        // isn't zooming; a mostly-vertical scroll falls straight through.
        if !zoom_mod && self.overlay.is_none() {
            let (dx, dy) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (x * WHEEL_PIXELS_PER_LINE, y),
                MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
            };
            if dx.abs() > dy.abs() * 1.2 && dx.abs() > 0.5 {
                let (px, py) = self.cursor_px;
                let scroll = self.scroll_lines;
                if let Some(gpu) = self.gpu.as_mut() {
                    if gpu.pipeline.try_table_pan(px, py, scroll, dx) {
                        gpu.window.request_redraw();
                        return;
                    }
                }
            }
        }
        // Convert the delta to a line count (LineDelta or PixelDelta).
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINES_PER_NOTCH,
            MouseScrollDelta::PixelDelta(p) => {
                self.scroll_px_accum += p.y as f32;
                let whole = (self.scroll_px_accum / WHEEL_PIXELS_PER_LINE).trunc();
                self.scroll_px_accum -= whole * WHEEL_PIXELS_PER_LINE;
                whole
            }
        };
        if self.overlay.is_some() {
            // A summoned picker OWNS the wheel (it is modal): wheel drives the
            // LIST (advance the selection/scroll window, like ↑/↓); the document
            // behind it does NOT scroll. Symmetric with the click/hover consume.
            if lines.abs() >= 1.0 {
                self.overlay_wheel(lines);
            }
        } else if zoom_mod {
            // Cmd/Super + wheel: zoom in/out (wheel up = zoom in).
            if lines.abs() >= 1.0 {
                let dir = lines.signum();
                self.set_zoom(self.zoom + dir * render::ZOOM_STEP);
            }
        } else if lines.abs() >= 1.0 {
            // Free scroll: wheel up moves content down (scroll up), so a
            // positive wheel y DECREASES the top scroll line.
            self.wheel_scroll(-lines);
            self.sync_view(false);
        }
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.window.request_redraw();
        }
    }

}
