//! LIFETIME STATS — a quiet personal odometer. The persisted running counters
//! of how this awl has been used across its whole life: characters typed, keys
//! pressed, honest active-writing time, distinct files touched, the total pixel
//! distance the caret has travelled, and per-world active-writing time (which
//! theme world you actually live in). LOCAL and PRIVATE — a curiosity to peek
//! at in the HUD, never uploaded (reinforcing awl's zero-network invariant).
//!
//! This module is the PURE data model + the mutation primitives + the injected-
//! clock helpers + the (de)serializer; the App-side wiring (loading at launch,
//! the tracking hooks on the keyboard-input path / caret moves / file opens, and
//! the flush on the autosave triggers) lives in `app.rs` / `app/files.rs`. The
//! HUD DISPLAY is a separate phase — it only needs the store + `App::stats` +
//! this file's accessors.
//!
//! **Where it lives:** beside the scratch stash, the session file, and the two
//! recent-* MRUs (`fs::data_root()/stats.toml`), NOT inside `config.toml` — the
//! config file is the user's own hand-edited settings; this is MACHINE STATE the
//! app itself reads and writes as you work (the SAME reasoning as
//! [`crate::session`] / [`crate::recents`], and the SAME hand-rolled-TOML-writer
//! paired-with-the-real-`toml`-parser shape).
//!
//! **Determinism (CRITICAL):** every read/write goes through the `App`-side seam,
//! which is native-only (`cfg(not(target_arch = "wasm32"))`, like the daemon /
//! session restore — a quiet native-desktop odometer) and lives ONLY on the live
//! `App`. The headless `--screenshot`/`--keys` capture never constructs an `App`
//! (`main::run::replay_keys` / `load_buffer` build a bare `Buffer`), so a capture
//! is STRUCTURALLY incapable of reading or writing `stats.toml` — the tripwire
//! test `main::run::tests::headless_replay_never_touches_the_stats_file` proves
//! it. The HUD shows real values LIVE and fixed `"—"` placeholders in a capture,
//! exactly as the existing SESSION TIME / FILE CREATED HUD fields already do.

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

/// The honest-active-writing IDLE CAP: the most time one keystroke may attribute
/// to active writing. A THINK-PAUSE (staring at the page, re-reading a sentence)
/// under this cap counts as writing time; a WALK-AWAY longer than it stops
/// accruing until the next keystroke, so an editor left open overnight does not
/// bank eight hours of "writing". 2 minutes — generous enough that a real pause
/// mid-sentence is never clipped, tight enough that a coffee break is. Not
/// focused-time (which overcounts reading) and not session-time (which counts
/// idle) — the interval BETWEEN consecutive keystrokes, capped.
#[cfg(not(target_arch = "wasm32"))]
pub const IDLE_CAP_MS: u64 = 120_000;

/// GRADUATION threshold — a command GRADUATES once it has been invoked via its own
/// CHORD (the fast, learned path) at least this many times. A TASTE constant, start
/// at 5: enough repetitions that the chord is genuinely in the fingers, few enough
/// that a command in daily use graduates within a session or two. The discoverability
/// surfacing (phase 2) drops a graduated command from its "you could use the chord"
/// list — you already know it.
pub const GRADUATION_N: u64 = 5;

/// The DOOR a command dispatch came through — the three discoverability surfaces the
/// silent usage ledger attributes each invocation to. `Chord` is the FAST path (a
/// keyboard chord, OR a direct mouse gesture like right-click-to-suggest / double-
/// click-to-reset — a learned, deliberate invocation, not a browse); `Palette` (Cmd-P)
/// and `Menu` (the macOS menu bar) are the SLOW discovery surfaces graduation keys on
/// (`slow()` — "you keep browsing to find it"). Three variants exactly, per the round's
/// design law.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    Chord,
    Palette,
    Menu,
}

/// Per-command lifetime invocation counts, split by [`Door`] — one row of the
/// discoverability ledger. `Copy` (three `u64`s) so the queries hand it back by value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DoorCounts {
    pub chord: u64,
    pub palette: u64,
    pub menu: u64,
}

