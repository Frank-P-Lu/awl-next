//! STRICT REPLAY TRUTHFULNESS — the ONE classification of every deferred
//! [`Effect`] the headless `--keys` replay can encounter, and the error/warning
//! surface both replay modes share.
//!
//! The headless replay (`main/run.rs::replay_keys`) drives the REAL
//! `apply_core` seam, but the core defers its filesystem/OS/window work as an
//! [`Effect`] for the caller — and the capture caller can only honestly perform
//! SOME of them. This module names that honesty in data:
//!
//!   * [`EffectClass::Applied`] — the replay performs the effect FOR REAL
//!     (or the effect is a cosmetic one-shot whose settled frame is
//!     byte-identical by contract, so there is nothing unperformed to observe).
//!   * [`EffectClass::Intercepted`] — an EXTERNAL handoff (open a URL, compose
//!     a mailto:, move a file to the OS Trash, a DOM download) is OBSERVED and
//!     RECORDED — payload included — but deliberately not performed. Recording
//!     rides [`Intercept`], the seam the scenario trace (phase 5) consumes.
//!   * [`EffectClass::Unsupported`] — live-App-only work the replay cannot
//!     perform, whose skip leaves the session in a DIFFERENT state than live
//!     (a config write that never lands, a rename that never happens). The
//!     strict mode aborts on these, naming the exact action + effect.
//!
//! The dividing line between the last two: an INTERCEPTED effect's skip changes
//! nothing about subsequent in-app state (the handoff leaves the editor as-is),
//! while an UNSUPPORTED effect's skip silently diverges the session from what
//! the same keys would do live. Truthfulness means the strict runner refuses to
//! continue past a divergence rather than verify a fiction.
//!
//! [`classify`] is a NO-WILDCARD match over [`Effect`] (and, for
//! [`Effect::OverlayAccept`], over [`OverlayKind`]): a future variant fails to
//! compile here until someone consciously classifies it. `main/run.rs`'s
//! replay loop consults this classification; the two can only drift if a human
//! edits one without the other, which the bucket-pinning tests below guard.
//!
//! MODES ([`Mode`]): the legacy one-off `--keys` flag stays PERMISSIVE (never
//! aborts; warns on stderr and records, so existing captures keep working
//! byte-identically apart from stderr). STRICT is the scenario-runner default
//! the later phases plumb through — exposed today via the opt-in
//! `--strict-replay` flag on `--screenshot --keys`. Strict also refuses an
//! unbound chord or dangling prefix sequence at replay time
//! (`keyspec::ChordResolver` — resolution moved INTO the replay loop so the
//! search guard can consume a chord first; an unparseable token still errors
//! at parse time via `keyspec::parse_chords`) and a missing layout oracle
//! before the first key ([`missing_oracle_error`]), so a strict run's motion
//! verdicts always rode the real wrap geometry.

use crate::actions::Effect;
use crate::keymap::Action;
use crate::overlay::OverlayKind;

/// How a replay treats the effects (and chords) it cannot honestly apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The legacy `--keys` door: never aborts. Crossing an Intercepted or
    /// Unsupported seam WARNS on stderr (and records the same line in the
    /// replay result, so the warning itself is testable).
    Permissive,
    /// The scenario-runner default: ABORT on any Unsupported effect, naming
    /// the exact action + effect ([`strict_error`]). Intercepted effects are
    /// recorded silently — observing a handoff without performing it IS the
    /// strict contract, not a compromise of it.
    Strict,
}

/// The truthfulness class of one [`Effect`] under headless replay. See the
/// module doc for the Applied / Intercepted / Unsupported dividing lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectClass {
    Applied,
    /// `detail` is the observed handoff payload (the URL, the trash-bound
    /// root-relative path, …) — empty when the effect carries none.
    Intercepted { detail: String },
    /// `why` names the live-App-only work the replay cannot perform, for the
    /// strict error / permissive warning.
    Unsupported { why: &'static str },
}

/// One classified effect: its stable snake_case `name` (used in errors,
/// warnings, and the phase-5 trace) plus its class.
pub struct Classified {
    pub name: &'static str,
    pub class: EffectClass,
}

