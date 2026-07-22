# awl docs — Render: adaptive column, theme caps, overlay chrome, picker rows

> Read BEFORE touching `render/`, `theme/model.rs` (RenderCaps), `overlay/`, `settings.rs` palette rows, or picker/row layout. Moved verbatim out of CLAUDE.md 2026-07-22 (queue item 17); earlier round history: `git log -p CLAUDE.md`.

## Adaptive-column placement (`render/geometry.rs`)

- On a small screen the symmetric centered column cramped the margin outline while the right margin sat empty. `TextPipeline::column_left` is the ONE owner of an ADAPTIVE policy (no config knob): shifts RIGHT under pressure to grant the outline a rail (taking the empty right-margin space), back to symmetric when there's room. Every downstream reader (caret/selection/washes, hit-test, drag handle, gutter) composes it for free.
- Pure policy `adaptive_column_left`, one formula `desired_left.min(max_left).max(symmetric_left)`, three regimes (WIDE = byte-identical passthrough to the old column; NARROW = shift, width never touched; NARROWEST = re-centers → outline auto-hides at its existing floor). Hide threshold + shift threshold read the SAME `left` (can't drift). Transition is INSTANT (glide banked). **LIVE-ONLY:** the feel of a real small-screen resize.

## Theme capabilities as data (`theme/model.rs::RenderCaps`)

- Render call sites never branch on world identity — they read a `RenderCaps` field: `selection_style`, `caret_block_style`, `backdrop`, `elevation`, `decorative_wash`, `image_reveal`, `highlight_texture`, plus the personality fields `card_anchor`, `chrome_face`, `motion`, `list_style`, `facet_style` (and `TitleStyle::Placard{corner,scale,ink}`). Born as the Wagtail refactor ("DEFAULT = every other world"); that framing is DEAD — 16 worlds now, and MANY set fields away from default (Firetail: placard BL 4.5/Bold + `chrome_face` Named("Archivo Black") + Bordered elevation; Galah/Magpie/Mangrove/Firetail: `list_style` Bars + placards; six worlds TopLeft `card_anchor` while DEFAULT flipped TopCenter; lava worlds `motion` CALM). **No theme may need its own code path** — new personality = new caps field + data.
- `is_one_bit()` still exists (pins Wagtail's identity for the monochrome law tests) but the RENDERER no longer reads it. **GREP-LAW `theme_caps_law`:** fails if `.is_one_bit(` or a quoted world name appears in real code under `src/render/` — structurally bans a future per-theme special case.

## Overlay personality & chrome composition (render/chrome/ + theme/model.rs)

- A fortnight of rounds, one shape: overlay/chrome VARIETY is DATA in `RenderCaps`, never a per-world code path. `ListStyle` (Pane default | Bars = per-row plates), `FacetStyle` (Text default | Band | Chips), `CardAnchor` (incl. TopRight + `mirrors_growth` for Bars), `TitleStyle::Placard{PlacardCorner, PlacardInk}`.
- **HELD BACK, not dead:** Chips is REBUILT-for-real but ships inert pending the user's variant pick; poster facets stay Text. Probe forces for galleries: `AWL_FACET_STYLE_FORCE`, `AWL_OVERLAY_ANCHOR_FORCE` (env, CLI-invisible).
- **`PlacardCorner::Auto` derives COMPLEMENTARY to the card anchor via ONE owner `render::derived_placard_corner`;** `overlay_shape_placard` shrinks-to-fit so placards never clip — the old "every placard BL" pin is RETIRED for an end-to-end no-clip OUTCOME law.
- **One-owner geometry seams (route through these, never re-derive):** `overlay_card_x`, `overlay_row_top`/`_of`/`_index` (+ `header_gap`), `push_overlay_hint_spans`, `overlay_footer_reclaim`.
- The theme picker RETIRED its runtime lens strip (user decision 2026-07-15, recorded in src/facets.rs); the axes are a build-time ruler.

## Settings in the palette + overlay titles (`overlay/` + `settings.rs`)

- The Cmd-P palette's rows are catalog commands **∪** `settings::SETTINGS` (a settings row like "Keymap" is fuzzy-findable straight from the palette). Still ONE `OverlayKind::Command`; the union is DATA (`attach_settings_rows`, an `is_setting` flag). A settings row shows its current value in the secondary column; marker prefix `§ ` (measured bundled in `AwlMarks.ttf`; the gear ⚙ is NOT bundled, so it never competed). Dispatch parity via ONE owner `dispatch_settings_row` (`close_on_toggle` = the only difference: palette closes, Settings menu stays).
- **Every `OverlayKind` names itself** (`OverlayKind::title`, no-wildcard) — drawn as a muted prefix on the picker's input line (Rename/InsertLink opt out via `draws_title_prefix`, their own prompt orients). Sidecar `overlay.title`.

## Picker rows (`render/rowlayout`)

Picker rows go through `render/rowlayout` — never place row text directly. A PRIMARY cell (never dropped, elided last-resort) + optional SECONDARY right column (first to yield). `rowlayout::plan` → `fits` → `fit_primary` (the only elision door). The law test enumerates `OverlayKind` with a NO-WILDCARD match. The bottom-left page-mode GUTTER rides the same owner (`gutter_plan`) — stacked, so neither line yields from width pressure (the filename never wraps; the fix for "DESIGN.md → DESIG/N.md and the project vanishes").
