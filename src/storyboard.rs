//! src/storyboard.rs — STORYBOARD scenarios: a checked-in TOML file that drives
//! one hermetic strict-replay session end-to-end and the byte-stable TRACE the
//! run emits.
//!
//! A storyboard is the phase-5 face of the scenario harness: `press` / `type`
//! steps feed the SAME chord stream a `--keys` spec would (parsed by
//! [`crate::keyspec`], resolved through the real keymap inside the strict
//! replay loop), `pause` / `run_for` advance the VIRTUAL clock in fixed frame
//! steps (each tick a film frame — see `capture::film::FRAME_MS`), and
//! `expect` steps assert basic state (cursor / overlay kind / search state /
//! selection / a buffer-text fragment) against the live session. TOML because
//! that is the repo's one config idiom (`config.toml`, `session.toml`,
//! `keymap-defaults.toml`) — a taste call logged in the phase report.
//!
//! Parsing is STRICT where the user config is deliberately lenient: a typo'd
//! step key must abort the scenario at parse time, never silently no-op a step
//! (the harness's truthfulness contract — same reason the strict replay refuses
//! an unbound chord). Every `press`/`type` step's chords are validated
//! structurally here, before any rendering starts.
//!
//! The TRACE ([`Trace`] → [`render_trace`]) is hand-rolled JSON like the
//! capture sidecar (`capture/sidecar.rs` — no serde, so the emitted bytes stay
//! byte-stable): every chord's action + effect classification
//! (applied / intercepted / unsupported — [`crate::replay::classify`]'s
//! vocabulary), every assertion's outcome, and the abort (if any) naming the
//! offender. Repeated runs of the same storyboard produce a byte-identical
//! `trace.json` — the trace carries no clock, no path, no machine fact.

use anyhow::{bail, Context, Result};

use crate::keyspec::{self, Chord};

/// A parsed storyboard: the optional seed document (as written — relative paths
/// resolve against the storyboard file's own directory), an optional world to
/// render in, and the step list.
pub struct Storyboard {
    pub name: String,
    /// The document the scenario opens, exactly as the TOML spelled it
    /// (`None` = the scratch buffer). The CALLER resolves it against the
    /// storyboard's directory and seeds the hermetic sandbox with its bytes.
    pub file: Option<String>,
    /// Optional world name (`--theme`'s vocabulary); `None` keeps the default.
    pub theme: Option<String>,
    pub steps: Vec<StepKind>,
}

/// One storyboard step. Exactly one of the five TOML keys per `[[step]]`.
pub enum StepKind {
    /// Replay a `--keys` chord spec (one or more space-separated chords).
    Press { spec: String, chords: Vec<Chord> },
    /// Type literal text — each char the chord a `--keys` spec would spell
    /// ([`keyspec::text_chords`]), so typing rides the real keymap / search guard.
    Type { text: String, chords: Vec<Chord> },
    /// Advance the virtual clock `ms` milliseconds (fixed frame steps), letting
    /// whatever is animating settle. Reads as "wait".
    Pause { ms: u32 },
    /// Advance the virtual clock `ms` milliseconds — identical machinery to
    /// `pause`; reads as "let the animation run".
    RunFor { ms: u32 },
    /// Assert basic session state; the outcomes land in the trace.
    Expect(Expect),
}

impl StepKind {
    /// The step's stable kind tag for the trace.
    pub fn kind_str(&self) -> &'static str {
        match self {
            StepKind::Press { .. } => "press",
            StepKind::Type { .. } => "type",
            StepKind::Pause { .. } => "pause",
            StepKind::RunFor { .. } => "run_for",
            StepKind::Expect(_) => "expect",
        }
    }

    /// The step's input payload for the trace (the spec / text / ms; empty for
    /// an expect step, whose payload is its assert list).
    pub fn input_str(&self) -> String {
        match self {
            StepKind::Press { spec, .. } => spec.clone(),
            StepKind::Type { text, .. } => text.clone(),
            StepKind::Pause { ms } | StepKind::RunFor { ms } => ms.to_string(),
            StepKind::Expect(_) => String::new(),
        }
    }
}

/// An `expect` step's assertions — each present field is one checked assertion,
/// leaning on the sidecar's own state vocabulary (cursor line:col, the overlay
/// kind string with `"none"` for closed, the search panel's active/query pair,
/// selection presence, a buffer-text fragment).
#[derive(Default)]
pub struct Expect {
    pub cursor: Option<(usize, usize)>,
    pub overlay: Option<String>,
    pub search_active: Option<bool>,
    pub search_query: Option<String>,
    pub selection: Option<bool>,
    pub text_contains: Option<String>,
}

