//! DATE FORMATTING for "Insert Date" (Cmd-P) + the Settings menu's "Date
//! format" cycling row — a small, CLOSED set of five hand-rolled formats (no
//! date/time crate anywhere in the tree, no locale sniffing, no free-form
//! pattern strings).
//!
//! [`DateFormat`] is the ONE owner of the five arms + their cycle order; its
//! [`DateFormat::format`] method is a PURE function of `(year, month, day)` —
//! no clock read, unit-tested with fixed dates below. The chosen format is a
//! process-global ([`active_format`]/[`set_active_format`]), mirroring
//! `caret::mode()`/`spell::active_variant()`: seeded once from `Config` at
//! launch (`Config::apply_sticky_globals`), read live everywhere (the
//! renderer never exists here — only the Settings row + Insert Date do), and
//! cycled + persisted by the Settings menu's "Date format" row
//! (`App::cycle_date_format`, `settings.rs`).
//!
//! DETERMINISM: "today" is NOT read by anything in this module — the pure
//! formatter takes explicit `(year, month, day)`. The one clock-touching
//! helper, [`today_from_system_clock`], is LIVE-ONLY in spirit (the headless
//! capture path never calls it); it exists here only so both live call sites
//! (`App::insert_date`, the Settings overlay's live "today" preview) share
//! ONE conversion. The headless capture path uses the FIXED
//! [`CAPTURE_PLACEHOLDER_YMD`] instead — mirroring the HUD's session-time /
//! file-created placeholders (`hud::PLACEHOLDER`) and History's
//! `history_now: Option<u64>` seam (`docs/platform.md`), so a `--keys
//! "... Insert Date"` replay and the Settings row's live-preview example are
//! byte-identical across runs and machines.

use std::sync::atomic::{AtomicU8, Ordering};

/// The FIVE date-insert formats, in CYCLE order (also the settings row's
/// stepping order and [`ALL`]'s iteration order — one owner). No free-form
/// pattern strings: this is the whole closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateFormat {
    /// `22/07/26` — DD/MM/YY. THE DEFAULT.
    DdMmYy,
    /// `07/22/26` — MM/DD/YY.
    MmDdYy,
    /// `2026-07-22` — ISO 8601.
    Iso,
    /// `2026/07/22` — YYYY/MM/DD.
    YyyyMmDd,
    /// `22 July 2026` — D Month YYYY (day unpadded, full English month name).
    DMonthYyyy,
}

impl Default for DateFormat {
    fn default() -> Self {
        DateFormat::DdMmYy
    }
}