impl DoorCounts {
    /// Bump the count for `door` by one.
    #[cfg(not(target_arch = "wasm32"))]
    fn bump(&mut self, door: Door) {
        match door {
            Door::Chord => self.chord += 1,
            Door::Palette => self.palette += 1,
            Door::Menu => self.menu += 1,
        }
    }
    /// The SLOW-DOOR total — palette + menu presses. The "still discovering it" signal
    /// [`Stats::graduation_candidates`] ranks by; a chord press (the fast path) is
    /// deliberately excluded.
    pub fn slow(&self) -> u64 {
        self.palette + self.menu
    }
    /// Every invocation, all three doors.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn total(&self) -> u64 {
        self.chord + self.palette + self.menu
    }
    /// Whether this command has GRADUATED — invoked via its chord at least
    /// [`GRADUATION_N`] times (the fast path is in the fingers now).
    pub fn graduated(&self) -> bool {
        self.chord >= GRADUATION_N
    }
}

/// The whole persisted odometer. Every field is a monotonically-growing lifetime
/// total (they only ever increase); a fresh install starts every counter at zero
/// via [`Stats::default`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// Printable characters INSERTED (the `Action::InsertChar` path). A count of
    /// what you actually wrote, distinct from `keystrokes` (which also counts
    /// motions, chords, deletes).
    pub chars_typed: u64,
    /// ALL key presses that reached the editor past the lone-modifier / IME /
    /// preedit filters — printable inserts, motions, chords, everything.
    pub keystrokes: u64,
    /// Honest ACTIVE-WRITING time in milliseconds — the sum of the capped
    /// interval between consecutive keystrokes (see [`IDLE_CAP_MS`] and
    /// [`active_delta`]).
    pub active_writing_ms: u64,
    /// The DISTINCT set of file paths ever opened (touched via `App::load_path`,
    /// the same door the recent-files MRU rides). Stored as an insertion-ordered
    /// SET (deduped on insert) so `.len()` is the distinct-file count and the
    /// paths themselves survive for a future "files touched" listing.
    pub files_touched: Vec<PathBuf>,
    /// Total EUCLIDEAN pixel distance the caret has travelled (document space —
    /// so a big logical jump like Cmd-Down over a long file legitimately adds a
    /// lot, independent of scroll). An `f64` accumulator; presented later in a
    /// fun human unit (screens / metres) by the HUD phase.
    pub caret_distance_px: f64,
    /// Active-writing milliseconds attributed to the theme world that was active
    /// when they accrued (keyed by `Theme::name`) — the source for a
    /// "most-lived-in world" readout. Sums to `active_writing_ms`.
    pub per_world_ms: BTreeMap<String, u64>,
    /// THE SILENT USAGE LEDGER — per-command lifetime invocation counts, split by the
    /// [`Door`] each came through, keyed by the command's config SLUG
    /// (`commands::slug_for_action`). The discoverability signal: which commands you
    /// keep reaching via a slow door (palette / menu) but have a chord for, and which
    /// you have GRADUATED into the chord. Only catalog commands appear here (a motion /
    /// self-insert never keys a row). Recorded silently on the live App, surfaced ONLY
    /// where the user chooses to look — never a nudge.
    pub command_usage: BTreeMap<String, DoorCounts>,
}

/// The capped active-writing delta ONE keystroke contributes: the interval since
/// the previous keystroke, clamped to [`IDLE_CAP_MS`]. PURE — the injected-clock
/// core the live `App` feeds real millis into, so the idle-cap rule is testable
/// with no clock at all. A `now` before `last` (a clock that went backwards —
/// never expected from a monotonic source) contributes 0 rather than a huge
/// wrap, via `saturating_sub`.
#[cfg(not(target_arch = "wasm32"))]
pub fn active_delta(last_input_ms: u64, now_ms: u64) -> u64 {
    now_ms.saturating_sub(last_input_ms).min(IDLE_CAP_MS)
}

/// The euclidean distance between two caret positions — the per-move caret-travel
/// step. PURE, so the accumulation is testable without a pipeline. `f64` to match
/// the `caret_distance_px` accumulator and keep a long lifetime's running sum
/// from losing precision.
#[cfg(not(target_arch = "wasm32"))]
pub fn caret_step(from: (f32, f32), to: (f32, f32)) -> f64 {
    let dx = (to.0 - from.0) as f64;
    let dy = (to.1 - from.1) as f64;
    (dx * dx + dy * dy).sqrt()
}