/// The session-state snapshot an `expect` step is checked against — a plain
/// data view so [`eval_expect`] stays pure/unit-testable (the runner builds it
/// from the live `ReplaySession`).
pub struct StateView {
    pub cursor: (usize, usize),
    /// The open overlay's kind string, or `"none"`.
    pub overlay: String,
    pub search_active: bool,
    pub search_query: String,
    pub selection: bool,
    pub text: String,
}

/// Evaluate one `expect` step: one [`AssertTrace`] per present field, in the
/// struct's fixed field order (stable trace bytes).
pub fn eval_expect(exp: &Expect, state: &StateView) -> Vec<AssertTrace> {
    let mut out = Vec::new();
    if let Some((l, c)) = exp.cursor {
        let (al, ac) = state.cursor;
        out.push(AssertTrace {
            check: "cursor",
            expected: format!("{l}:{c}"),
            actual: format!("{al}:{ac}"),
            pass: (al, ac) == (l, c),
        });
    }
    if let Some(kind) = &exp.overlay {
        out.push(AssertTrace {
            check: "overlay",
            expected: kind.clone(),
            actual: state.overlay.clone(),
            pass: &state.overlay == kind,
        });
    }
    if let Some(active) = exp.search_active {
        out.push(AssertTrace {
            check: "search_active",
            expected: active.to_string(),
            actual: state.search_active.to_string(),
            pass: state.search_active == active,
        });
    }
    if let Some(q) = &exp.search_query {
        out.push(AssertTrace {
            check: "search_query",
            expected: q.clone(),
            actual: state.search_query.clone(),
            pass: &state.search_query == q,
        });
    }
    if let Some(sel) = exp.selection {
        out.push(AssertTrace {
            check: "selection",
            expected: sel.to_string(),
            actual: state.selection.to_string(),
            pass: state.selection == sel,
        });
    }
    if let Some(frag) = &exp.text_contains {
        let present = state.text.contains(frag.as_str());
        out.push(AssertTrace {
            check: "text_contains",
            expected: frag.clone(),
            actual: if present { "present".into() } else { "absent".into() },
            pass: present,
        });
    }
    out
}

/// Parse a storyboard TOML. STRICT: unknown keys, a step with zero or two
/// action keys, an empty spec, or an unparseable chord all error here, naming
/// the offender — a storyboard typo must never silently skip a step.
/// `fallback_name` names the board when the TOML has no `name` (the caller
/// passes the file stem).
pub fn parse(src: &str, fallback_name: &str) -> Result<Storyboard> {
    let table: toml::Table =
        src.parse().map_err(|e| anyhow::anyhow!("storyboard TOML parse error: {e}"))?;
    for key in table.keys() {
        if !matches!(key.as_str(), "name" | "file" | "theme" | "step") {
            bail!("storyboard: unknown top-level key {key:?} (expected name/file/theme/step)");
        }
    }
    let str_key = |k: &str| -> Result<Option<String>> {
        match table.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => bail!("storyboard: `{k}` must be a string, got {v:?}"),
        }
    };
    let name = str_key("name")?.unwrap_or_else(|| fallback_name.to_string());
    let file = str_key("file")?;
    let theme = str_key("theme")?;
    let steps_raw = match table.get("step") {
        Some(toml::Value::Array(arr)) if !arr.is_empty() => arr,
        Some(toml::Value::Array(_)) | None => bail!("storyboard: no [[step]] entries"),
        Some(v) => bail!("storyboard: `step` must be [[step]] tables, got {v:?}"),
    };
    let mut steps = Vec::with_capacity(steps_raw.len());
    for (i, v) in steps_raw.iter().enumerate() {
        let t = v
            .as_table()
            .ok_or_else(|| anyhow::anyhow!("storyboard: step {i} is not a table"))?;
        steps.push(parse_step(t).with_context(|| format!("storyboard: step {i}"))?);
    }
    Ok(Storyboard { name, file, theme, steps })
}

