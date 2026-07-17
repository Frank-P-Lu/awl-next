//! WRITING STREAKS — a quiet, entirely-LOCAL record of how much you write each
//! day, summoned as a year-calendar heatmap card (the contribution-graph shape).
//!
//! This module is the PURE data model + the calendar/intensity arithmetic + the
//! (de)serializer + the summoned-card open flag. It is deliberately TIMEZONE- and
//! CLOCK-FREE: every recording call takes the calendar day as an INJECTED
//! `"YYYY-MM-DD"` string (the `prune_ladder` precedent — a pure function of
//! `(state, day)`), so the whole boundary rule is unit-testable with no clock at
//! all. The App-side wiring (the live word-delta sampling on the autosave flush
//! triggers, and the local-calendar-day read) lives in [`crate::app`]'s
//! `app/streaks.rs`; the CARD RENDER lives in `render/chrome/hud.rs`, reusing the
//! HUD's float-card pipeline exactly like the About + Lifetime cards do.
//!
//! **What a day records (two figures, data is cheap — the visualization picks):**
//!  - `words` — the DAY TOTAL of NET WORDS ADDED, each per-flush delta clamped to
//!    0 before it is summed (deleting is editing, not un-writing, so a day you
//!    trimmed prose still reads as a writing day, never a negative one).
//!  - `raw_net` — the same deltas summed WITHOUT the clamp (can go negative on a
//!    heavy-cut day), kept alongside so a future readout can show true net churn.
//!
//! **Where it lives:** beside the scratch stash / session / stats / recent-* files
//! (`fs::data_root()/streaks.toml`), NOT inside `config.toml` — machine state the
//! app reads and writes as you work, the SAME reasoning + hand-rolled-writer /
//! real-`toml`-parser shape as [`crate::stats`] and [`crate::session`].
//!
//! **Determinism (CRITICAL):** the recording engine is native-only and lives ONLY
//! on the live `App` (armed like autosave), so a headless `--screenshot`/`--keys`
//! capture is STRUCTURALLY incapable of touching `streaks.toml`. The CARD is
//! drawn on any target, but a capture never pushes a live view, so it renders the
//! fixed synthetic [`placeholder`] year + fixed streak numbers — exactly the HUD
//! determinism boundary (`hud_stats` → `"—"`).

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// How many INTENSITY LEVELS a heatmap square can take: level 0 is EMPTY (no
/// words that day), levels 1..=4 are the filled quartiles. Five total — the
/// contribution-graph convention, and the count the tint law enforces
/// distinguishable steps across.
pub const LEVELS: usize = 5;

/// How many WEEK COLUMNS the year grid shows (a hair over a calendar year, the
/// contribution-graph shape). The last column is the current week.
pub const WEEKS: usize = 53;

/// Days per week column (Sunday row 0 … Saturday row 6).
pub const DAYS_PER_WEEK: usize = 7;

/// Total heatmap squares (`WEEKS × DAYS_PER_WEEK`).
pub const CELLS: usize = WEEKS * DAYS_PER_WEEK;

/// The summoned Writing-streaks card's OPEN flag — the shared summoned-card
/// mechanism (see [`crate::card::CardFlag`]), DEFAULT CLOSED. Summoned only via
/// the palette "Writing streaks" command / the `--streaks` capture flag.
static STREAKS: crate::card::CardFlag = crate::card::CardFlag::new();

/// True when the Writing streaks card is currently summoned.
pub fn streaks_open() -> bool {
    STREAKS.is_open()
}

/// Open or close the card explicitly (mirrors [`crate::lifetime::set_open`]).
pub fn set_open(open: bool) {
    STREAKS.set_open(open);
}

/// One calendar day's writing figures. `words` is the clamped-non-negative DAY
/// TOTAL (the heatmap intensity + the streak both key off it); `raw_net` is the
/// unclamped net (may be negative) kept for a future churn readout.
///
/// The whole RECORDING model ([`Streaks`] + the calendar/intensity arithmetic) is
/// native-only: the engine that fills it is native-only, and the wasm build only
/// ever draws the card from the fixed [`placeholder`] (never a live [`Streaks`]),
/// so gating it here keeps the wasm build warning-clean.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayWords {
    pub words: u64,
    pub raw_net: i64,
}