impl Stats {
    /// Record ONE keystroke: bump `keystrokes`, bump `chars_typed` iff it was a
    /// printable insert, and fold the capped active-writing interval (see
    /// [`active_delta`]) into BOTH the global total and the active world's bucket.
    /// The clock is INJECTED (`last_input_ms`/`now_ms`, monotonic millis from the
    /// App's own session origin) so the whole rule is unit-testable; `None`
    /// `last_input_ms` (the very first keystroke of the session, or the first
    /// after a fresh load) contributes no interval, only the counter bumps.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn record_keystroke(
        &mut self,
        printable: bool,
        world: &str,
        last_input_ms: Option<u64>,
        now_ms: u64,
    ) {
        self.keystrokes += 1;
        if printable {
            self.chars_typed += 1;
        }
        if let Some(last) = last_input_ms {
            let delta = active_delta(last, now_ms);
            self.active_writing_ms += delta;
            if delta > 0 {
                *self.per_world_ms.entry(world.to_string()).or_default() += delta;
            }
        }
    }

    /// Add a caret MOVE's travel to the odometer (pure [`caret_step`] sum).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn record_caret_move(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.caret_distance_px += caret_step(from, to);
    }

    /// Record a file OPEN into the distinct-files set. Deduped (a re-open of an
    /// already-seen path is inert); returns whether the path was NEWLY added
    /// (so a caller can skip persisting when nothing changed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn touch_file(&mut self, path: PathBuf) -> bool {
        if self.files_touched.contains(&path) {
            return false;
        }
        self.files_touched.push(path);
        true
    }

    /// The distinct-files count — the odometer figure the HUD shows. (Consumed by
    /// the HUD DISPLAY phase; exercised by this module's + `app/stats.rs`' tests.)
    #[allow(dead_code)]
    pub fn files_touched_count(&self) -> usize {
        self.files_touched.len()
    }

    /// The world you have lived in most (the largest `per_world_ms` bucket), name
    /// + its millis; `None` when nothing has accrued yet. A tie resolves to the
    /// alphabetically-first name (`BTreeMap` iteration order) — deterministic.
    /// (Consumed by the HUD DISPLAY phase; exercised by this module's tests.)
    #[allow(dead_code)]
    pub fn most_used_world(&self) -> Option<(&str, u64)> {
        self.per_world_ms
            .iter()
            .max_by_key(|&(_, &ms)| ms)
            .map(|(name, &ms)| (name.as_str(), ms))
    }

    /// Record ONE command dispatch into the ledger: bump the per-[`Door`] count for the
    /// catalog command `slug`. The caller resolves `slug` from the dispatched `Action`
    /// (`commands::slug_for_action`), so a motion / self-insert / prefix never reaches
    /// here. Cheap: one map lookup + one integer bump (the key is only newly allocated
    /// the FIRST time a command is seen).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn record_command(&mut self, slug: String, door: Door) {
        self.command_usage.entry(slug).or_default().bump(door);
    }

    /// The recorded [`DoorCounts`] for command `slug` (all zeros if never invoked).
    pub fn command_counts(&self, slug: &str) -> DoorCounts {
        self.command_usage.get(slug).copied().unwrap_or_default()
    }

    /// Whether command `slug` has GRADUATED — its chord-door count has reached
    /// [`GRADUATION_N`] (see [`DoorCounts::graduated`]).
    #[allow(dead_code)]
    pub fn is_graduated(&self, slug: &str) -> bool {
        self.command_counts(slug).graduated()
    }

    /// The graduation CANDIDATES the discoverability surfacing (phase 2) renders:
    /// commands the user keeps reaching via a SLOW door (palette / menu) that HAVE a
    /// chord to graduate into and have NOT yet graduated — ranked most-slow-door-uses
    /// first, ties broken by slug (deterministic). `has_native_chord` reports whether a
    /// slug carries a native chord (catalog DATA — injected so this pure ledger query
    /// stays catalog-free + unit-testable, the single-owner seam the app fills from
    /// `commands::has_native_chord`); `top_n` caps the list. Surfaced only where the
    /// user chooses to look — never a nudge.
    #[allow(dead_code)]
    pub fn graduation_candidates(
        &self,
        has_native_chord: impl Fn(&str) -> bool,
        top_n: usize,
    ) -> Vec<(String, DoorCounts)> {
        let mut ranked: Vec<(String, DoorCounts)> = self
            .command_usage
            .iter()
            .filter(|(slug, c)| c.slow() > 0 && !c.graduated() && has_native_chord(slug))
            .map(|(slug, c)| (slug.clone(), *c))
            .collect();
        ranked.sort_by(|a, b| b.1.slow().cmp(&a.1.slow()).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(top_n);
        ranked
    }
}