/// One `[[step]]` table → its [`StepKind`]: exactly ONE of the five step keys,
/// nothing else beside it.
fn parse_step(t: &toml::Table) -> Result<StepKind> {
    const KEYS: [&str; 5] = ["press", "type", "pause", "run_for", "expect"];
    let present: Vec<&str> =
        KEYS.iter().copied().filter(|k| t.contains_key(*k)).collect();
    if present.len() != 1 {
        bail!("expected exactly one of press/type/pause/run_for/expect, found {present:?}");
    }
    if let Some(extra) = t.keys().find(|k| !KEYS.contains(&k.as_str())) {
        bail!("unknown step key {extra:?}");
    }
    let key = present[0];
    match (key, t.get(key).unwrap()) {
        ("press", toml::Value::String(spec)) => {
            let chords = keyspec::parse_chords(spec)?;
            if chords.is_empty() {
                bail!("`press` spec is empty");
            }
            Ok(StepKind::Press { spec: spec.clone(), chords })
        }
        ("type", toml::Value::String(text)) => {
            let chords = keyspec::text_chords(text)?;
            if chords.is_empty() {
                bail!("`type` text is empty");
            }
            Ok(StepKind::Type { text: text.clone(), chords })
        }
        ("pause", v) | ("run_for", v) => {
            let ms = v
                .as_integer()
                .filter(|&n| n > 0 && n <= 60_000)
                .ok_or_else(|| anyhow::anyhow!("`{key}` must be 1..=60000 (ms), got {v:?}"))?
                as u32;
            Ok(if key == "pause" { StepKind::Pause { ms } } else { StepKind::RunFor { ms } })
        }
        ("expect", toml::Value::Table(e)) => Ok(StepKind::Expect(parse_expect(e)?)),
        ("expect", v) => bail!("`expect` must be a table, got {v:?}"),
        (k, v) => bail!("`{k}` must be a string, got {v:?}"),
    }
}

/// The `expect` sub-table → [`Expect`]. Strict keys; at least one assertion.
fn parse_expect(t: &toml::Table) -> Result<Expect> {
    let mut exp = Expect::default();
    for (k, v) in t {
        match (k.as_str(), v) {
            ("cursor", toml::Value::Array(a)) => {
                let nums: Vec<usize> = a
                    .iter()
                    .map(|x| x.as_integer().filter(|&n| n >= 0).map(|n| n as usize))
                    .collect::<Option<Vec<_>>>()
                    .filter(|ns| ns.len() == 2)
                    .ok_or_else(|| {
                        anyhow::anyhow!("expect.cursor must be [line, col] (non-negative ints)")
                    })?;
                exp.cursor = Some((nums[0], nums[1]));
            }
            ("overlay", toml::Value::String(s)) => exp.overlay = Some(s.clone()),
            ("search_active", toml::Value::Boolean(b)) => exp.search_active = Some(*b),
            ("search_query", toml::Value::String(s)) => exp.search_query = Some(s.clone()),
            ("selection", toml::Value::Boolean(b)) => exp.selection = Some(*b),
            ("text_contains", toml::Value::String(s)) => exp.text_contains = Some(s.clone()),
            (k, v) => bail!("expect: unknown or mistyped key {k:?} = {v:?}"),
        }
    }
    if exp.cursor.is_none()
        && exp.overlay.is_none()
        && exp.search_active.is_none()
        && exp.search_query.is_none()
        && exp.selection.is_none()
        && exp.text_contains.is_none()
    {
        bail!("expect step asserts nothing");
    }
    Ok(exp)
}

// ── The trace: what the run DID, byte-stable ───────────────────────────────

/// The whole run's trace, rendered to `trace.json` by [`render_trace`].
pub struct Trace {
    /// The storyboard's name (never a path — the trace carries no machine fact).
    pub storyboard: String,
    /// The virtual clock's fixed frame step (ms/film frame).
    pub frame_ms: u32,
    pub steps: Vec<TraceStep>,
    /// A strict abort (unsupported effect / unbound chord / failed expectation),
    /// or `None` for a clean run.
    pub abort: Option<TraceAbort>,
}

/// One step's trace entry.
pub struct TraceStep {
    pub index: usize,
    /// [`StepKind::kind_str`].
    pub kind: &'static str,
    /// [`StepKind::input_str`].
    pub input: String,
    /// The inclusive film-frame range this step emitted, or `None` (expect
    /// steps render nothing).
    pub frames: Option<(u32, u32)>,
    /// Every chord the step replayed, with its action + effect classification.
    pub chords: Vec<ChordTrace>,
    /// Every assertion an expect step checked.
    pub asserts: Vec<AssertTrace>,
}