/// The whole persisted record: a calendar day (`"YYYY-MM-DD"`) → its figures. A
/// `BTreeMap` so iteration (and the serialized file) is chronological +
/// deterministic. A fresh install is empty.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streaks {
    pub days: BTreeMap<String, DayWords>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Streaks {
    /// Fold one per-flush word DELTA into `day`. The clamped-to-0 part sums into
    /// the DAY TOTAL (`words`) — a negative delta (a net-cut flush) contributes
    /// nothing rather than eroding the day; the RAW delta sums into `raw_net`
    /// (which may go negative). A day that nets exactly zero on both counters
    /// carries nothing and is dropped, so an idle flush never litters the file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn record_delta(&mut self, day: &str, delta: i64) {
        if delta == 0 {
            return;
        }
        let e = self.days.entry(day.to_string()).or_default();
        if delta > 0 {
            e.words = e.words.saturating_add(delta as u64);
        }
        e.raw_net = e.raw_net.saturating_add(delta);
        if e.words == 0 && e.raw_net == 0 {
            self.days.remove(day);
        }
    }

    /// The DAY TOTAL of net words for `day` (0 if never recorded).
    pub fn words_on(&self, day: &str) -> u64 {
        self.days.get(day).map(|d| d.words).unwrap_or(0)
    }

    /// The CURRENT STREAK ending at `today`: the count of consecutive calendar
    /// days with words written, ending today OR yesterday. Counting yesterday
    /// keeps a live streak from reading 0 all morning before you've written
    /// today — the standard contribution-graph semantics. 0 when neither today
    /// nor yesterday has any words.
    pub fn current_streak(&self, today: &str) -> u64 {
        let mut day = today.to_string();
        if self.words_on(&day) == 0 {
            day = prev_day(&day);
            if self.words_on(&day) == 0 {
                return 0;
            }
        }
        let mut count = 0u64;
        while self.words_on(&day) > 0 {
            count += 1;
            day = prev_day(&day);
        }
        count
    }

    /// The card's RENDER VIEW for a given `today`: the 371 heatmap buckets (ending
    /// at today's week), the current streak, and today's word total — everything
    /// the pixels + the sidecar need, computed once from this store.
    pub fn view(&self, today: &str) -> StreaksView {
        let mut cells = [0u8; CELLS];
        // The grid ends at the current week; column WEEKS-1 holds today. Sunday of
        // today's week is `today - dow`; the grid's first Sunday is (WEEKS-1) weeks
        // before that. A cell dated in the FUTURE (later this week than today) stays
        // level 0 — no writing has happened there yet.
        let today_days = match parse_ymd(today) {
            Some((y, m, d)) => days_from_civil(y, m, d),
            None => 0,
        };
        let dow = weekday(today_days); // 0 = Sunday … 6 = Saturday
        let grid_first_sunday = today_days - dow - ((WEEKS as i64 - 1) * DAYS_PER_WEEK as i64);
        // Peak words over the PAST-AND-TODAY window drives the quartile scaling, so
        // a light writer's few hundred words still lights the ladder's top rung.
        let mut peak = 0u64;
        let mut dates = [0i64; CELLS];
        for col in 0..WEEKS {
            for row in 0..DAYS_PER_WEEK {
                let idx = col * DAYS_PER_WEEK + row;
                let date_days =
                    grid_first_sunday + (col as i64) * DAYS_PER_WEEK as i64 + row as i64;
                dates[idx] = date_days;
                if date_days <= today_days {
                    let (y, m, d) = civil_from_days(date_days);
                    peak = peak.max(self.words_on(&fmt_ymd(y, m, d)));
                }
            }
        }
        for idx in 0..CELLS {
            if dates[idx] > today_days {
                continue; // future cell: stays empty
            }
            let (y, m, d) = civil_from_days(dates[idx]);
            cells[idx] = bucket(self.words_on(&fmt_ymd(y, m, d)), peak);
        }
        StreaksView {
            cells,
            streak: self.current_streak(today),
            today_words: self.words_on(today),
        }
    }
}

/// Everything the summoned card renders THIS frame: the heatmap buckets, the
/// streak count, and today's words. Built live from a [`Streaks`] store +
/// today's date, or the fixed [`placeholder`] in a headless capture.
#[derive(Debug, Clone, PartialEq)]
pub struct StreaksView {
    /// `WEEKS × DAYS_PER_WEEK` intensity buckets (0..=4), column-major: cell
    /// `col*7 + row`, row 0 = Sunday.
    pub cells: [u8; CELLS],
    pub streak: u64,
    pub today_words: u64,
}