/// Where the stats file lives: beside the scratch stash + session + recent-*
/// files, same data root.
#[cfg(not(target_arch = "wasm32"))]
pub fn stats_path() -> PathBuf {
    crate::fs::data_root().join("stats.toml")
}

/// Load the persisted odometer from `path` through the active `FileSystem`
/// backend. A MISSING or unparseable file degrades to an EMPTY [`Stats`] — never
/// an error, mirroring [`crate::session::load`]'s leniency.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Stats {
    match crate::fs::active().read_to_string(path) {
        Ok(src) => from_toml(&src),
        Err(_) => Stats::default(),
    }
}

/// Persist `stats` to `path` ATOMICALLY (temp-sibling + rename, via
/// [`crate::fs::write_atomic`] — the same primitive the autosave engine, the
/// scratch stash, the session file, and the recent-* MRUs use).
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &Path, stats: &Stats) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = crate::fs::active().create_dir_all(parent);
    }
    crate::fs::write_atomic(path, to_toml(stats).as_bytes())
}

/// Serialize `stats` to the on-disk TOML shape — pure, no fs. Hand-rolled
/// (mirrors `crate::session::to_toml`) since the crate's `toml` dependency loads
/// with only the `parse` feature (no serializer); reading it back goes through
/// the real `toml` parser via [`from_toml`], so the two halves never have to
/// hand-agree on escaping rules.
#[cfg(not(target_arch = "wasm32"))]
pub fn to_toml(stats: &Stats) -> String {
    let mut out = String::new();
    out.push_str(&format!("chars_typed = {}\n", stats.chars_typed));
    out.push_str(&format!("keystrokes = {}\n", stats.keystrokes));
    out.push_str(&format!("active_writing_ms = {}\n", stats.active_writing_ms));
    // A plain decimal; `to_string` never emits exponent/`inf`/`NaN` for the
    // finite non-negative sums this ever holds, and the `toml` parser reads it
    // back as an f64 float.
    out.push_str(&format!("caret_distance_px = {}\n", f64_toml(stats.caret_distance_px)));
    out.push_str("files_touched = [\n");
    for p in &stats.files_touched {
        out.push_str("  ");
        out.push_str(&quote(p));
        out.push_str(",\n");
    }
    out.push_str("]\n");
    if !stats.per_world_ms.is_empty() {
        out.push_str("\n[per_world]\n");
        for (world, ms) in &stats.per_world_ms {
            out.push_str(&format!("{} = {}\n", quote(Path::new(world)), ms));
        }
    }
    // The usage ledger: one inline table per command slug (BTreeMap → sorted, so the
    // file is deterministic). Read back via the real `toml` parser (inline tables parse
    // as tables) in `from_toml`, so the two halves never hand-agree on escaping.
    if !stats.command_usage.is_empty() {
        out.push_str("\n[command_usage]\n");
        for (slug, c) in &stats.command_usage {
            out.push_str(&format!(
                "{} = {{ chord = {}, palette = {}, menu = {} }}\n",
                quote(Path::new(slug)),
                c.chord,
                c.palette,
                c.menu
            ));
        }
    }
    out
}

