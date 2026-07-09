//! JSON-string escaping + the held-key streak/`step_held` clamp math --
//! split out of the former monolithic `capture::tests` (2026-07
//! code-organization pass).

use super::super::*;
use super::super::animated::step_held;
use super::super::sidecar::json_string;
use super::{held_run_keeps_steady_streak};

#[test]
fn json_string_escapes_quote_backslash_newline_and_control() {
    // Every sidecar string field flows through json_string; this is its only
    // direct test (the schema test that exercises it is GPU-gated, so on a
    // headless box the JSON contract is otherwise untested).
    assert_eq!(json_string("a\"b\\c\n\t"), "\"a\\\"b\\\\c\\n\\t\"");
    // A control char below 0x20 becomes a \uXXXX escape (0x01 -> ).
    assert_eq!(json_string("\u{01}"), "\"\\u0001\"");
    // Carriage return + tab are their short escapes.
    assert_eq!(json_string("\r\t"), "\"\\r\\t\"");
    // Round-trip a tricky string back through a real JSON parser: the escaped
    // literal must parse to exactly the original bytes.
    let tricky = "path \"with\" \\slashes\\ and\n\tcontrol\u{01}\u{1f}";
    let parsed: String = serde_json::from_str(&json_string(tricky))
        .expect("json_string output must be valid JSON");
    assert_eq!(parsed, tricky);
}

#[test]
fn step_held_advances_and_clamps() {
    // line lengths: line 0 = 5 chars, line 1 = 2 chars, line 2 = 8 chars.
    let lens = [5usize, 2, 8];
    let last = 2;
    // RIGHT advances one char, then clamps at the line end.
    assert_eq!(step_held((0, 3), HeldDir::Right, &lens, last), (0, 4));
    assert_eq!(step_held((0, 5), HeldDir::Right, &lens, last), (0, 5));
    // LEFT decrements, saturating at column 0.
    assert_eq!(step_held((0, 1), HeldDir::Left, &lens, last), (0, 0));
    assert_eq!(step_held((0, 0), HeldDir::Left, &lens, last), (0, 0));
    // DOWN advances a line and pins the column to the shorter dest line.
    assert_eq!(step_held((0, 4), HeldDir::Down, &lens, last), (1, 2));
    assert_eq!(step_held((2, 8), HeldDir::Down, &lens, last), (2, 8)); // clamp at last line
    // UP retreats a line and clamps the column to that line's length.
    assert_eq!(step_held((2, 7), HeldDir::Up, &lens, last), (1, 2));
    assert_eq!(step_held((0, 3), HeldDir::Up, &lens, last), (0, 3)); // saturate at line 0
}

#[test]
fn held_right_run_streak_steady_over_gap() {
    // A long line so RIGHT never clamps mid-run.
    held_run_keeps_steady_streak(HeldDir::Right, &[40, 40, 40, 40, 40, 40, 40], (3, 5));
}

#[test]
fn held_down_run_streak_steady_over_gap() {
    // Enough lines (all wide) so DOWN advances a real line each step.
    held_run_keeps_steady_streak(HeldDir::Down, &[20; 12], (0, 5));
}