/// The DETERMINISTIC SYNTHETIC view a headless capture renders (no live store, so
/// no clock/fs). A fixed, varied year (every intensity level present) + fixed
/// streak numbers — byte-stable across machines, exactly the HUD-placeholder
/// determinism boundary. NEVER used on the live App (which always has a store).
pub fn placeholder() -> StreaksView {
    let mut cells = [0u8; CELLS];
    for (i, c) in cells.iter_mut().enumerate() {
        // A cheap deterministic quadratic scramble, identical on every machine.
        // `i*(i+7) mod 11` visits exactly the residues {0,1,5,7,8,10} as `i`
        // ranges over the grid, so the map below is chosen to cover ALL FIVE
        // levels (every level therefore appears in the synthetic year, and the
        // 11/7 period lays them on a gentle diagonal rather than flat stripes).
        let v = (i * (i + 7)) % 11;
        *c = match v {
            0 | 10 => 0,
            1 => 1,
            5 => 2,
            7 => 3,
            _ => 4, // residue 8
        };
    }
    StreaksView { cells, streak: 12, today_words: 347 }
}

/// The intensity BUCKET (0..=4) for `words` against the window `peak`. 0 iff no
/// words; otherwise the quartile of `words/peak` (rounded UP so any writing lands
/// at least level 1). A zero `peak` (nothing written anywhere) leaves everything
/// at 0. (Native-only — the wasm build's card draws only the fixed [`placeholder`]
/// buckets, never a live [`Streaks::view`].)
#[cfg(not(target_arch = "wasm32"))]
pub fn bucket(words: u64, peak: u64) -> u8 {
    if words == 0 || peak == 0 {
        return 0;
    }
    let frac = (words as f32 / peak as f32).clamp(0.0, 1.0);
    ((frac * 4.0).ceil() as u8).clamp(1, 4)
}

/// The weekday of an epoch-day count: 0 = Sunday … 6 = Saturday. Day 0
/// (1970-01-01) was a Thursday (=4), so `(days + 4) mod 7` maps to this 0=Sunday
/// convention.
#[cfg(not(target_arch = "wasm32"))]
fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// Days since 1970-01-01 for a civil `(y, m, d)` — Howard Hinnant's
/// days-from-civil algorithm (the SAME pure conversion `history/picker.rs` and
/// `crashlog.rs` each carry their own copy of; the small duplication is the
/// accepted answer there, and one more keeps this module dependency-free +
/// wasm-safe). Valid for any Gregorian date.
#[cfg(not(target_arch = "wasm32"))]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`]: an epoch-day count back to civil
/// `(y, m, d)`. Howard Hinnant's civil-from-days.
#[cfg(not(target_arch = "wasm32"))]
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse a strict `"YYYY-MM-DD"` into `(y, m, d)`, or `None` if it isn't shaped
/// like one (lenient callers treat `None` as "unusable").
#[cfg(not(target_arch = "wasm32"))]
fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let mut it = s.split('-');
    let y = it.next()?.parse::<i64>().ok()?;
    let m = it.next()?.parse::<i64>().ok()?;
    let d = it.next()?.parse::<i64>().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// Format `(y, m, d)` as a zero-padded `"YYYY-MM-DD"`.
#[cfg(not(target_arch = "wasm32"))]
pub fn fmt_ymd(y: i64, m: i64, d: i64) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

/// The calendar day BEFORE `day` (`"YYYY-MM-DD"` in, one out). A malformed input
/// is returned unchanged (so a `while words_on > 0` walk over a bad key simply
/// terminates rather than looping).
#[cfg(not(target_arch = "wasm32"))]
pub fn prev_day(day: &str) -> String {
    match parse_ymd(day) {
        Some((y, m, d)) => {
            let (py, pm, pd) = civil_from_days(days_from_civil(y, m, d) - 1);
            fmt_ymd(py, pm, pd)
        }
        None => day.to_string(),
    }
}

/// The civil calendar day (`"YYYY-MM-DD"`) for an epoch-SECONDS stamp — pure, so
/// the live App feeds it `system_now()`-derived seconds ALREADY shifted by the
/// local UTC offset (see `app/streaks.rs`), keeping the timezone read at the one
/// live seam and this conversion unit-testable with no clock. `div_euclid` floors
/// toward negative infinity so a pre-epoch stamp still maps to the right day.
#[cfg(not(target_arch = "wasm32"))]
pub fn civil_date_from_epoch_secs(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    fmt_ymd(y, m, d)
}

/// Where the streaks file lives: beside the stats / session / recent-* files.
#[cfg(not(target_arch = "wasm32"))]
pub fn streaks_path() -> PathBuf {
    crate::fs::data_root().join("streaks.toml")
}

