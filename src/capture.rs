//! Headless one-frame capture: render the shared text pipeline to an offscreen
//! texture, read the pixels back to the CPU, and write a PNG + a JSON sidecar.
//!
//! This is the PRIMARY verification path for the project: same input => byte
//! stable PNG, plus a machine-readable description of render state.
//!
//! The harness is split into focused submodules (the `render.rs` precedent), with
//! this file as the module ROOT holding only the shared constants + the wiring:
//! - [`gpu`]: the headless wgpu device / offscreen target / pixel readback.
//! - [`opts`]: the public INPUT types ([`CaptureOpts`] + its metadata blocks).
//! - [`modes`]: the SINGLE-FRAME capture entry points + shared snapshot helpers.
//! - [`animated`]: the `--capture-timeline` / `--capture-held` per-step drivers.
//! - [`sidecar`]: the hand-rolled JSON sidecar writer.
//! - [`oracle`]: the headless visual-line motion oracle for `--keys` replay.
//!
//! Every public item is re-exported here so the `capture::*` call sites resolve
//! exactly as before.

/// Deterministic canvas size for headless renders.
pub const CANVAS_WIDTH: u32 = 1200;
pub const CANVAS_HEIGHT: u32 = 800;
/// Offscreen format. Srgb so glyphon's default (sRGB) blending matches windowed.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The sidecar SCHEMA. [`SCHEMA_VERSION`] (defined just below this history table)
/// is THE single number — bump it ONCE per sidecar-shape change and APPEND a row
/// to the table below (never edit a past row; that is what makes a bump a
/// one-line, merge-friendly edit). The three emitted shapes derive from it via
/// [`schema_plain`] (`--screenshot`, caret block absent), [`schema_timeline`]
/// (`--capture-timeline`, caret block, no `trail`), and [`schema_held`]
/// (`--capture-held`, caret block WITH the `trail`) — PLAIN = N, TIMELINE = N+1,
/// HELD = N+2 — so the three shape strings can never drift from each other.
///
/// HISTORY TABLE (append-only) — each entry is `/N` (the PLAIN number at that
/// round) and what changed:
///
/// `/86` (was `/83`) added `font.cjk` — the Japanese-bundle round's resolved CJK
/// family + whether it's the bundled Noto Serif/Sans JP face (see
/// `render::TextPipeline::cjk_report`), `null` when the buffer has no CJK run.
/// (Landed alongside the WYSIWYG round's `/83` `wysiwyg` block bump, so that
/// merge carried both additions in one further bump.)
///
/// `/89` (was `/86`) adds `buffers` — the multi-buffer core round's `{ open,
/// active }` report (see `crate::buffers::BufferRegistry`, CAPTURE.md).
///
/// `/92` (was `/89`) is the i18n round: a top-level `doc_lang` field (the
/// document's own frontmatter `lang:` tag, `null` when untagged/non-markdown —
/// see `crate::frontmatter::detect`), and `font.scripts` — `font.cjk`'s shape
/// generalized to all four non-Latin scripts (`{ ja, zh_hans, zh_hant, ko }`,
/// each `{family, bundled}|null` — see `render::TextPipeline::script_font_report`).
/// The HUD block also gains a `lang` field (see `hud::Stats`).
///
/// `/95` (was `/92`) FIXES the `gutter` block to always agree with the pixels
/// (the gutter-elision bug: a long filename used to WRAP mid-word in the
/// left-margin box while `gutter.name`/`gutter.project` kept reporting the raw,
/// un-drawn text — see `render::rowlayout::gutter_plan` +
/// `render::TextPipeline::gutter_layout`). Same shape, corrected meaning: BOTH
/// `name` and `project` are EXACTLY as drawn — each independently fit to ONE
/// line, middle-elided (extension preserved) the instant the margin can't hold
/// it whole. A taste pass (still under this same `/95`, before either shape
/// shipped anywhere) settled the two lines' relationship: neither yields to the
/// other from width pressure — `project` is `""` here only when there is
/// genuinely no project to show, never as a forced yield to protect the
/// filename. Unaffected at any margin wide enough to hold both lines whole
/// (every existing wide-window capture).
///
/// `/98` (was `/95`) is the PROSE/CODE PAGE-WIDTH SPLIT: the 70-char measure is
/// a PROSE number; a recognized code file now reads its own `page_width_code`
/// (default 100, rustfmt's `max_width`) instead of sharing the prose measure.
/// The `page` block gains `class` (`"prose"`/`"code"` — `TextPipeline::page_class`,
/// the SAME classifier `Buffer::page_class` uses), so a reviewer can assert which
/// sticky measure is in effect directly from the sidecar. Every OTHER `page`
/// field is unchanged; a document whose class was already implicitly "prose"
/// under the old single `page_width` key renders byte-identically (same default
/// measure, `class: "prose"` newly reported).
/// `/99` (was `/98`) is the SUMMONED ABOUT CARD (`about.rs` + `menu.rs`'s
/// routed About item, replacing muda's predefined About dialog): a top-level
/// `about` block, `{ "open": bool }` — `false` by default (byte-identical),
/// `true` after the palette "About" command / `--keys` replaying it. See
/// CLAUDE.md's menu-bar section for why About moved off muda's predefined
/// item (a real use-after-free fix in `menu::install`, unrelated to About
/// specifically, plus a taste upgrade to an in-app card).
/// `/100` (was `/99`) is the LINE-ENDINGS SURFACE (the VS Code EOL model's UI
/// half): the held stats HUD gains a LINE ENDINGS row, so the `hud` block gains
/// an `eol` field (`"LF"`/`"CRLF"` — the active buffer's on-disk ending,
/// `Buffer::eol`). Unlike the HUD's dropped clock/fs fields this is a PURE
/// function of the buffer, so it carries its real value in a headless capture
/// (a CRLF fixture reports `"CRLF"`, an LF fixture `"LF"`, and the palette
/// "Line endings…" command flips it). Every other field is unchanged.
/// `/103` (was `/100`) is the PER-WORLD ORNAMENT FACE (`theme::Theme::
/// ornament_face`): the `font` block gains an `ornament` field — the family the
/// active world shapes its markdown section-break fleuron (`---`/`***`/`___`) AND
/// its About end-mark in (one of `"EB Garamond"` / `"Junicode"` / `"Awl Marks"`).
/// A pure function of the active theme, so it carries its real value in every
/// capture; a default `--screenshot` (Tawny → `"Awl Marks"`) is byte-identical
/// apart from the new field. Every other field is unchanged.
/// `/106` (was `/103`) is HIDE-DOTFILES-IN-PICKERS: the file pickers (go-to /
/// browse) hide dot-prefixed entries by default (`index::is_hidden_entry`, `.env*`
/// excepted), with a `Cmd-Shift-.` reveal toggle, so the `overlay` block gains a
/// `show_hidden` bool. `items` already reflects the filtering (dotfiles absent by
/// default, present after the toggle). A default `--screenshot` (no overlay) emits
/// `show_hidden: false` in the inactive-overlay block; every other field unchanged.
/// `/109` (was `/106`) is GFM TABLES: `Options::ENABLE_TABLES` is on, so the
/// `md_spans` block gains `table_pipe` (the cell-delimiter `|`), `table_sep` (the
/// `|---|` header-separator row), and `table_header` (a header cell's content) tags
/// for a markdown table. awl renders the table as styled SOURCE (dim the structural
/// markup), never a drawn grid; a non-table markdown / code buffer is byte-identical.
/// `/112` (was `/109`) is THEME-PICKER SWATCHES: the `overlay` block gains a
/// `swatches` array parallel to `items` — for a THEME picker (`lens` set) each row's
/// `[ground_hex, accent_hex]` (its palette chip's ground band + accent dot, from
/// `theme::swatch_for`), `[]` for every other overlay. A non-theme / no-overlay
/// capture is byte-identical apart from the new empty `swatches: []` field.
/// `/115` (was `/112`) is OVERLAY EMPTY STATE: the `overlay` block gains an `empty`
/// field — the shared calm message drawn when a picker has NO candidate rows (an
/// empty corpus → per-kind "no history yet"/"no suggestions"/…, or a query that
/// matched nothing → "no matches"), else `null`. From the one owner
/// `OverlayState::empty_notice`, shared with the rendered dim message row. A picker
/// WITH rows (and a no-overlay capture) is byte-identical apart from `empty: null`.
/// `/118` (was `/115`) is OVERLAY GIT TAGS: the `overlay` block gains a `git` array
/// parallel to `items` — for a Project / Browse picker each row that is itself a git
/// repo carries a dim `"git"` tag drawn in the row's SECONDARY column (replacing the
/// old `• ` name-prefix marker), `""` for a non-git row and `[]` for a git-free
/// listing / every other overlay. From the one owner `OverlayState::item_git_tags`,
/// shared with the rendered right-column tag. A git-free / no-overlay capture is
/// byte-identical apart from the new empty `git: []` field.
/// `/121` (was `/118`) is the OVERLAY SCROLL-WINDOW report: the `overlay` block gains a
/// `window` object `{ top, lines, sel_row, card_h, canvas_h }` — the DRAWN candidate
/// window (flat pickers = rows, faceted/grouped pickers = headers + rows counted
/// together), so a headless test can assert the card is BOUNDED (`card_h ≤ canvas_h`)
/// and the selection stays visible (`sel_row < lines`, the selected row's position among
/// the drawn candidate lines).
/// This is the fix for the grouped/faceted path (go-to / browse / theme under a
/// sectioned lens) rendering its whole list uncapped off the bottom of the screen. From
/// the one owner `TextPipeline::overlay_window_report`, the SAME geometry the card draws
/// from. `null` for a no-overlay capture, which is otherwise byte-identical.
/// `/124` (was `/121`) DROPS the theme-picker `swatches` field: the per-row palette
/// chips were removed (the live doc preview already shows each world), so the `overlay`
/// block no longer carries a `swatches` array. Every other field is unchanged; the
/// active-lens `lens`/`lens_strip` reporting is untouched.
/// `/127` (was `/124`) ADDS the top-level `tables` block: the WYSIWYG table-grid
/// round reports each rendered GFM table's `{ range, rows, cols, col_widths,
/// revealed }` (empty `[]` for a non-table / WYSIWYG-off frame). Every other field
/// is unchanged; a non-table capture stays byte-identical.
/// `/130` (was `/127`) ADDS the top-level `images` block: the inline-images round
/// reports each markdown `![alt](path)` image's `{ range, line, path, width_hint,
/// display_w, display_h, missing, revealed }` (empty `[]` for a non-image /
/// images-off / wasm frame). Every other field is unchanged; a non-image capture
/// stays byte-identical.
/// `/133` (was `/130`) ADDS the top-level `outline` block: the persistent margin
/// outline round reports `{ on, headings:[{text,level,line}], current }` — the
/// document's headings (distilled from the markdown parse) + the nearest heading
/// at/above the caret (`current`, or `null`). `on` mirrors `outline_on()` (OFF by
/// default at the time this schema entry landed). Every other field is
/// unchanged; a default (outline-off) capture was byte-identical at the time.
/// **NO SCHEMA FIELD CHANGE, but a later behavior flip:** `outline_on()`'s
/// built-in default flipped to ON on 2026-07-09 (a user-decided taste reversal,
/// see `outline.rs`'s module doc) — `outline.on` now defaults `true`, and a
/// markdown buffer WITH headings under page mode legitimately draws the margin
/// outline in a default capture where it previously did not.
/// `/136` (was `/133`) ADDS five LIFETIME-ODOMETER fields to the `hud` block
/// (`chars`/`writing`/`files`/`caret_travel`/`world`) — the held HUD's quiet
/// personal odometer. All five are LIVE-ONLY: the fixed `"—"` placeholder in a
/// capture (no persisted store), so the block stays byte-stable across machines.
/// Every other field is unchanged; a default (HUD-released) capture is byte-identical.
/// `/139` (was `/136`) ADDS the `ancestors` array to the `outline` block — the
/// current heading's ANCESTOR CHAIN (the heading indices the caret is nested inside),
/// the rest of the "lit path" the margin-outline design-crit round lifts alongside
/// `current`. Empty `[]` when there is no current heading or it is top-level. A pure
/// text + caret fact; every other field is unchanged, so a default (outline-off)
/// capture stays byte-identical apart from the added key.
/// `/142` (was `/139`) SPLITS the LIFETIME ODOMETER out of the `hud` block into a
/// new top-level `lifetime` block (`{ open, characters, time_writing,
/// files_touched, caret_travel, your_world }`) — the summoned "Lifetime stats"
/// card. The five odometer fields are REMOVED from `hud` (now a pure per-doc peek:
/// `held`/`words`/`reading_min`/`percent`/`lang`/`eol`). `lifetime.open` is false
/// by default and the five figures are the fixed `"—"` placeholder in a capture
/// (LIVE-ONLY, no persisted store), so a default (Lifetime-closed) capture stays
/// byte-identical apart from the moved keys.
/// `/145` (was `/142`) REMOVES the top-level `focus` block — FOCUS MODE (the
/// iA-Writer paragraph/sentence dimming) was removed as a feature, so its
/// `{ mode, active_start, active_end }` block is gone. Every other field is
/// unchanged; a default capture is byte-identical apart from the dropped key.
/// `/148` (was `/145`) adds the ASSET CLEANER picker (`OverlayKind::Assets`,
/// `overlay.mode == "assets"`) — the `overlay` block's existing `items` / `bindings`
/// / `empty` fields carry its orphan list (leaf names + a "size · parent" secondary +
/// the empty-state line), so no new field was added; the version bumps because a new
/// overlay mode string is now reachable. A default capture (no overlay) is
/// byte-identical.
/// `/151` adds the HOLD-⌘ SHORTCUT PEEK block (`peek`, `{ open, rows: [{chord, name}] }`
/// — the discoverability round's held shortcut card): `open` is false by default (a
/// default capture is byte-identical), and `rows` reports the settled card's shortcuts
/// (the curated STARTER SIX in a capture, since no live ledger runs there). Summoned in
/// a capture via the `--peek` flag.
/// `/154` adds the overlay BREADCRUMB (`overlay.return_to`, the summoning overlay's
/// mode string or null): any picker summoned FROM another overlay — a Settings
/// sub-picker, or a command run from the palette — remembers its way back, so Esc
/// (and a value-picking accept) POPS one level instead of closing everything. `null`
/// for every top-level summon (⌘O / ⌘T / a menu click), so a default `--screenshot`
/// is byte-identical apart from the new always-present `return_to: null` field.
/// `/157` adds the CJK-PRIORITY LANGUAGE picker (`OverlayKind::CjkLang`,
/// `overlay.mode == "cjk_lang"`) — the Settings "CJK priority" row (a
/// `SettingKind::List`, retired) grows up into a `SettingKind::Picker` row
/// renamed "Ambiguous CJK reads as": Enter opens this new picker (the four
/// ambiguity-ladder languages in writer-words, mirroring the Dictionary picker's
/// shape) instead of dropping into config.toml as text, and its value cell now
/// reads the live ladder's front language in writer-words ("Japanese") rather
/// than the raw comma-joined BCP 47 codes. No new sidecar field — the existing
/// `overlay` block's `items`/`bindings`/`hint` carry the new picker exactly like
/// every other flat, non-faceting kind; the version bumps because a new overlay
/// mode string is now reachable, and the "CJK priority" row's own value-cell text
/// changed shape. A default capture (no overlay) is byte-identical.
// `/160`-`/162` — the WEB/LINUX MENU BAR round: a new top-level `menubar` block
// (`{ shown, open_menu, items }`) reporting the awl-rendered menu bar's state, read
// from the SAME `menubar` globals + `menu::roster()` the renderer draws from. Default
// OFF on macOS (the capture platform), so a plain `--screenshot` reports
// `shown: false` and is byte-identical; `--menu-bar` / `--menu-open N` drive it on.
// `/163`-`/165` — THE TABLE ROUND (columns + horizontal pan + THE X-RAY): a new
// top-level `xray` block (`{ active, line, chars, pan }`) reporting the caret-in-
// table floated-source state — `active: true` when the caret sits on a GFM table
// row (the row's raw source floats non-wrapping over the still-drawn grid; the
// document NEVER reflows). The `tables` block's `revealed` now means "the x-ray is
// active on this table" (grid stays drawn) rather than "grid parked", and a
// revealed table's source stays in `wysiwyg.concealed` (zero-reflow). Default OFF
// (no caret in a table), so a plain `--screenshot` reports `active: false` and is
// byte-identical.
// `/164` — THE KEYMAP FLAVOR ROUND: the `project` block gains `keymap_flavor`
// (`"native"`/`"emacs"`, the EFFECTIVE config `keymap` value —
// `crate::keymap::KeymapFlavor::config_name`), mirroring `notes_root`/
// `workspace`'s "verifiable from `--config` with no flags" precedent. Present
// only when `project` itself is (a `--root`-driven capture); a plain
// `--screenshot`'s `project: null` is unaffected either way. Defaults to
// `"native"` everywhere the flavor isn't config-driven (the timeline/held
// capture modes, which take no `--config`).
// `/165` — THE NOTES VERBS round: the `hud` block gains `saved` — the SAVED
// stat's already-phrased text ("unsaved changes" / a calm relative-time phrase /
// the fixed "—" placeholder headlessly, via `crate::hud::saved_readout`). A
// default capture (HUD not held) still reports it (mirrors `eol`/`percent`'s
// always-present shape), always the placeholder there since no live clock ever
// reaches a capture.
// `/166` — CHECK FOR UPDATES round: the `about` block gains `checked` — the
// "checked … ago" line (`crate::updates::checked_line`). A headless capture
// (About open) reports the fixed placeholder string `"checked —"` (mirrors
// `hud.saved`'s own placeholder-string, not `null`, precedent); live with no
// marker ever written it is JSON `null` (the card omits the line entirely);
// live with a marker it's the phrased string. `about.open` is unaffected.
// `/167` — THE PALETTE-SETTINGS-UNION + OVERLAY-TITLES round: the `overlay`
// block gains `title` — the picker's short self-announcement
// (`crate::overlay::OverlayKind::title`), the same text the render path draws
// as a quiet prefix on the input line. `null` when no overlay is open, a
// nonempty lowercase string for every kind otherwise. Settings rows joining
// the Cmd-P palette adds no field of its own — a settings row appears as an
// ordinary `overlay.items` entry (its `"§ "`-prefixed display text), and its
// current value rides the EXISTING `overlay.bindings` column.
// `/168` — THE LAVA-LAMP GROUND round (machinery only): `page.background` gains a
// new tagged arm, `{ "kind": "lava", ground, blob_lo, blob_hi, edge, dithered,
// phase }` — the metaball ground's DATA plus the effective render `phase` (a
// FIXED t=0 in every headless capture, the determinism law). The block now
// reports the EFFECTIVE background actually drawn (honoring the dev `AWL_LAVA`
// gallery knob) rather than `theme::background()` directly; for every one of the
// fifteen shipped worlds (no knob, no lava world) that is identical, so a default
// capture's `page.background` CONTENT is byte-unchanged — only the schema string
// bumps.
// `/169` — PASSIVE CRASH RECOVERY: the `about` block gains `pending_crash`.
// False by default/headlessly; an explicit injected pending marker reports true
// and adds the matching quiet About-card line. The startup center notice is gone.
// `/170` — THE FORMAT POPOVER: a new top-level `popover` block (`{ on, shown,
// card, buttons }`) reporting the reveal-on-select format toolbar — `on` mirrors
// `crate::popover::popover_on()` (default ON), `shown` is whether it is up this
// frame (a mouse selection live, or the `AWL_POPOVER` capture probe), and when
// shown `card` is `[x,y,w,h]` + `buttons` is one `{label, active, x0, x1}` per
// button (the SAME geometry the buttons draw + the click hit-test reads). Down by
// default (no pointer in a capture), so a plain `--screenshot` reports
// `shown: false, card: null, buttons: []` and is byte-identical apart from the
// always-present block.
// `/171` — WRITING STREAKS: a new top-level `streaks` block (`{ open, streak,
// today_words, cells: [..] }`) — the summoned year-calendar heatmap card
// (`streaks.rs`). `open` is false by default (a default capture is byte-identical
// apart from the added block), true via the palette "Writing streaks" command /
// the `--streaks` capture flag. The figures + `cells` intensity grid are the LIVE
// year the App pushed OR the fixed synthetic `streaks::placeholder` in a capture
// (no persisted store), so a `--streaks` capture is deterministic + byte-stable.
// `/172` — THE WRITER'S DIFF: a new top-level `diff` block — `null` for every
// ordinary capture (a plain `--screenshot` is byte-identical apart from the added
// key), else `{ active, label, struck, washed, modified, moved, folds }` reporting
// the STATE of the read-only prose-diff view (`crate::prosediff`) the capture
// harness rendered (`AWL_DIFF_OLD`/`AWL_DIFF_NEW`). A state oracle only — the
// struck/washed APPEARANCE is asserted over the PNG's pixels.
// `/173` — PER-WORLD HEADING WEIGHT (+ Ladder J): the `theme` block gains
// `heading_bold` — the EFFECTIVE section-heading weight bit this capture rendered
// with (the active world's `Theme::heading_bold` folded through the ONE owner
// `markdown::heading_weight_bold`, so the `AWL_HEADING_BOLD_FORCE` gallery knob is
// reflected honestly). `true` ⇒ `##`/`###`+ shaped at the world's real bundled
// Bold; the TITLE (`#`) never bolds regardless. The size ladder also retuned
// (Ladder J: TITLE 1.6 / SECTION 1.3 / SUBHEAD 1.15), so markdown captures'
// heading geometry changes everywhere — expected, not a schema matter.
pub const SCHEMA_VERSION: u32 = 173;