/// One intercepted external handoff, recorded in replay order — the seam the
/// phase-5 scenario trace consumes (the trace FILE itself is a later phase;
/// this in-memory record is deliberately already in its vocabulary: a stable
/// effect name + the observed payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intercept {
    /// The effect's stable name ([`classify`]'s `name`), e.g. `"follow_link"`.
    pub effect: &'static str,
    /// The observed handoff payload (URL / trash path / …); `""` when the
    /// effect carries none.
    pub detail: String,
}

/// Classify one [`Effect`] — the ONE owner of the Applied / Intercepted /
/// Unsupported truth, consulted by the replay loop in `main/run.rs`. A
/// NO-WILDCARD match: a future `Effect` variant fails to compile here until it
/// is consciously classified (same for a future [`OverlayKind`], via
/// [`accept_class`]).
pub fn classify(effect: &Effect) -> Classified {
    let c = |name, class| Classified { name, class };
    let applied = EffectClass::Applied;
    let unsupported = |why| EffectClass::Unsupported { why };
    match effect {
        // ── APPLIED: the replay performs these for real (see the matching
        // arms in `main/run.rs::replay_keys_mode` / `capture_screenshot`). ──
        Effect::None => c("none", applied),
        Effect::NewNote => c("new_note", applied),
        Effect::OpenSettings => c("open_settings", applied),
        Effect::OpenCredits => c("open_credits", applied),
        Effect::OpenGuide => c("open_guide", applied),
        Effect::RunAction(_) => c("run_action", applied),
        Effect::OverlayAccept(kind, _) => c("overlay_accept", accept_class(*kind)),
        Effect::JumpToLine(_) => c("jump_to_line", applied),
        Effect::ConvertScratchAndSave => c("convert_scratch_and_save", applied),
        // INSERT DATE: the headless replay performs the SAME insert live does
        // (against the fixed placeholder date instead of the real clock — see
        // `dateformat::CAPTURE_PLACEHOLDER_YMD`), so this is honestly Applied,
        // not a divergence.
        Effect::InsertDate => c("insert_date", applied),
        // The save already ran inside `apply_core` (`Buffer::save`, through the
        // active fs backend); only the live bottom-center NOTICE is skipped —
        // chrome, not session state.
        Effect::SaveDone { .. } => c("save_done", applied),
        // Cosmetic caret one-shots: the underlying edit/copy already applied in
        // the core, and the flourish's settled frame is byte-identical BY
        // CONTRACT (each variant's own doc in `actions.rs`), so the skipped
        // animation is unobservable in any capture — Applied, not a gap.
        Effect::Recoil(_) => c("recoil", applied),
        Effect::TypeImpact => c("type_impact", applied),
        Effect::DeleteSquash => c("delete_squash", applied),
        Effect::Gulp => c("gulp", applied),
        Effect::LineLand => c("line_land", applied),
        Effect::CopyPulse => c("copy_pulse", applied),

        // ── INTERCEPTED: external handoffs, observed + recorded, safely not
        // performed — skipping them leaves the editor state exactly as live
        // (the handoff target is OUTSIDE the app). ──
        Effect::FollowLink(url) => c("follow_link", EffectClass::Intercepted { detail: url.clone() }),
        Effect::ReportProblem => c("report_problem", EffectClass::Intercepted { detail: String::new() }),
        Effect::DownloadFile => c("download_file", EffectClass::Intercepted { detail: String::new() }),
        // The export renders the document + writes a sibling file (or a web
        // download) — a live-App-only external write the replay/capture safely
        // skips, leaving the editor state exactly as live. Recorded, not performed.
        Effect::Export(format) => {
            c("export", EffectClass::Intercepted { detail: format.ext().to_string() })
        }
        Effect::CheckForUpdates => c("check_for_updates", EffectClass::Intercepted { detail: String::new() }),
        Effect::TrashAsset { rel } => c("trash_asset", EffectClass::Intercepted { detail: rel.clone() }),

        // ── UNSUPPORTED: live-App-only work whose skip diverges the session
        // from what the same keys do live — strict replay aborts here. ──
        Effect::Quit => c(
            "quit",
            // Live exits the event loop; the replay has none and would keep
            // applying LATER keys past the "exit" — a real divergence. A
            // future scenario runner may promote this to a clean stop instead.
            unsupported("live exits the event loop; a replay would keep applying keys past it"),
        ),
        Effect::LastBuffer => c(
            "last_buffer",
            unsupported("the 2-deep buffer history is live-App-only; the buffer switch would not happen"),
        ),
        Effect::NotesFlip => c(
            "notes_flip",
            unsupported(
                "the remembered pre-flip root (2-deep, like LastBuffer) and the actual root switch are live-App-only; the project would not change",
            ),
        ),
        Effect::FinishBuffer => c(
            "finish_buffer",
            unsupported("the daemon notify + switch-away are live-App-only (the save itself already ran)"),
        ),
        Effect::KeepVersion { .. } => c(
            "keep_version",
            unsupported("pinning (and naming) writes the local-history store, gated off the capture path"),
        ),
        Effect::AddToDictionary(_) => c(
            "add_to_dictionary",
            unsupported("silencing the word + appending it to the personal-dictionary file are live-App-only; the squiggle would not clear"),
        ),
        Effect::RebindCommit { .. } => c(
            "rebind_commit",
            unsupported("the config write + live keymap reload are live-App-only; the binding would not take effect"),
        ),
        Effect::RebindReset { .. } => c(
            "rebind_reset",
            unsupported("the config write + live keymap reload are live-App-only; the reset would not take effect"),
        ),
        Effect::SettingToggle { .. } => c(
            "setting_toggle",
            unsupported("flipping the live global + persisting it are live-App-only; the setting would not change"),
        ),
        Effect::SettingValueCommit { .. } => c(
            "setting_value_commit",
            unsupported("parse-clamp-apply-persist is live-App-only; the value would not take effect"),
        ),
        Effect::SettingPathPick { .. } => c(
            "setting_path_pick",
            unsupported("the config folder-key write is live-App-only; the path would not take effect"),
        ),
        Effect::RenameNoteCommit { .. } => c(
            "rename_note_commit",
            unsupported("the disk rename is live-App-only; the buffer would keep its old path"),
        ),
        Effect::DuplicateNote => c(
            "duplicate_note",
            unsupported("the sibling copy + buffer swap are live-App-only"),
        ),
    }
}