/// Load the persisted record from `path` through the active `FileSystem`. A
/// MISSING file degrades to an EMPTY [`Streaks`] (mirrors [`crate::stats::load`]);
/// a present-but-unparseable file is preserved to a `.corrupt-*` sibling first
/// (via [`crate::durable`]) before the same lenient default proceeds.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Streaks {
    crate::durable::load_toml_store(path, from_toml)
}

/// Persist `streaks` to `path` ATOMICALLY (temp-sibling + rename via
/// [`crate::fs::write_atomic`], the same primitive stats/session/autosave use).
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &Path, streaks: &Streaks) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = crate::fs::active().create_dir_all(parent);
    }
    crate::fs::write_atomic(path, to_toml(streaks).as_bytes())
}

/// Serialize to the on-disk TOML shape — pure, no fs. Hand-rolled (the crate's
/// `toml` dep loads parse-only), read back through the real parser in
/// [`from_toml`] so the two halves never hand-agree on escaping. One `[days]`
/// inline-table row per date, sorted (BTreeMap order → deterministic file).
#[cfg(not(target_arch = "wasm32"))]
pub fn to_toml(streaks: &Streaks) -> String {
    let mut out = String::new();
    if !streaks.days.is_empty() {
        out.push_str("[days]\n");
        for (day, w) in &streaks.days {
            out.push_str(&format!(
                "{} = {{ words = {}, raw = {} }}\n",
                quote(day),
                w.words,
                w.raw_net
            ));
        }
    }
    out
}

/// Parse the on-disk TOML back into a [`Streaks`] — pure, LENIENT throughout
/// (mirrors [`crate::stats::from_toml`]): an unparseable document, a
/// missing/wrong-typed field, or a malformed row is skipped rather than erroring,
/// so a half-written or hand-edited file never blocks a launch. A row that nets
/// entirely zero is dropped (nothing to carry).
#[cfg(not(target_arch = "wasm32"))]
pub fn from_toml(src: &str) -> Streaks {
    let mut streaks = Streaks::default();
    let Ok(table) = src.parse::<toml::Table>() else {
        return streaks;
    };
    if let Some(days) = table.get("days").and_then(|v| v.as_table()) {
        for (day, v) in days {
            let Some(row) = v.as_table() else { continue };
            let words = row.get("words").and_then(|x| x.as_integer()).unwrap_or(0).max(0) as u64;
            let raw = row.get("raw").and_then(|x| x.as_integer()).unwrap_or(0);
            if words == 0 && raw == 0 {
                continue;
            }
            streaks.days.insert(day.clone(), DayWords { words, raw_net: raw });
        }
    }
    streaks
}