/// The full English month name table `[Jan..Dec]`, indexed `month - 1`. The
/// ONE place a month name is spelled — [`DateFormat::format`]'s `DMonthYyyy`
/// arm reads it, and the "all 12 month names" unit test below walks it.
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl DateFormat {
    /// Every format, in CYCLE order — the settings row's "Date format" Enter
    /// steps through exactly this array, wrapping ([`Self::cycle_next`]).
    pub const ALL: [DateFormat; 5] = [
        DateFormat::DdMmYy,
        DateFormat::MmDdYy,
        DateFormat::Iso,
        DateFormat::YyyyMmDd,
        DateFormat::DMonthYyyy,
    ];

    /// The persisted config-key SLUG (`date_format = "ddmmyy"` etc.) — the
    /// wire form both `Config::apply_sticky_globals` (parse, via
    /// [`Self::from_config_name`]) and `App::cycle_date_format`'s persist
    /// write share, mirroring `config::caret_mode_name`/`dictionary_name`.
    pub fn config_name(self) -> &'static str {
        match self {
            DateFormat::DdMmYy => "ddmmyy",
            DateFormat::MmDdYy => "mmddyy",
            DateFormat::Iso => "iso",
            DateFormat::YyyyMmDd => "yyyymmdd",
            DateFormat::DMonthYyyy => "dmonthyyyy",
        }
    }

    /// Resolve a persisted slug back to a [`DateFormat`] — the inverse of
    /// [`Self::config_name`]. `None` for an unrecognized string (a typo, a
    /// stale/foreign value), which the lenient config loader then reads as
    /// "unset" exactly like `caret_mode`/`dictionary`'s own leniency — never
    /// a parse error, never a crash.
    pub fn from_config_name(s: &str) -> Option<DateFormat> {
        Self::ALL.into_iter().find(|f| f.config_name() == s)
    }

    /// The human-readable NAME of this format — drawn as the quiet secondary
    /// (description) column of the Date-format PICKER, beside the row's live
    /// example date. Mirrors `caret::CaretMode::label` / `spell::DictVariant::
    /// label`: a capitalized reading name, distinct from the lower-case wire
    /// [`Self::config_name`]. The row's PRIMARY text is the example date itself
    /// (what-you-see-is-what-inserts), so this only has to orient which ordering
    /// each example is.
    pub fn label(self) -> &'static str {
        match self {
            DateFormat::DdMmYy => "Day / Month / Year",
            DateFormat::MmDdYy => "Month / Day / Year",
            DateFormat::Iso => "ISO 8601",
            DateFormat::YyyyMmDd => "Year / Month / Day",
            DateFormat::DMonthYyyy => "Day Month Year",
        }
    }

    /// The NEXT format in [`Self::ALL`]'s cycle order, wrapping past the end
    /// back to [`DateFormat::DdMmYy`] — what Enter on the Settings menu's
    /// "Date format" row does (`App::cycle_date_format`).
    pub fn cycle_next(self) -> DateFormat {
        let i = Self::ALL
            .iter()
            .position(|f| *f == self)
            .expect("DateFormat::ALL lists every variant");
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// THE PURE FORMATTER: render civil date `(year, month, day)` in this
    /// format. No clock read, no locale table beyond the fixed English
    /// [`MONTH_NAMES`] — a `month`/`day` outside the sane 1..=12 / 1..=31
    /// civil range still renders (whatever the arithmetic/indexing yields)
    /// rather than panicking; every real caller passes an actual civil date
    /// ([`today_from_system_clock`] / [`CAPTURE_PLACEHOLDER_YMD`]).
    pub fn format(self, year: i32, month: u32, day: u32) -> String {
        // `YY` is the year mod 100, always rendered as two digits (a negative
        // `year` — pre-1 BCE civil dates — is not a real case here, but
        // `rem_euclid` keeps the result non-negative regardless).
        let yy = year.rem_euclid(100);
        match self {
            DateFormat::DdMmYy => format!("{day:02}/{month:02}/{yy:02}"),
            DateFormat::MmDdYy => format!("{month:02}/{day:02}/{yy:02}"),
            DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::YyyyMmDd => format!("{year:04}/{month:02}/{day:02}"),
            DateFormat::DMonthYyyy => {
                let name = MONTH_NAMES.get(month.wrapping_sub(1) as usize).copied().unwrap_or("?");
                format!("{day} {name} {year:04}")
            }
        }
    }
}

/// The user's chosen format — 0 = unset (reads as the [`DateFormat::default`]
/// DD/MM/YY), 1..=5 = `ALL[n-1]`. A process-global like `caret::MODE_OVERRIDE`
/// / `theme`'s active index: every reader (the Settings row, Insert Date, the
/// sidecar) consults the SAME cell, seeded once from `Config` at launch
/// (`Config::apply_sticky_globals`) and updated live by
/// `App::cycle_date_format`.
static ACTIVE_FORMAT: AtomicU8 = AtomicU8::new(0);

fn format_to_u8(f: DateFormat) -> u8 {
    // +1 keeps sentinel 0 reserved for genuine unset; `position` is always Some by
    // ALL's exhaustiveness.
    DateFormat::ALL.iter().position(|c| *c == f).expect("DateFormat::ALL lists every variant") as u8
        + 1
}