/// The per-[`OverlayKind`] class of an [`Effect::OverlayAccept`] — accepts are
/// the one effect whose truthfulness depends on WHICH picker emitted it. A
/// NO-WILDCARD match, mirroring [`OverlayKind::accept_disposition`]'s own law:
/// a future kind fails to compile until it declares whether its accept is
/// honestly applied headlessly.
fn accept_class(kind: OverlayKind) -> EffectClass {
    match kind {
        // Applied for real: Goto drives the multi-buffer registry switch
        // inline in the replay loop; Project re-roots and History restores in
        // `capture_screenshot`'s accept stage; Theme / Caret / Dictionary /
        // CjkLang / Date set their process-global CORE-level
        // (`actions/overlay_nav.rs`), so the replay session observes them exactly
        // as live does.
        OverlayKind::Goto
        | OverlayKind::Project
        | OverlayKind::History
        | OverlayKind::Theme
        | OverlayKind::Caret
        | OverlayKind::Dictionary
        | OverlayKind::CjkLang
        | OverlayKind::Date => EffectClass::Applied,
        // The note move (mkdir + rename under the notes root) is live-App-only;
        // headlessly the buffer keeps its old path — a divergence.
        OverlayKind::MoveDest => EffectClass::Unsupported {
            why: "the note move (mkdir + rename) is live-App-only; the buffer would keep its old path",
        },
        // These pickers never EMIT an `OverlayAccept` today (Browse re-routes
        // files through Goto; Command runs via `RunAction`; Spell edits in the
        // core; Keybindings/Settings/Assets/Rename/InsertLink ride their own
        // effects or core-internal edits — see `actions/overlay_nav.rs`).
        // Defaulted to Unsupported so a NEW emission aborts a strict run
        // loudly until someone classifies it, rather than silently passing.
        OverlayKind::Browse
        | OverlayKind::Command
        | OverlayKind::Spell
        | OverlayKind::Keybindings
        | OverlayKind::Settings
        | OverlayKind::Assets
        | OverlayKind::Rename
        | OverlayKind::InsertLink
        | OverlayKind::KeepName => EffectClass::Unsupported {
            why: "this picker is not expected to emit an accept effect; classify it in replay::accept_class before strict replay can pass it",
        },
    }
}