/// A string as a quoted + escaped TOML basic string (the day key).
#[cfg(not(target_arch = "wasm32"))]
fn quote(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_closed() {
        let _g = crate::testlock::serial();
        set_open(false);
        assert!(!streaks_open(), "the Writing streaks card is closed by default");
        set_open(true);
        assert!(streaks_open());
        set_open(false);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn record_delta_clamps_the_day_total_but_keeps_raw_net() {
        let mut s = Streaks::default();
        s.record_delta("2026-07-17", 100);
        s.record_delta("2026-07-17", -30); // a net-cut flush
        s.record_delta("2026-07-17", 20);
        // Day total: only the positive deltas summed (100 + 20 = 120).
        assert_eq!(s.words_on("2026-07-17"), 120);
        // Raw net: all deltas summed (100 - 30 + 20 = 90).
        assert_eq!(s.days.get("2026-07-17").unwrap().raw_net, 90);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn record_delta_zero_records_nothing() {
        let mut s = Streaks::default();
        s.record_delta("2026-07-17", 0);
        assert!(s.days.is_empty(), "a zero delta records nothing");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn current_streak_counts_consecutive_days_and_tolerates_today_being_blank() {
        let mut s = Streaks::default();
        for day in ["2026-07-14", "2026-07-15", "2026-07-16"] {
            s.days.insert(day.to_string(), DayWords { words: 10, raw_net: 10 });
        }
        // Today (17th) blank but yesterday (16th) written → streak still alive = 3.
        assert_eq!(s.current_streak("2026-07-17"), 3);
        // Today written extends it to 4.
        s.days.insert("2026-07-17".to_string(), DayWords { words: 5, raw_net: 5 });
        assert_eq!(s.current_streak("2026-07-17"), 4);
        // A two-day gap (neither today nor yesterday) → 0.
        assert_eq!(s.current_streak("2026-07-19"), 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bucket_maps_quartiles_and_zero() {
        assert_eq!(bucket(0, 100), 0, "no words is always empty");
        assert_eq!(bucket(50, 0), 0, "zero peak leaves everything empty");
        assert_eq!(bucket(1, 100), 1, "any writing lands at least level 1");
        assert_eq!(bucket(25, 100), 1);
        assert_eq!(bucket(26, 100), 2);
        assert_eq!(bucket(50, 100), 2);
        assert_eq!(bucket(75, 100), 3);
        assert_eq!(bucket(76, 100), 4);
        assert_eq!(bucket(100, 100), 4, "the peak day is the top rung");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn civil_date_from_epoch_secs_maps_the_day() {
        // 2026-07-17 00:00:00 UTC = 1_784_246_400s; any second that day maps to it.
        let start = days_from_civil(2026, 7, 17) * 86_400;
        assert_eq!(civil_date_from_epoch_secs(start), "2026-07-17");
        assert_eq!(civil_date_from_epoch_secs(start + 86_399), "2026-07-17");
        assert_eq!(civil_date_from_epoch_secs(start + 86_400), "2026-07-18");
        // A negative (pre-epoch) stamp still floors to the right day.
        assert_eq!(civil_date_from_epoch_secs(-1), "1969-12-31");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn civil_date_round_trips_and_prev_day_steps_back() {
        // Epoch day 0 is 1970-01-01 (a Thursday → weekday 4).
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(0), 4);
        // Round trip a modern date.
        let d = days_from_civil(2026, 7, 17);
        assert_eq!(civil_from_days(d), (2026, 7, 17));
        // prev_day steps across a month boundary.
        assert_eq!(prev_day("2026-08-01"), "2026-07-31");
        assert_eq!(prev_day("2026-01-01"), "2025-12-31");
        // A malformed key is returned unchanged (walk-terminating).
        assert_eq!(prev_day("not-a-date"), "not-a-date");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn view_lays_today_in_the_last_column_and_leaves_the_future_empty() {
        let mut s = Streaks::default();
        s.days.insert("2026-07-17".to_string(), DayWords { words: 500, raw_net: 500 });
        let v = s.view("2026-07-17");
        // 2026-07-17 is a Friday → weekday 5. It sits in the LAST week column at
        // row 5; the peak day lights level 4.
        let today_days = days_from_civil(2026, 7, 17);
        assert_eq!(weekday(today_days), 5);
        let today_idx = (WEEKS - 1) * DAYS_PER_WEEK + 5;
        assert_eq!(v.cells[today_idx], 4, "today (the only writing day) is the peak rung");
        // Saturday of this week is in the future → empty.
        let future_idx = (WEEKS - 1) * DAYS_PER_WEEK + 6;
        assert_eq!(v.cells[future_idx], 0, "a day later this week than today stays empty");
        assert_eq!(v.streak, 1);
        assert_eq!(v.today_words, 500);
    }

    #[test]
    fn placeholder_is_deterministic_and_spans_every_level() {
        let a = placeholder();
        let b = placeholder();
        assert_eq!(a, b, "the synthetic year is byte-stable");
        for lvl in 0..LEVELS as u8 {
            assert!(a.cells.iter().any(|&c| c == lvl), "level {lvl} appears in the synthetic year");
        }
        assert_eq!(a.streak, 12);
        assert_eq!(a.today_words, 347);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn round_trips_through_toml_and_is_lenient() {
        let mut s = Streaks::default();
        s.days.insert("2026-07-16".to_string(), DayWords { words: 200, raw_net: 180 });
        s.days.insert("2026-07-17".to_string(), DayWords { words: 0, raw_net: -40 });
        assert_eq!(from_toml(&to_toml(&s)), s);
        // Garbage / empty degrade to an empty record, never a panic.
        assert_eq!(from_toml(""), Streaks::default());
        assert_eq!(from_toml("not valid toml {{{"), Streaks::default());
        assert_eq!(from_toml("other = 3\n"), Streaks::default());
        // A wrong-typed row is skipped.
        let lenient = from_toml("[days]\n\"2026-07-17\" = { words = \"lots\", raw = 5 }\n");
        assert_eq!(lenient.words_on("2026-07-17"), 0);
        assert_eq!(lenient.days.get("2026-07-17").map(|d| d.raw_net), Some(5));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_and_save_round_trip_through_in_memory_fs() {
        use std::sync::Arc;
        let fake = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fake, || {
            let path = PathBuf::from("/data/streaks.toml");
            assert_eq!(load(&path), Streaks::default(), "missing file: empty record");
            let mut s = Streaks::default();
            s.record_delta("2026-07-17", 250);
            save(&path, &s).unwrap();
            assert_eq!(load(&path), s);
        });
    }
}
