//! src/config/apply.rs — the LAUNCH-TIME APPLY seam: the "apply" third of the
//! 2026-07 code-organization split (see `config/mod.rs` for the full module
//! doc). [`Config::apply_sticky_globals`] is the one place a freshly-loaded
//! config lands on every process-global sticky preference, honouring
//! flag-over-config precedence — reading/parsing lives in `config::model`;
//! format-preserving disk writes live in `config::write`.

use super::model::{parse_caret_mode, parse_dictionary};
use super::Config;

impl Config {
    /// LAUNCH-APPLY the remembered THEME / PAGE / CARET onto the process-globals
    /// (`theme::set_active_by_name` / `page::set_page_on` / `caret::set_mode`), so the
    /// editor opens in the state it was last left. Honours flag > config: each
    /// `*_flag` says the matching CLI flag was already supplied (and thus already set
    /// the global), so that pref is SKIPPED — the explicit flag wins. A stale/unknown
    /// remembered theme or caret value is ignored (keeps the built-in default). ZOOM is
    /// deliberately NOT here: it is per-instance, applied via `config.zoom` in
    /// `App::new` (live) and folded into `opts.zoom` (capture). Used by `main` after
    /// the config loads; the windowed + capture paths share this one seam.
    ///
    /// `measure_flag` says the `--measure N` flag already set the page WIDTH global, so
    /// the remembered per-class override is SKIPPED (the explicit flag wins) —
    /// mirroring how `page_flag` gates the remembered `page_mode`. `initial_class`
    /// is the STARTING buffer's [`crate::page::PageClass`] (derived from the launch
    /// `file` argument via `PageClass::of_path` — no `Buffer` exists yet at this call
    /// site), so the remembered `page_width_prose`/`page_width_code` resolves to the
    /// class that actually matters for the very first frame. A later buffer SWITCH
    /// (live `App::sync_page_measure`, or the headless `--keys` Goto switch) re-reads
    /// [`Self::measure_for`] against the buffer THEN active, independent of this
    /// initial pin.
    pub fn apply_sticky_globals(
        &self,
        theme_flag: bool,
        page_flag: bool,
        caret_flag: bool,
        measure_flag: bool,
        initial_class: crate::page::PageClass,
    ) {
        if !theme_flag {
            if let Some(name) = self.theme.as_deref() {
                crate::theme::set_active_by_name(name);
            }
        }
        if !page_flag {
            if let Some(on) = self.page_mode {
                crate::page::set_page_on(on);
            }
        }
        // AUTO-PAGE-ON for an AMBIENT-GROUND world (lava OR twinkling stars —
        // `Theme::has_ambient_motion`, the one gate): both grounds live ENTIRELY
        // in the MARGINS (masked/culled out of the writing column), so page mode
        // MUST be on for the ground to exist at all — page-off means no margins
        // means no lamp and no stars (the lava probe's own finding #2). Such a
        // world therefore forces the centered column on, OVERRIDING the
        // remembered `page_mode` / `--page` (the margins ARE the feature). Read
        // AFTER the theme is set above, so it reflects the world that will
        // actually open. Firetail/Mangrove (lava) + Currawong (stars) hit this
        // arm today.
        if crate::theme::active().has_ambient_motion() {
            crate::page::set_page_on(true);
        }
        if !measure_flag {
            crate::page::set_measure(self.measure_for(initial_class));
        }
        if !caret_flag {
            if let Some(m) = self.caret_mode.as_deref().and_then(parse_caret_mode) {
                crate::caret::set_mode(m);
            }
        }
        // WRITING NITS has no CLI flag (it is a quiet, always-available hint), so the
        // remembered value applies unconditionally when present; absent = the built-in
        // default (ON), which the `nits::NITS_ON` global already carries.
        if let Some(on) = self.writing_nits {
            crate::nits::set_nits_on(on);
        }
        // SPELLCHECK has no CLI flag either (like writing_nits): the remembered
        // on/off applies unconditionally when present; absent = the built-in
        // default (ON), which the `spell::SPELLCHECK_ON` global already carries.
        if let Some(on) = self.spellcheck {
            crate::spell::set_spellcheck_on(on);
        }
        // DICTIONARY has no CLI flag either (like writing_nits): the remembered
        // variant applies unconditionally when present + recognized; absent/unknown
        // leaves the `spell::ACTIVE_VARIANT` global at its built-in default (en_US),
        // so a plain launch — and a default `--screenshot` — stays byte-identical.
        if let Some(v) = self.dictionary.as_deref().and_then(parse_dictionary) {
            crate::spell::set_active_variant(v);
        }
        // WYSIWYG has no CLI flag either (like writing_nits/spellcheck): the
        // remembered on/off applies unconditionally when present; absent = the
        // built-in default (ON), which `markdown::WYSIWYG_ON` already carries.
        if let Some(on) = self.wysiwyg {
            crate::markdown::set_wysiwyg_on(on);
        }
        // FORMAT POPOVER: same pattern (no CLI flag) — the remembered on/off
        // applies when present; absent = the built-in default (ON), which
        // `popover::POPOVER_ON` already carries. OFF makes the mouse-summon a total
        // no-op, so a plain launch stays byte-identical.
        if let Some(on) = self.popover {
            crate::popover::set_popover_on(on);
        }
        // INLINE IMAGES: same pattern — the remembered on/off applies when
        // present; absent = the built-in default (ON), which
        // `markdown::INLINE_IMAGES_ON` already carries (and which is inert on
        // wasm, where `inline_images_on()` ignores the flag).
        if let Some(on) = self.inline_images {
            crate::markdown::set_inline_images_on(on);
        }
        // CODE LIGATURES: same pattern (no CLI flag) — the remembered on/off
        // applies when present; absent = the built-in default (ON), which
        // `render::CODE_LIGATURES_ON` already carries. Gates only code buffers'
        // programming ligatures; prose fi/fl is always on regardless.
        if let Some(on) = self.code_ligatures {
            crate::render::set_code_ligatures_on(on);
        }
        // PERSISTENT MARGIN OUTLINE: like the toggles above, the built-in default
        // is ON (`outline::OUTLINE_ON` starts true — flipped 2026-07-09, a
        // user-decided taste reversal of the original opt-in-off call; see
        // `outline.rs`'s module doc). A remembered value applies unconditionally
        // when present, EITHER direction (a config `outline = false` still wins);
        // absent leaves the global at its own default (ON), so a plain launch
        // with no config carries the new default forward.
        if let Some(on) = self.outline {
            crate::outline::set_outline_on(on);
        }
        // MENU BAR: the built-in default is PLATFORM-derived (`menubar::MENU_BAR_ON`
        // starts ON for web/Linux, OFF for macOS). A remembered value applies
        // unconditionally when present, EITHER direction (a config `menu_bar = false`
        // hides it on web/Linux); absent leaves the global at its own platform default,
        // so a plain launch with no config carries the right default forward. The
        // `--menu-bar` capture flag sets the global directly (before this runs).
        if let Some(on) = self.menu_bar {
            crate::menubar::set_menu_bar_on(on);
        }
        // TYPEWRITER SCROLL: unlike the outline, still opt-in — the built-in
        // default is OFF (`typewriter::TYPEWRITER_ON` starts false). A remembered
        // value applies unconditionally when present; absent leaves the global
        // OFF, so a plain launch (and a default `--screenshot`) keeps the
        // cursor-follow scroll → byte-identical.
        if let Some(on) = self.typewriter_scroll {
            crate::typewriter::set_typewriter_on(on);
        }
        // CJK AMBIGUITY LADDER: seed the live process global (`frontmatter::
        // cjk_priority()`, read by the Settings menu's "Ambiguous CJK reads as"
        // row) from a configured list, normalized to a well-formed 4-member
        // permutation. Absent config leaves the global at its own built-in
        // default (`DEFAULT_CJK_PRIORITY`), so a plain launch (and a default
        // `--screenshot`) is unaffected. The RENDER ladder is unaffected either
        // way — it stays `self.cjk_priority_or_default()`, read fresh.
        if let Some(v) = &self.cjk_priority {
            crate::frontmatter::set_cjk_priority(v);
        }
    }
}