/// The permissive `--keys` warning line for a non-Applied effect — `None` for
/// Applied (the common case warns about nothing). The ONE owner of the warning
/// wording: `main/run.rs` prints exactly this string to stderr AND records it
/// in the replay result, so tests pin the same text users see.
pub fn warn_line(action: &Action, c: &Classified) -> Option<String> {
    match &c.class {
        EffectClass::Applied => None,
        EffectClass::Intercepted { detail } => {
            let payload = if detail.is_empty() { String::new() } else { format!(" ({detail})") };
            Some(format!(
                "--keys replay: intercepted `{}`{payload} from action {:?} — recorded, not performed",
                c.name, action
            ))
        }
        EffectClass::Unsupported { why } => Some(format!(
            "--keys replay: skipped unsupported effect `{}` from action {:?} — {}",
            c.name, action, why
        )),
    }
}

/// The strict-mode abort for an Unsupported effect: names the exact action AND
/// effect (the spec's contract), plus the live-App-only reason. Only ever
/// built for [`EffectClass::Unsupported`].
pub fn strict_error(action: &Action, c: &Classified) -> anyhow::Error {
    let why = match &c.class {
        EffectClass::Unsupported { why } => why,
        // `main/run.rs` only calls this on the Unsupported arm; a misuse still
        // produces an honest (if less specific) error rather than a panic.
        EffectClass::Applied | EffectClass::Intercepted { .. } => "not an unsupported effect",
    };
    anyhow::anyhow!(
        "strict replay: unsupported effect `{}` from action {:?} — {}",
        c.name,
        action,
        why
    )
}