/// The EFFECTIVE date-insert format right now.
pub fn active_format() -> DateFormat {
    let raw = ACTIVE_FORMAT.load(Ordering::Relaxed);
    if raw == 0 {
        return DateFormat::default();
    }
    DateFormat::ALL.get(raw as usize - 1).copied().unwrap_or_default()
}

/// Set the active date-insert format (the Settings row's cycle commit, or the
/// launch-time seed from `Config`).
pub fn set_active_format(f: DateFormat) {
    ACTIVE_FORMAT.store(format_to_u8(f), Ordering::Relaxed);
}

/// The FIXED, numberless-in-spirit placeholder civil date every HEADLESS
/// capture renders "today" as — mirroring the HUD's session-time/file-created
/// `"—"` placeholder (`hud::PLACEHOLDER`), except here a REAL (if arbitrary)
/// date is more useful than a dash: it lets the settings-row example / a
/// `--keys "... Insert Date"` capture show actual formatted digits, so a
/// reviewer can eyeball DD/MM vs MM/DD ordering and zero-padding straight
/// from the sidecar/PNG. Deliberately NOT tied to any real "today" (which
/// would make a capture differ day-to-day) — a single-digit day AND month (7,
/// 3) so every format's zero-padding is exercised, and day != month so a
/// DD/MM <-> MM/DD swap is visually obvious.
pub const CAPTURE_PLACEHOLDER_YMD: (i32, u32, u32) = (2009, 3, 7);

