# Focus-mode removal inventory (compiled 2026-07-09, HEAD was 0312d4a — LINE NUMBERS HAVE DRIFTED across faaf3f7/136fdd0/5e3934c/df773ba; treat as a map, re-verify every site by grep before editing)

Focus mode = the iA-Writer-style paragraph/sentence dimming (CycleFocusMode, C-x d). NOT window focus (WindowEvent::Focused, sidecar `focus` may be FIELD focus — check), NOT overlay query-field focus.

## A. Pure focus-mode code (pass 1 deletes outright)
- src/focus.rs (whole file, ~177 lines): FocusMode enum Off/Paragraph/Sentence, FOCUS_MODE AtomicU8 + TEST_LOCK, mode()/set_mode()/cycle(), dim_srgb(), active_range(), FOCUS_DIM_STRENGTH/FOCUS_FADE_SECS, 5 unit tests.
- src/buffer/focus.rs (whole file, ~183 lines): paragraph_bounds_str/sentence_bounds_str + helpers — used ONLY by focus::active_range.
- src/render/focus.rs — the focus-only parts: update_focus (driver), settle_focus (capture pin), focus_report (sidecar), step_focus (fade stepping), focus fade state. KEEP ALIVE in pass 1: clear_focus_spans, refresh_focus_spans, color_char_range, line_doc_byte_start AND the multi-feature report fns living in this file (md_report, wysiwyg_report, outline_current, outline_report, syn_report, syn_lang_report — NOT focus-specific).
- src/render.rs TextPipeline: fields focus_cur/focus_prev/focus_t/focus_initialized/focus_sig/focus_lines (+ their new() init), has_baked_theme_colors' focus condition, set_view's update_focus call, two refresh_focus_spans(true) restyle calls, step_focus term in the advance OR-chain.
- src/render/layers.rs prepare_text_layer: default_color chooses focus::dim_srgb() when mode != Off — simplify to theme::base_content() path only.
- src/render/tests.rs: focus_typewriter_centers_the_cursor_row, focus_paragraph_colors_only_the_active_unit, focus_in_unit_edit_does_not_rekick_fade, focus_survives_buffer_shrink_below_colored_lines, clear_focus_spans_skips_out_of_range_stale_index, focus_move_reconcealing_a_wysiwyg_line_invalidates_stale_row_geometry, syn_attrs_comment_tiers_and_focus_override (this one: the focus-override HALF dies; comment-tier half may be worth keeping).

## B. Shared seams (pass 2 rearchitects — LOAD-BEARING for markdown/syntax/CJK/wysiwyg)
- clear_focus_spans + color_char_range (render/focus.rs) re-lay md/syn/CJK/conceal spans — the cursor-move conceal-refresh (refresh_rule_conceal / wysiwyg reveal) flows through these. Pass 2: fold into build_line_attrs/an honestly-named refresh module; delete the focus color-override plumbing.
- src/render/spans.rs add_focus_overlay_spans (~40 lines) + the color_override parameter threading — focus-only overlay mechanism; pass 2 removes it and simplifies signatures.
- src/render/text.rs: focus_lines guard in rule-conceal refresh (`!self.focus_lines.contains(&li)`), build_line_attrs focus comments.
- Multi-feature report fns need a new home when render/focus.rs dies (render/reports.rs or fold into text.rs).

## C. Mechanical enum/test/catalog updates (pass 1)
- src/keymap.rs: Action::CycleFocusMode variant + C-x d arm (+ doc comments).
- src/commands.rs: "Focus mode" Command entry + names() array entry.
- src/actions.rs: CycleFocusMode → focus::cycle() match arm.
- src/actions/tests.rs: ~4 pattern-match/law-test sites naming CycleFocusMode.
- src/app/apply.rs: CycleFocusMode effect-side arm.
- src/app/viewstate.rs: follow_scroll_strategy takes focus::FocusMode param — remove param; `let center = mode != Off || typewriter` → `let center = typewriter`; its test cases.
- src/whichkey.rs test: has("d", "Focus mode").
- src/menu.rs: View item "Focus mode" (ri("awl.focus_mode", ...)) + VIEW_ITEMS index comment adjust.
- src/menu_icons.rs: draw_focus_mode + "awl.focus_mode" registrations + icon tests/lists.
- src/main/args.rs: --focus flag parse block + comment.
- src/config.rs: comments naming focus mode as sticky/ephemeral state (check for a focus sticky key — apply_sticky_globals).

## D. Capture/headless (pass 1)
- src/capture/sidecar.rs: focus_json() + `"focus": {focus}` template line → SCHEMA BUMP per convention; update schema-history note.
- src/capture/modes.rs: focus-centering branch + settle_focus call.
- src/capture/animated.rs: two settle_focus calls.
- capture tests asserting the focus block.

## E. Docs (pass 1 or 2)
- CLAUDE.md: focus mentions in span-seam descriptions (~10 sites; keep window-focus mentions), sidecar schema list, menu roster, icon list.
- DESIGN.md: focus-as-depth §, muted-ink focus-dim mention.
- PHILOSOPHY.md: focus-mode full-ink example.
- Keep typewriter references intact (typewriter is NOT being removed; it loses only its focus interaction).

## Sequencing law
Pass 1 = A+C+D (+ the E lines that describe deleted surface), keeping B compiling (dead params tolerated, focus_lines always-empty etc.). Full suite green → commit. Pass 2 = B (fold/rename/simplify, reports re-homed, remaining E), full suite green → commit.