/// `awl-capture/N` — the `--screenshot` single frame (caret block absent).
pub fn schema_plain() -> String {
    format!("awl-capture/{SCHEMA_VERSION}")
}
/// `awl-capture/N+1` — a `--capture-timeline` step (caret block, no `trail`).
pub fn schema_timeline() -> String {
    format!("awl-capture/{}", SCHEMA_VERSION + 1)
}
/// `awl-capture/N+2` — a `--capture-held` step (caret block WITH the `trail`).
pub fn schema_held() -> String {
    format!("awl-capture/{}", SCHEMA_VERSION + 2)
}

mod animated;
mod film;
// `pub(crate)`: the render bench suite (`render::benchsuite`) drives frames
// through the SAME headless device / offscreen target / pixel readback this
// module owns — one owner of the wgpu plumbing, not an aligned copy.
pub(crate) mod gpu;
mod modes;
mod opts;
mod oracle;
mod sidecar;

pub use animated::{capture_held, capture_timeline, HeldDir};
pub use film::{FilmRenderer, FRAME_MS};
pub use modes::{
    capture_motion, capture_motion_diagonal, capture_motion_vertical, capture_with,
};
pub use opts::{BuffersInfo, CaptureInfo, CaptureOpts, DiffInfo, OverlayInfo, ProjectInfo};
pub use oracle::build_oracle;
// The sidecar module stays private (write-only); its JSON-string escaper is the
// crate's ONE escaper, shared with the storyboard trace so the two artifacts
// can never disagree on escaping.
pub(crate) use sidecar::json_string;

// The [`OraclePipeline`] type is part of the module's public surface but is not
// named at a call site today (the oracle is returned only as `Option<_>`), so
// re-exporting it as a bin-crate would otherwise warn unused.
#[allow(unused_imports)]
pub use oracle::OraclePipeline;

#[cfg(test)]
mod tests;