/// Epoch-day count (days since 1970-01-01, UTC) -> civil `(year, month,
/// day)`. Howard Hinnant's `civil_from_days` algorithm (public domain) — the
/// SAME arithmetic `crate::streaks::civil_from_days` uses natively;
/// duplicated here UNGATED (no `cfg(not(target_arch = "wasm32"))`) because
/// Insert Date's live path must also run in the browser build, where
/// `streaks` compiles out entirely. Pure integer math, no clock read — unit-
/// tested below with fixed epoch-day inputs.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// LIVE "today" as a UTC civil `(year, month, day)`, read from the platform
/// wall clock via [`crate::clock::system_now`] — the ONE place both live call
/// sites (`App::insert_date`, the Settings overlay's live "today" preview
/// gather) read the clock, so they can never drift apart. UTC, not the local
/// timezone: awl carries no timezone database (matches
/// `crashlog::civil_date`/`streaks::civil_ymd_from_epoch_secs`'s own
/// UTC-civil-day convention) — a date typed near local midnight may read as
/// "yesterday" or "tomorrow" relative to the user's own wall clock, a known,
/// documented simplification, not a bug to chase here.
pub fn today_from_system_clock() -> (i32, u32, u32) {
    let secs = crate::clock::system_now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs.div_euclid(86_400))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DateFormat::format — every arm, fixed dates ──────────────────────

    #[test]
    fn dd_mm_yy_zero_pads_day_and_month() {
        assert_eq!(DateFormat::DdMmYy.format(2026, 7, 22), "22/07/26");
        // Single-digit day AND month both zero-pad.
        assert_eq!(DateFormat::DdMmYy.format(2009, 3, 7), "07/03/09");
        assert_eq!(DateFormat::DdMmYy.format(2000, 1, 1), "01/01/00");
    }

    #[test]
    fn mm_dd_yy_zero_pads_day_and_month() {
        assert_eq!(DateFormat::MmDdYy.format(2026, 7, 22), "07/22/26");
        assert_eq!(DateFormat::MmDdYy.format(2009, 3, 7), "03/07/09");
        assert_eq!(DateFormat::MmDdYy.format(2000, 1, 1), "01/01/00");
    }

    #[test]
    fn iso_is_yyyy_mm_dd_with_dashes() {
        assert_eq!(DateFormat::Iso.format(2026, 7, 22), "2026-07-22");
        assert_eq!(DateFormat::Iso.format(2009, 3, 7), "2009-03-07");
    }

    #[test]
    fn yyyy_mm_dd_is_iso_with_slashes() {
        assert_eq!(DateFormat::YyyyMmDd.format(2026, 7, 22), "2026/07/22");
        assert_eq!(DateFormat::YyyyMmDd.format(2009, 3, 7), "2009/03/07");
    }

    #[test]
    fn d_month_yyyy_day_is_never_zero_padded() {
        assert_eq!(DateFormat::DMonthYyyy.format(2026, 7, 22), "22 July 2026");
        // Single-digit day: NOT zero-padded ("7", not "07") — the format's
        // single "D" (vs "DD/MM/YY"'s doubled letters) says so.
        assert_eq!(DateFormat::DMonthYyyy.format(2009, 3, 7), "7 March 2009");
        assert_eq!(DateFormat::DMonthYyyy.format(2000, 1, 1), "1 January 2000");
    }

    #[test]
    fn d_month_yyyy_names_every_month() {
        let expected = [
            "January", "February", "March", "April", "May", "June", "July", "August",
            "September", "October", "November", "December",
        ];
        for (i, name) in expected.iter().enumerate() {
            let month = (i + 1) as u32;
            assert_eq!(
                DateFormat::DMonthYyyy.format(2020, month, 15),
                format!("15 {name} 2020"),
                "month {month} must render as {name:?}"
            );
        }
    }

    // ── cycle order / round-trip ──────────────────────────────────────────

    #[test]
    fn default_is_dd_mm_yy() {
        assert_eq!(DateFormat::default(), DateFormat::DdMmYy);
        assert_eq!(DateFormat::ALL[0], DateFormat::DdMmYy);
    }

    #[test]
    fn cycle_next_steps_through_all_five_and_wraps() {
        let start = DateFormat::DdMmYy;
        let mut cur = start;
        let mut seen = vec![cur];
        for _ in 0..4 {
            cur = cur.cycle_next();
            seen.push(cur);
        }
        assert_eq!(
            seen,
            vec![
                DateFormat::DdMmYy,
                DateFormat::MmDdYy,
                DateFormat::Iso,
                DateFormat::YyyyMmDd,
                DateFormat::DMonthYyyy,
            ]
        );
        // The sixth step wraps back to the start.
        assert_eq!(cur.cycle_next(), start);
    }

    #[test]
    fn config_name_round_trips_for_every_format() {
        for f in DateFormat::ALL {
            let slug = f.config_name();
            assert_eq!(DateFormat::from_config_name(slug), Some(f), "{slug:?} must round-trip");
        }
    }

    #[test]
    fn from_config_name_rejects_garbage() {
        assert_eq!(DateFormat::from_config_name("bogus"), None);
        assert_eq!(DateFormat::from_config_name(""), None);
        assert_eq!(DateFormat::from_config_name("DDMMYY"), None, "case-sensitive, like caret_mode/dictionary");
    }

    // ── active_format process-global ───────────────────────────────────────

    #[test]
    fn active_format_defaults_and_round_trips() {
        let _g = crate::testlock::serial();
        let saved = active_format();
        set_active_format(DateFormat::Iso);
        assert_eq!(active_format(), DateFormat::Iso);
        set_active_format(DateFormat::DMonthYyyy);
        assert_eq!(active_format(), DateFormat::DMonthYyyy);
        set_active_format(saved); // restore, so this leaks no state to another test
    }

    // ── civil_from_days — Howard Hinnant reference vectors ────────────────

    #[test]
    fn civil_from_days_known_dates() {
        // Epoch day 0 is 1970-01-01 (the Unix epoch itself).
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // A negative (pre-epoch) day steps back across the year boundary.
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2024-01-01T00:00:00Z is Unix time 1_704_067_200s = epoch day 19723
        // (a well-known reference value), exercising a leap-year-adjacent
        // century boundary in the era/yoe arithmetic.
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // 2000-01-01 is epoch day 10957 (Unix time 946_684_800s); +60 days
        // (31 in January + 29 in February, since 2000 IS a leap year — a
        // /400 exception, not just /4) lands on 2000-03-01, exercising the
        // Feb-29-in-a-leap-year crossing.
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(10_957 + 60), (2000, 3, 1));
    }
}