/// One replayed chord's record: which action it resolved to (or `None` when the
/// open search panel consumed it / it armed a prefix) and how its effect was
/// classified ([`crate::replay::classify`]'s stable names).
pub struct ChordTrace {
    pub chord: String,
    /// The resolved action's `Debug` form, or `None` (search-consumed / prefix).
    pub action: Option<String>,
    /// The effect's stable name (`"none"`, `"follow_link"`, …), or the two
    /// keymap-free outcomes: `"search_input"` / `"prefix"`.
    pub effect: String,
    /// `"applied"` / `"intercepted"` / `"unsupported"`.
    pub class: &'static str,
    /// An intercepted handoff's observed payload (URL / trash path); else `""`.
    pub detail: String,
}

/// One assertion's outcome.
pub struct AssertTrace {
    pub check: &'static str,
    pub expected: String,
    pub actual: String,
    pub pass: bool,
}

/// The abort record: which step died and the exact error (the same text the
/// process exits with, so the trace and stderr can never disagree).
pub struct TraceAbort {
    pub step: usize,
    pub reason: String,
}

/// Render the trace as deterministic JSON (`awl-trace/1`). Hand-rolled like the
/// capture sidecar — one string via the shared [`crate::capture::json_string`]
/// escaper — so the bytes are a pure function of the trace.
pub fn render_trace(t: &Trace) -> String {
    let jstr = crate::capture::json_string;
    let mut steps = String::new();
    for (i, s) in t.steps.iter().enumerate() {
        let frames = match s.frames {
            Some((a, b)) => format!("[{a}, {b}]"),
            None => "null".to_string(),
        };
        let chords: Vec<String> = s
            .chords
            .iter()
            .map(|c| {
                format!(
                    "{{ \"chord\": {}, \"action\": {}, \"effect\": {}, \"class\": {}, \"detail\": {} }}",
                    jstr(&c.chord),
                    c.action.as_deref().map(jstr).unwrap_or_else(|| "null".into()),
                    jstr(&c.effect),
                    jstr(c.class),
                    jstr(&c.detail),
                )
            })
            .collect();
        let asserts: Vec<String> = s
            .asserts
            .iter()
            .map(|a| {
                format!(
                    "{{ \"check\": {}, \"expected\": {}, \"actual\": {}, \"pass\": {} }}",
                    jstr(a.check),
                    jstr(&a.expected),
                    jstr(&a.actual),
                    a.pass,
                )
            })
            .collect();
        steps.push_str(&format!(
            "    {{ \"index\": {}, \"kind\": {}, \"input\": {}, \"frames\": {}, \"chords\": [{}], \"asserts\": [{}] }}{}\n",
            s.index,
            jstr(s.kind),
            jstr(&s.input),
            frames,
            join_multiline(&chords),
            join_multiline(&asserts),
            if i + 1 < t.steps.len() { "," } else { "" },
        ));
    }
    let abort = match &t.abort {
        Some(a) => format!(
            "{{ \"step\": {}, \"reason\": {} }}",
            a.step,
            crate::capture::json_string(&a.reason)
        ),
        None => "null".to_string(),
    };
    format!(
        "{{\n  \"schema\": \"awl-trace/1\",\n  \"storyboard\": {},\n  \"frame_ms\": {},\n  \"steps\": [\n{}  ],\n  \"abort\": {}\n}}\n",
        crate::capture::json_string(&t.storyboard),
        t.frame_ms,
        steps,
        abort,
    )
}