/// A finite f64 as a TOML float literal that always round-trips as a float (never
/// a bare integer, which `toml` would parse back as an `Integer`): appends `.0`
/// when the value has no fractional part.
#[cfg(not(target_arch = "wasm32"))]
fn f64_toml(v: f64) -> String {
    if !v.is_finite() {
        return "0.0".to_string();
    }
    let s = v.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// A string as a quoted + escaped TOML basic string (used for both paths and
/// world/key names).
#[cfg(not(target_arch = "wasm32"))]
fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse the on-disk TOML shape back into a [`Stats`] — pure, no fs. LENIENT
/// throughout (mirrors `crate::session::from_toml`): an unparseable document, a
/// missing/wrong-typed field, or a malformed entry is simply skipped rather than
/// erroring, so a half-written or hand-edited stats file never blocks a launch.
/// A `caret_distance_px` written by an older build as a bare integer is read via
/// the integer fallback so it still round-trips.
#[cfg(not(target_arch = "wasm32"))]
pub fn from_toml(src: &str) -> Stats {
    let mut stats = Stats::default();
    let Ok(table) = src.parse::<toml::Table>() else {
        return stats;
    };
    let u64_of = |key: &str| -> u64 {
        table
            .get(key)
            .and_then(|v| v.as_integer())
            .unwrap_or(0)
            .max(0) as u64
    };
    stats.chars_typed = u64_of("chars_typed");
    stats.keystrokes = u64_of("keystrokes");
    stats.active_writing_ms = u64_of("active_writing_ms");
    stats.caret_distance_px = table
        .get("caret_distance_px")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .filter(|f| f.is_finite() && *f >= 0.0)
        .unwrap_or(0.0);
    if let Some(arr) = table.get("files_touched").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                // Dedup on load too, so a hand-edited duplicate never inflates
                // the distinct count.
                stats.touch_file(PathBuf::from(s));
            }
        }
    }
    if let Some(t) = table.get("per_world").and_then(|v| v.as_table()) {
        for (world, v) in t {
            if let Some(ms) = v.as_integer() {
                if ms > 0 {
                    stats.per_world_ms.insert(world.clone(), ms as u64);
                }
            }
        }
    }
    // The usage ledger: each command slug maps to an inline table of per-door counts
    // (a `toml` inline table parses as a `Table`). Lenient throughout — a missing
    // door defaults to 0, a wrong-typed count degrades to 0, an all-zero row is
    // dropped rather than stored (nothing to carry).
    if let Some(t) = table.get("command_usage").and_then(|v| v.as_table()) {
        for (slug, v) in t {
            if let Some(row) = v.as_table() {
                let door = |k: &str| -> u64 {
                    row.get(k).and_then(|x| x.as_integer()).unwrap_or(0).max(0) as u64
                };
                let counts =
                    DoorCounts { chord: door("chord"), palette: door("palette"), menu: door("menu") };
                if counts.total() > 0 {
                    stats.command_usage.insert(slug.clone(), counts);
                }
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut stats = Stats {
            chars_typed: 1234,
            keystrokes: 5678,
            active_writing_ms: 987_654,
            files_touched: vec![PathBuf::from("/home/me/a.md"), PathBuf::from("/home/me/b c.rs")],
            caret_distance_px: 42_195.5,
            per_world_ms: BTreeMap::new(),
            command_usage: BTreeMap::new(),
        };
        stats.per_world_ms.insert("Tawny".to_string(), 1000);
        stats.per_world_ms.insert("Mopoke".to_string(), 250);
        // The usage ledger round-trips too (inline-table per slug).
        stats.command_usage.insert(
            "go_to_file".to_string(),
            DoorCounts { chord: 12, palette: 3, menu: 1 },
        );
        stats
            .command_usage
            .insert("switch_theme".to_string(), DoorCounts { chord: 0, palette: 7, menu: 2 });
        assert_eq!(from_toml(&to_toml(&stats)), stats);
    }

    #[test]
    fn caret_distance_round_trips_as_a_float_even_when_whole() {
        // A whole-number distance must still read back as the same f64, not be
        // dropped by the parser treating a bare `123` as an integer.
        let stats = Stats { caret_distance_px: 5000.0, ..Stats::default() };
        let back = from_toml(&to_toml(&stats));
        assert_eq!(back.caret_distance_px, 5000.0);
    }

    #[test]
    fn empty_or_garbage_toml_yields_default() {
        assert_eq!(from_toml(""), Stats::default());
        assert_eq!(from_toml("not valid toml {{{"), Stats::default());
        assert_eq!(from_toml("other = 3\n"), Stats::default());
    }

    #[test]
    fn negative_or_wrong_typed_fields_degrade_to_zero() {
        // A hand-edited garbage value never crashes or reads negative.
        let s = from_toml("chars_typed = -5\nkeystrokes = \"lots\"\ncaret_distance_px = true\n");
        assert_eq!(s.chars_typed, 0);
        assert_eq!(s.keystrokes, 0);
        assert_eq!(s.caret_distance_px, 0.0);
    }

    #[test]
    fn load_and_save_round_trip_through_in_memory_fs() {
        use std::sync::Arc;
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            let path = PathBuf::from("/data/stats.toml");
            assert_eq!(load(&path), Stats::default(), "missing file: empty stats");
            let stats = Stats { chars_typed: 99, keystrokes: 100, ..Stats::default() };
            save(&path, &stats).unwrap();
            assert_eq!(load(&path), stats);
        });
    }

    #[test]
    fn active_delta_counts_a_think_pause_and_caps_a_walk_away() {
        // A 15-second think-pause counts in full (under the 2-minute cap).
        assert_eq!(active_delta(1_000, 1_000 + 15_000), 15_000);
        // A 5-minute walk-away is CAPPED at IDLE_CAP_MS — the odometer never
        // banks time you were not actually at the keyboard.
        assert_eq!(active_delta(1_000, 1_000 + 300_000), IDLE_CAP_MS);
        // Exactly at the cap: the full cap, not clipped further.
        assert_eq!(active_delta(0, IDLE_CAP_MS), IDLE_CAP_MS);
        // A backwards clock contributes nothing rather than a huge wrap.
        assert_eq!(active_delta(5_000, 1_000), 0);
    }

    #[test]
    fn record_keystroke_attributes_capped_active_time_to_the_world() {
        let mut stats = Stats::default();
        // First keystroke of the session: only the counters bump (no interval).
        stats.record_keystroke(true, "Tawny", None, 1_000);
        assert_eq!(stats.keystrokes, 1);
        assert_eq!(stats.chars_typed, 1);
        assert_eq!(stats.active_writing_ms, 0);
        assert!(stats.per_world_ms.is_empty(), "no interval on the first keystroke");
        // A 15s think-pause under Tawny then a 5-minute walk-away under Mopoke:
        // the first counts in full, the second caps.
        stats.record_keystroke(false, "Tawny", Some(1_000), 1_000 + 15_000);
        stats.record_keystroke(true, "Mopoke", Some(16_000), 16_000 + 300_000);
        assert_eq!(stats.keystrokes, 3);
        assert_eq!(stats.chars_typed, 2, "only the two printable presses");
        assert_eq!(stats.active_writing_ms, 15_000 + IDLE_CAP_MS);
        assert_eq!(stats.per_world_ms.get("Tawny"), Some(&15_000));
        assert_eq!(stats.per_world_ms.get("Mopoke"), Some(&IDLE_CAP_MS));
        assert_eq!(stats.most_used_world(), Some(("Mopoke", IDLE_CAP_MS)));
    }

    #[test]
    fn caret_distance_accumulates_a_pure_euclidean_sum() {
        let mut stats = Stats::default();
        // A 3-4-5 triangle then a straight 10 down: 5 + 10 = 15.
        stats.record_caret_move((0.0, 0.0), (3.0, 4.0));
        stats.record_caret_move((3.0, 4.0), (3.0, 14.0));
        assert!((stats.caret_distance_px - 15.0).abs() < 1e-9);
        // A no-move step adds nothing.
        stats.record_caret_move((3.0, 14.0), (3.0, 14.0));
        assert!((stats.caret_distance_px - 15.0).abs() < 1e-9);
    }

    #[test]
    fn touch_file_dedupes_and_counts_distinct_paths() {
        let mut stats = Stats::default();
        assert!(stats.touch_file(PathBuf::from("/a.md")), "first open is new");
        assert!(stats.touch_file(PathBuf::from("/b.md")), "second distinct open is new");
        assert!(!stats.touch_file(PathBuf::from("/a.md")), "a re-open is not new");
        assert_eq!(stats.files_touched_count(), 2, "distinct count, not open count");
    }

    #[test]
    fn record_command_attributes_each_door_separately() {
        let mut stats = Stats::default();
        stats.record_command("go_to_file".into(), Door::Chord);
        stats.record_command("go_to_file".into(), Door::Chord);
        stats.record_command("go_to_file".into(), Door::Palette);
        stats.record_command("go_to_file".into(), Door::Menu);
        let c = stats.command_counts("go_to_file");
        assert_eq!((c.chord, c.palette, c.menu), (2, 1, 1), "each door counts independently");
        assert_eq!(c.total(), 4);
        assert_eq!(c.slow(), 2, "slow = palette + menu, never chord");
        // A never-seen command reads all zeros.
        assert_eq!(stats.command_counts("nope"), DoorCounts::default());
    }

    #[test]
    fn graduation_triggers_at_the_threshold_on_chord_presses_only() {
        let mut stats = Stats::default();
        // Slow-door presses NEVER graduate, however many.
        for _ in 0..100 {
            stats.record_command("save".into(), Door::Palette);
        }
        assert!(!stats.is_graduated("save"), "slow-door use never graduates a command");
        // Chord presses graduate exactly at GRADUATION_N.
        for i in 1..=GRADUATION_N {
            stats.record_command("save".into(), Door::Chord);
            assert_eq!(
                stats.is_graduated("save"),
                i >= GRADUATION_N,
                "graduates on the GRADUATION_N-th chord press, not before"
            );
        }
        assert!(stats.is_graduated("save"));
    }

    #[test]
    fn graduation_candidates_rank_slow_door_use_and_exclude_chordless_and_graduated() {
        let mut stats = Stats::default();
        // A command reached often via the palette, with a chord, not yet graduated.
        for _ in 0..4 {
            stats.record_command("go_to_file".into(), Door::Palette);
        }
        // Another, reached via the menu twice — fewer slow uses, so ranks below.
        stats.record_command("save".into(), Door::Menu);
        stats.record_command("save".into(), Door::Menu);
        // Already GRADUATED (chord in the fingers) — excluded even with slow uses.
        for _ in 0..GRADUATION_N {
            stats.record_command("switch_theme".into(), Door::Chord);
        }
        stats.record_command("switch_theme".into(), Door::Palette);
        // Has NO native chord — excluded (nothing to graduate into) despite slow uses.
        for _ in 0..9 {
            stats.record_command("settings".into(), Door::Palette);
        }
        // Chord-only (no slow uses) — excluded (not "still discovering it").
        stats.record_command("copy".into(), Door::Chord);

        // Injected catalog truth: only these three carry a native chord.
        let has_chord = |slug: &str| {
            matches!(slug, "go_to_file" | "save" | "switch_theme" | "copy")
        };
        let ranked = stats.graduation_candidates(has_chord, 10);
        let slugs: Vec<&str> = ranked.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["go_to_file", "save"],
            "ranked by slow-door count; chordless + graduated + chord-only all excluded"
        );
        assert_eq!(ranked[0].1.slow(), 4);
        assert_eq!(ranked[1].1.slow(), 2);
        // top_n caps the list.
        assert_eq!(stats.graduation_candidates(has_chord, 1).len(), 1);
    }

    #[test]
    fn graduation_candidate_ties_break_by_slug_deterministically() {
        let mut stats = Stats::default();
        stats.record_command("bbb".into(), Door::Palette);
        stats.record_command("aaa".into(), Door::Palette);
        stats.record_command("ccc".into(), Door::Menu);
        let ranked = stats.graduation_candidates(|_| true, 10);
        let slugs: Vec<&str> = ranked.iter().map(|(s, _)| s.as_str()).collect();
        // Equal slow counts (1 each) → alphabetical by slug, stable across runs.
        assert_eq!(slugs, vec!["aaa", "bbb", "ccc"]);
    }
}
