//! src/config/ — the PERSISTENT CONFIG, split by natural seam (2026-07
//! code-organization pass) out of the former `config.rs` monolith:
//! [`model`] (the [`Config`] data model + TOML parse), [`write`] (the
//! format-preserving disk writers, [`DEFAULT_TEMPLATE`] included), and
//! [`apply`] (the launch-time process-global apply). Every external path
//! (`config::Config`, `config::config_path`, `config::dictionary_name`, …)
//! is unchanged — this file only re-exports.
//!
//! awl's settings live in a text file you edit AS TEXT in awl itself (the
//! Settings command opens it; `Cmd-S` saves + live-reloads). The file is
//! TOML at `$XDG_CONFIG_HOME/awl/config.toml` (or `~/.config/awl/...`):
//!
//! ```toml
//! notes_root = "~/notes"
//! workspace  = "~/code"
//! [keys]
//! save         = ["Cmd-S", "C-x C-s"]  # up to 2 chords: native + your own emacs
//! switch_theme = "Cmd-T"               # a single chord still works
//! ```
//!
//! Every command takes UP TO 2 bindings (slot 1 = NATIVE/macOS, slot 2 = EMACS);
//! both fire. A `[keys]` value is therefore a LIST of up to 2 chords, or a single
//! string (the old form) for a one-chord rebind.
//!
//! PRECEDENCE is always explicit CLI flag > config file > built-in default, so an
//! ABSENT config (or any absent field) reproduces the current defaults exactly —
//! loading is purely additive and never changes behaviour on its own. The keymap
//! consumes [`Config::keys`] (see `keymap::KeymapState::with_overrides`); `main` /
//! `app` fold `notes_root`/`workspace` into the existing `resolve_*` paths.

mod apply;
mod model;
mod write;

pub use model::{caret_mode_name, config_path, dictionary_name, Config};
#[allow(unused_imports)] // parse_caret_mode/parse_dictionary: public API surface
// (the inverse of caret_mode_name/dictionary_name), reached in-crate via
// `config::model::` directly (apply.rs, tests.rs) rather than this re-export
// today — kept for the same reason theme's per-world consts stay exported.
pub use model::{parse_caret_mode, parse_dictionary};
#[allow(unused_imports)] // DEFAULT_TEMPLATE: public API surface (the documented
// starter file), reached in-crate via `config::write::DEFAULT_TEMPLATE`
// directly (write.rs's own writers) rather than this re-export today.
pub use write::DEFAULT_TEMPLATE;

#[cfg(test)]
mod tests;