/// Join nested records one-per-line (readable diffs) — still fully
/// deterministic; an empty list stays `[]`-tight.
fn join_multiline(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!("\n      {}\n    ", items.join(",\n      "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = r#"
name = "mini"
file = "doc.md"
theme = "Tawny"

[[step]]
type = "hi"

[[step]]
press = "C-s"

[[step]]
pause = 40

[[step]]
run_for = 100

[[step]]
[step.expect]
cursor = [0, 2]
overlay = "none"
search_active = true
search_query = "hi"
selection = false
text_contains = "hi"
"#;

    #[test]
    fn parses_every_step_kind_and_the_header() {
        let b = parse(DEMO, "fallback").unwrap();
        assert_eq!(b.name, "mini");
        assert_eq!(b.file.as_deref(), Some("doc.md"));
        assert_eq!(b.theme.as_deref(), Some("Tawny"));
        let kinds: Vec<&str> = b.steps.iter().map(|s| s.kind_str()).collect();
        assert_eq!(kinds, vec!["type", "press", "pause", "run_for", "expect"]);
        match &b.steps[0] {
            StepKind::Type { text, chords } => {
                assert_eq!(text, "hi");
                assert_eq!(chords.len(), 2);
            }
            _ => panic!("step 0 is a type step"),
        }
        match &b.steps[4] {
            StepKind::Expect(e) => {
                assert_eq!(e.cursor, Some((0, 2)));
                assert_eq!(e.overlay.as_deref(), Some("none"));
                assert_eq!(e.search_active, Some(true));
                assert_eq!(e.text_contains.as_deref(), Some("hi"));
            }
            _ => panic!("step 4 is an expect step"),
        }
    }

    #[test]
    fn name_falls_back_to_the_callers_stem() {
        let b = parse("[[step]]\npress = \"C-n\"\n", "demo-file").unwrap();
        assert_eq!(b.name, "demo-file");
        assert_eq!(b.file, None);
        assert_eq!(b.theme, None);
    }

    #[test]
    fn strict_parse_rejects_typos_instead_of_skipping_steps() {
        // Unknown top-level key.
        assert!(parse("flie = \"x\"\n[[step]]\npress = \"a\"\n", "t").is_err());
        // A step with no action key, two action keys, or an unknown key.
        assert!(parse("[[step]]\nnote = \"x\"\n", "t").is_err());
        assert!(parse("[[step]]\npress = \"a\"\ntype = \"b\"\n", "t").is_err());
        assert!(parse("[[step]]\npress = \"a\"\nwait = 3\n", "t").is_err());
        // A garbled chord fails at parse time, before any rendering.
        assert!(parse("[[step]]\npress = \"frobnicate\"\n", "t").is_err());
        // Zero/negative/absurd durations.
        assert!(parse("[[step]]\npause = 0\n", "t").is_err());
        assert!(parse("[[step]]\nrun_for = -5\n", "t").is_err());
        // An expect step must assert SOMETHING, with known keys only.
        assert!(parse("[[step]]\n[step.expect]\n", "t").is_err());
        assert!(parse("[[step]]\n[step.expect]\ncursur = [0, 0]\n", "t").is_err());
        // No steps at all.
        assert!(parse("name = \"empty\"\n", "t").is_err());
    }

    #[test]
    fn eval_expect_reports_each_field_in_fixed_order() {
        let state = StateView {
            cursor: (2, 5),
            overlay: "command".into(),
            search_active: false,
            search_query: String::new(),
            selection: true,
            text: "the quick fox".into(),
        };
        let exp = Expect {
            cursor: Some((2, 5)),
            overlay: Some("command".into()),
            search_active: Some(true), // deliberate mismatch
            search_query: None,
            selection: Some(true),
            text_contains: Some("quick".into()),
        };
        let out = eval_expect(&exp, &state);
        let checks: Vec<&str> = out.iter().map(|a| a.check).collect();
        assert_eq!(checks, vec!["cursor", "overlay", "search_active", "selection", "text_contains"]);
        assert!(out[0].pass && out[1].pass && out[3].pass && out[4].pass);
        assert!(!out[2].pass, "the deliberate mismatch fails");
        assert_eq!(out[2].expected, "true");
        assert_eq!(out[2].actual, "false");
        assert_eq!(out[4].actual, "present");
    }

    #[test]
    fn render_trace_is_byte_stable_and_names_the_abort() {
        let trace = Trace {
            storyboard: "mini".into(),
            frame_ms: 20,
            steps: vec![
                TraceStep {
                    index: 0,
                    kind: "press",
                    input: "s-q".into(),
                    frames: None,
                    chords: vec![ChordTrace {
                        chord: "s-q".into(),
                        action: Some("Quit".into()),
                        effect: "quit".into(),
                        class: "unsupported",
                        detail: String::new(),
                    }],
                    asserts: vec![],
                },
                TraceStep {
                    index: 1,
                    kind: "expect",
                    input: String::new(),
                    frames: None,
                    chords: vec![],
                    asserts: vec![AssertTrace {
                        check: "cursor",
                        expected: "0:0".into(),
                        actual: "0:0".into(),
                        pass: true,
                    }],
                },
            ],
            abort: Some(TraceAbort { step: 0, reason: "strict replay: unsupported effect `quit`".into() }),
        };
        let a = render_trace(&trace);
        let b = render_trace(&trace);
        assert_eq!(a, b, "same trace, same bytes");
        assert!(a.contains("\"schema\": \"awl-trace/1\""));
        assert!(a.contains("\"class\": \"unsupported\""));
        assert!(a.contains("\"abort\": { \"step\": 0"));
        assert!(a.contains("unsupported effect `quit`"));
        // Parses as JSON-shaped enough for jq-style consumers: balanced braces.
        assert_eq!(a.matches('{').count(), a.matches('}').count());
    }
}