/// The strict-mode abort for a missing LAYOUT ORACLE: without the offscreen
/// shaped pipeline (no GPU adapter), visual-line motion silently falls back to
/// LOGICAL lines — fine for the permissive door, a fiction the strict runner
/// refuses to verify against.
pub fn missing_oracle_error() -> anyhow::Error {
    anyhow::anyhow!(
        "strict replay: layout oracle unavailable (no GPU adapter) — \
         visual-line motion would fall back to logical lines instead of the shaped wrap geometry"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caret::RecoilDir;

    /// One sample instance of EVERY `Effect` variant (the compile-time
    /// exhaustiveness law lives in `classify`'s own no-wildcard match; this
    /// roster makes each variant's BUCKET explicit and reviewed).
    fn roster() -> Vec<Effect> {
        vec![
            Effect::None,
            Effect::Quit,
            Effect::LastBuffer,
            Effect::NotesFlip,
            Effect::NewNote,
            Effect::OpenSettings,
            Effect::OpenCredits,
            Effect::OpenGuide,
            Effect::RunAction(Action::Save),
            Effect::OverlayAccept(OverlayKind::Goto, "a.md".into()),
            Effect::JumpToLine(3),
            Effect::RebindCommit { slug: "save".into(), binding: "C-t".into(), confirmed: false },
            Effect::RebindReset { slug: "save".into() },
            Effect::Recoil(RecoilDir::Left),
            Effect::TypeImpact,
            Effect::DeleteSquash,
            Effect::Gulp,
            Effect::LineLand,
            Effect::FinishBuffer,
            Effect::KeepVersion { name: Some("draft A".into()) },
            Effect::FollowLink("https://example.com".into()),
            Effect::ReportProblem,
            Effect::DownloadFile,
            Effect::Export(crate::export::Format::Docx),
            Effect::CheckForUpdates,
            Effect::CopyPulse,
            Effect::SettingToggle { key: "wysiwyg".into() },
            Effect::SettingValueCommit { key: "page_width_prose".into(), value: "66".into() },
            Effect::SettingPathPick { key: "notes_root".into(), path: "/tmp/n".into() },
            Effect::TrashAsset { rel: "assets/orphan.png".into() },
            Effect::ConvertScratchAndSave,
            Effect::SaveDone { ok: true, message: "saved".into() },
            Effect::RenameNoteCommit { new_name: "new.md".into() },
            Effect::DuplicateNote,
            Effect::InsertDate,
        ]
    }

    #[test]
    fn every_effect_lands_in_its_documented_bucket() {
        // The bucket each variant belongs to, pinned by NAME (the classify
        // match is the compile-time sweep; this is the reviewed membership).
        let applied = [
            "none", "new_note", "open_settings", "open_credits", "open_guide", "run_action",
            "overlay_accept", "jump_to_line", "convert_scratch_and_save", "save_done", "recoil",
            "type_impact", "delete_squash", "gulp", "line_land", "copy_pulse", "insert_date",
        ];
        let intercepted = [
            "follow_link", "report_problem", "download_file", "export", "check_for_updates",
            "trash_asset",
        ];
        let unsupported = [
            "quit", "last_buffer", "notes_flip", "finish_buffer", "keep_version", "rebind_commit",
            "rebind_reset", "setting_toggle", "setting_value_commit", "setting_path_pick",
            "rename_note_commit", "duplicate_note",
        ];
        for e in roster() {
            let c = classify(&e);
            let expected: &[&str] = match c.class {
                EffectClass::Applied => &applied,
                EffectClass::Intercepted { .. } => &intercepted,
                EffectClass::Unsupported { .. } => &unsupported,
            };
            assert!(expected.contains(&c.name), "`{}` classified off its documented bucket", c.name);
        }
        // The three buckets partition the roster exactly (no name missing/extra).
        assert_eq!(roster().len(), applied.len() + intercepted.len() + unsupported.len());
    }

    #[test]
    fn effect_names_are_unique_and_stable() {
        let mut names: Vec<&'static str> = roster().iter().map(|e| classify(e).name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate effect name in classify");
    }

    #[test]
    fn intercepted_effects_carry_their_payload_as_detail() {
        let follow = classify(&Effect::FollowLink("https://awl.example/g".into()));
        assert_eq!(
            follow.class,
            EffectClass::Intercepted { detail: "https://awl.example/g".into() }
        );
        let trash = classify(&Effect::TrashAsset { rel: "assets/o.png".into() });
        assert_eq!(trash.class, EffectClass::Intercepted { detail: "assets/o.png".into() });
        // Payload-free handoffs record an empty detail, not a placeholder.
        let report = classify(&Effect::ReportProblem);
        assert_eq!(report.class, EffectClass::Intercepted { detail: String::new() });
    }

    #[test]
    fn overlay_accepts_are_classified_per_kind() {
        // The headlessly-real accepts stay Applied…
        for kind in [
            OverlayKind::Goto,
            OverlayKind::Project,
            OverlayKind::History,
            OverlayKind::Theme,
            OverlayKind::Caret,
            OverlayKind::Dictionary,
            OverlayKind::CjkLang,
        ] {
            let c = classify(&Effect::OverlayAccept(kind, "v".into()));
            assert_eq!(c.class, EffectClass::Applied, "{kind:?} accept should be Applied");
        }
        // …the live-only note move is Unsupported…
        let mv = classify(&Effect::OverlayAccept(OverlayKind::MoveDest, "inbox".into()));
        assert!(matches!(mv.class, EffectClass::Unsupported { .. }));
        // …and a kind that never emits an accept fails safe (Unsupported), so a
        // new emission aborts a strict run until consciously classified.
        let odd = classify(&Effect::OverlayAccept(OverlayKind::Spell, "word".into()));
        assert!(matches!(odd.class, EffectClass::Unsupported { .. }));
    }

    #[test]
    fn strict_error_and_warn_line_name_the_exact_action_and_effect() {
        let c = classify(&Effect::Quit);
        let err = strict_error(&Action::Quit, &c).to_string();
        assert!(err.contains("`quit`"), "effect named: {err}");
        assert!(err.contains("Quit"), "action named: {err}");
        assert!(err.starts_with("strict replay:"), "strict prefix: {err}");

        let warn = warn_line(&Action::Quit, &c).expect("unsupported warns");
        assert!(warn.contains("`quit`") && warn.contains("Quit"), "warn names both: {warn}");
        assert!(warn.starts_with("--keys replay:"), "permissive prefix: {warn}");

        // Intercepted warning carries the payload; Applied warns about nothing.
        let f = classify(&Effect::FollowLink("https://x.y/z".into()));
        let warn = warn_line(&Action::FollowLink, &f).expect("intercepted warns");
        assert!(warn.contains("`follow_link`") && warn.contains("https://x.y/z"), "{warn}");
        assert_eq!(warn_line(&Action::Save, &classify(&Effect::None)), None);
    }

    #[test]
    fn missing_oracle_error_names_the_fallback_it_refuses() {
        let msg = missing_oracle_error().to_string();
        assert!(msg.starts_with("strict replay:"), "{msg}");
        assert!(msg.contains("layout oracle"), "{msg}");
        assert!(msg.contains("logical lines"), "{msg}");
    }
}
