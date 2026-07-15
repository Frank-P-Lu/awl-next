//! `bench.json` writer + baseline reader + regression diff for `--bench-suite`.
//!
//! The writer is HAND-ROLLED like the capture sidecar (`capture/sidecar.rs` —
//! no serde, byte-stable output, the crate's one JSON idiom); the reader
//! parses ONLY this writer's own versioned shape (each cell on its own line,
//! known key order), which the round-trip test below keeps in lockstep. The
//! shape is namespaced `awl-bench/N` — deliberately NOT the capture sidecar's
//! `awl-capture/N` family, so this round changes no sidecar shape.
//!
//! The BASELINE is machine-keyed (hostname + arch, per the spec): a diff on a
//! foreign machine prints a calm note and exits clean — never a false alarm —
//! while on the baseline's own machine a >20% median regression (over a small
//! absolute floor), a vanished cell, or a WITNESS drift (the workload itself
//! changed) fails loudly so the merge-day ritual notices.

use std::path::Path;

use anyhow::{bail, Context as _, Result};

/// The bench.json shape version. Bump on any field change; a baseline written
/// under another version refuses to diff (regenerate deliberately).
pub(super) const SCHEMA: &str = "awl-bench/1";

/// One measured matrix cell, as written to (and read back from) bench.json.
/// `min_ms` is the DIFF statistic (the least-contended sample — robust when
/// concurrent builds share the machine, and a real O(doc) regression raises
/// the min by the same multiple); `median_ms`/`p90_ms` are the report pair.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CellRec {
    pub tier: String,
    pub scenario: String,
    pub samples: u64,
    pub min_ms: f64,
    pub median_ms: f64,
    pub p90_ms: f64,
    pub witness: Vec<(String, u64)>,
}

/// One documented matrix hole.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SkipRec {
    pub tier: String,
    pub scenario: String,
    pub reason: String,
}

/// The whole bench.json document.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BenchDoc {
    pub schema: String,
    pub host: String,
    pub arch: String,
    pub os: String,
    pub cpu: String,
    pub rustc: String,
    pub awl: String,
    pub profile: String,
    pub wall_s: f64,
    pub cells: Vec<CellRec>,
    pub skips: Vec<SkipRec>,
}

impl BenchDoc {
    /// Gather the machine + toolchain identity for a fresh run.
    pub(super) fn gather(wall_s: f64, cells: Vec<CellRec>, skips: Vec<SkipRec>) -> Self {
        BenchDoc {
            schema: SCHEMA.to_string(),
            host: hostname(),
            arch: std::env::consts::ARCH.to_string(),
            os: std::env::consts::OS.to_string(),
            cpu: cpu_brand(),
            rustc: rustc_version(),
            awl: env!("CARGO_PKG_VERSION").to_string(),
            profile: if cfg!(debug_assertions) { "debug" } else { "release" }.to_string(),
            wall_s,
            cells,
            skips,
        }
    }

    /// Serialize — one cell per line (the parser's contract).
    pub(super) fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str(&format!("  \"schema\": {},\n", q(&self.schema)));
        s.push_str(&format!(
            "  \"machine\": {{\"host\": {}, \"arch\": {}, \"os\": {}, \"cpu\": {}}},\n",
            q(&self.host),
            q(&self.arch),
            q(&self.os),
            q(&self.cpu)
        ));
        s.push_str(&format!(
            "  \"toolchain\": {{\"rustc\": {}, \"awl\": {}, \"profile\": {}}},\n",
            q(&self.rustc),
            q(&self.awl),
            q(&self.profile)
        ));
        s.push_str(&format!(
            "  \"canvas\": {{\"width\": {}, \"height\": {}, \"dpi\": {}}},\n",
            super::WIDTH,
            super::HEIGHT,
            super::DPI
        ));
        s.push_str(&format!("  \"wall_s\": {:.1},\n", self.wall_s));
        s.push_str("  \"cells\": [\n");
        for (i, c) in self.cells.iter().enumerate() {
            let witness = c
                .witness
                .iter()
                .map(|(k, v)| format!("{}: {}", q(k), v))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!(
                "    {{\"tier\": {}, \"scenario\": {}, \"samples\": {}, \"min_ms\": {:.3}, \"median_ms\": {:.3}, \"p90_ms\": {:.3}, \"witness\": {{{}}}}}{}\n",
                q(&c.tier),
                q(&c.scenario),
                c.samples,
                c.min_ms,
                c.median_ms,
                c.p90_ms,
                witness,
                if i + 1 == self.cells.len() { "" } else { "," }
            ));
        }
        s.push_str("  ],\n");
        s.push_str("  \"skips\": [\n");
        for (i, k) in self.skips.iter().enumerate() {
            s.push_str(&format!(
                "    {{\"tier\": {}, \"scenario\": {}, \"reason\": {}}}{}\n",
                q(&k.tier),
                q(&k.scenario),
                q(&k.reason),
                if i + 1 == self.skips.len() { "" } else { "," }
            ));
        }
        s.push_str("  ]\n");
        s.push_str("}\n");
        s
    }
}

/// JSON string escape (mirrors `capture/sidecar.rs`'s idiom).
fn q(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Extract the string value of `"key": "..."` from `hay` (first occurrence).
fn str_field(hay: &str, key: &str) -> Result<String> {
    let pat = format!("\"{key}\":");
    let at = hay.find(&pat).with_context(|| format!("missing field {key:?}"))?;
    let rest = &hay[at + pat.len()..];
    let open = rest.find('"').with_context(|| format!("field {key:?} is not a string"))?;
    let mut out = String::new();
    let mut chars = rest[open + 1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Ok(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => bail!("dangling escape in field {key:?}"),
            },
            c => out.push(c),
        }
    }
    bail!("unterminated string in field {key:?}")
}

/// Extract the numeric value of `"key": 12.3` from `hay` (first occurrence).
fn num_field(hay: &str, key: &str) -> Result<f64> {
    let pat = format!("\"{key}\":");
    let at = hay.find(&pat).with_context(|| format!("missing field {key:?}"))?;
    let rest = hay[at + pat.len()..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E'))
        .unwrap_or(rest.len());
    rest[..end]
        .parse::<f64>()
        .with_context(|| format!("bad number in field {key:?}: {:?}", &rest[..end]))
}

/// Parse a bench.json produced by [`BenchDoc::to_json`]. Only this writer's
/// own shape — the round-trip test keeps the two in lockstep.
pub(super) fn parse(text: &str) -> Result<BenchDoc> {
    let schema = str_field(text, "schema")?;
    let host = str_field(text, "host")?;
    let arch = str_field(text, "arch")?;
    let os = str_field(text, "os")?;
    let cpu = str_field(text, "cpu")?;
    let rustc = str_field(text, "rustc")?;
    let awl = str_field(text, "awl")?;
    let profile = str_field(text, "profile")?;
    let wall_s = num_field(text, "wall_s")?;
    let mut cells = Vec::new();
    let mut skips = Vec::new();
    let mut in_skips = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("\"skips\"") {
            in_skips = true;
            continue;
        }
        if !(t.starts_with('{') && t.contains("\"tier\":")) {
            continue;
        }
        if in_skips {
            skips.push(SkipRec {
                tier: str_field(t, "tier")?,
                scenario: str_field(t, "scenario")?,
                reason: str_field(t, "reason")?,
            });
        } else {
            let wopen = t.find("\"witness\":").context("cell line missing witness")?;
            let wbody_start = t[wopen..].find('{').context("witness must be an object")? + wopen + 1;
            let wbody_end = t[wbody_start..].find('}').context("unterminated witness")? + wbody_start;
            let mut witness = Vec::new();
            let body = t[wbody_start..wbody_end].trim();
            if !body.is_empty() {
                for pair in body.split(',') {
                    let (k, v) = pair.split_once(':').context("bad witness pair")?;
                    witness.push((
                        k.trim().trim_matches('"').to_string(),
                        v.trim().parse::<u64>().context("bad witness count")?,
                    ));
                }
            }
            cells.push(CellRec {
                tier: str_field(t, "tier")?,
                scenario: str_field(t, "scenario")?,
                samples: num_field(t, "samples")? as u64,
                min_ms: num_field(t, "min_ms")?,
                median_ms: num_field(t, "median_ms")?,
                p90_ms: num_field(t, "p90_ms")?,
                witness,
            });
        }
    }
    Ok(BenchDoc { schema, host, arch, os, cpu, rustc, awl, profile, wall_s, cells, skips })
}

/// What a baseline comparison concluded — pure data, printable + testable.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Diff {
    /// (tier, scenario, base median, current median) over the warn threshold.
    pub regressions: Vec<(String, String, f64, f64)>,
    /// Cells the baseline has that the current run lacks.
    pub missing: Vec<(String, String)>,
    /// Cells whose witness counters differ — the WORKLOAD changed.
    pub witness_drift: Vec<(String, String)>,
    /// Informational: cells >=20% faster than baseline.
    pub improved: Vec<(String, String, f64, f64)>,
    /// Informational: cells the current run has that the baseline lacks.
    pub new_cells: Vec<(String, String)>,
}

/// Warn at ~20% per cell on the MIN sample, over an absolute floor. The MIN
/// is the least-contended run — this machine routinely hosts concurrent
/// builds, and a median-based gate measurably cried wolf on 8 cells of
/// byte-identical code during calibration (+22–57% median, witnesses
/// identical) — while the regression CLASS this gate exists for (accidental
/// O(doc) work) raises the min by the same multiple as every other sample.
const WARN_RATIO: f64 = 1.20;
const WARN_FLOOR_MS: f64 = 0.5;

/// Compare two same-machine, same-shape docs. Pure — the printing + exit
/// policy live in [`diff_against`], so this is directly unit-testable.
pub(super) fn compare(base: &BenchDoc, cur: &BenchDoc) -> Diff {
    let mut d = Diff::default();
    for b in &base.cells {
        let Some(c) = cur
            .cells
            .iter()
            .find(|c| c.tier == b.tier && c.scenario == b.scenario)
        else {
            d.missing.push((b.tier.clone(), b.scenario.clone()));
            continue;
        };
        let mut bw = b.witness.clone();
        let mut cw = c.witness.clone();
        bw.sort();
        cw.sort();
        if bw != cw {
            d.witness_drift.push((b.tier.clone(), b.scenario.clone()));
            continue;
        }
        if c.min_ms > b.min_ms * WARN_RATIO && c.min_ms - b.min_ms > WARN_FLOOR_MS {
            d.regressions
                .push((b.tier.clone(), b.scenario.clone(), b.min_ms, c.min_ms));
        } else if c.min_ms < b.min_ms / WARN_RATIO && b.min_ms - c.min_ms > WARN_FLOOR_MS {
            d.improved
                .push((b.tier.clone(), b.scenario.clone(), b.min_ms, c.min_ms));
        }
    }
    for c in &cur.cells {
        if !base
            .cells
            .iter()
            .any(|b| b.tier == c.tier && b.scenario == c.scenario)
        {
            d.new_cells.push((c.tier.clone(), c.scenario.clone()));
        }
    }
    d
}

/// Diff the current run against the checked-in baseline file. Exit policy:
///   * no baseline file            -> note, Ok (first run / fresh checkout).
///   * schema mismatch             -> Err (regenerate deliberately).
///   * foreign machine             -> note, Ok (never a false alarm).
///   * profile mismatch            -> note, Ok (dev runs never gate).
///   * regressions / missing cells / witness drift -> Err (nonzero exit).
pub(super) fn diff_against(path: &Path, cur: &BenchDoc) -> Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            println!(
                "no baseline at {} — skipping diff (create one with scripts/bench.sh --update-baseline)",
                path.display()
            );
            return Ok(());
        }
    };
    let base = parse(&text).with_context(|| format!("unreadable baseline {}", path.display()))?;
    if base.schema != cur.schema {
        bail!(
            "baseline shape {} != current {} — regenerate it deliberately: scripts/bench.sh --update-baseline",
            base.schema,
            cur.schema
        );
    }
    if base.host != cur.host || base.arch != cur.arch {
        println!(
            "no baseline for this machine (baseline: {}/{}, this: {}/{}) — skipping diff",
            base.host, base.arch, cur.host, cur.arch
        );
        return Ok(());
    }
    if base.profile != cur.profile {
        println!(
            "baseline profile {:?} != current {:?} — skipping diff (bench in --release)",
            base.profile, cur.profile
        );
        return Ok(());
    }
    let d = compare(&base, cur);
    for (t, s, b, c) in &d.improved {
        println!("  improved  {t:>6} {s:<10} min {b:>9.3}ms -> {c:>9.3}ms ({:+.0}%)", (c / b - 1.0) * 100.0);
    }
    for (t, s) in &d.new_cells {
        println!("  new cell  {t:>6} {s:<10} (no baseline entry — next --update-baseline records it)");
    }
    for (t, s, b, c) in &d.regressions {
        println!("  REGRESSED {t:>6} {s:<10} min {b:>9.3}ms -> {c:>9.3}ms ({:+.0}%)", (c / b - 1.0) * 100.0);
    }
    for (t, s) in &d.missing {
        println!("  MISSING   {t:>6} {s:<10} (baseline has this cell; the current run does not)");
    }
    for (t, s) in &d.witness_drift {
        println!("  WITNESS   {t:>6} {s:<10} (witness counters changed — the workload itself moved)");
    }
    if !d.regressions.is_empty() || !d.missing.is_empty() || !d.witness_drift.is_empty() {
        bail!(
            "bench regression check failed: {} regressed, {} missing, {} witness-drifted \
             (intended? update deliberately: scripts/bench.sh --update-baseline)",
            d.regressions.len(),
            d.missing.len(),
            d.witness_drift.len()
        );
    }
    println!(
        "baseline diff clean — {} cells within {:.0}% of {}",
        base.cells.len(),
        (WARN_RATIO - 1.0) * 100.0,
        path.display()
    );
    Ok(())
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn cpu_brand() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(s) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return s;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(s) = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|v| v.trim().to_string())
            })
            .filter(|s| !s.is_empty())
        {
            return s;
        }
    }
    std::env::consts::ARCH.to_string()
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> BenchDoc {
        BenchDoc {
            schema: SCHEMA.to_string(),
            host: "mac \"quoted\" host".to_string(),
            arch: "aarch64".to_string(),
            os: "macos".to_string(),
            cpu: "Apple M-series".to_string(),
            rustc: "rustc 1.99.0".to_string(),
            awl: "0.1.0".to_string(),
            profile: "release".to_string(),
            wall_s: 93.5,
            cells: vec![
                CellRec {
                    tier: "S".to_string(),
                    scenario: "cold_open".to_string(),
                    samples: 5,
                    min_ms: 11.5,
                    median_ms: 12.345,
                    p90_ms: 13.5,
                    witness: vec![("reshapes".to_string(), 10), ("rows".to_string(), 58)],
                },
                CellRec {
                    tier: "L".to_string(),
                    scenario: "scroll".to_string(),
                    samples: 17,
                    min_ms: 3.5,
                    median_ms: 4.0,
                    p90_ms: 6.25,
                    witness: vec![("pages".to_string(), 16)],
                },
            ],
            skips: vec![SkipRec {
                tier: "CODE".to_string(),
                scenario: "zoom".to_string(),
                reason: "zoom burst is a prose-reading affordance".to_string(),
            }],
        }
    }

    /// Writer and parser stay in lockstep: what to_json emits, parse reads
    /// back FIELD-FOR-FIELD (incl. an escaped quote in the host string).
    #[test]
    fn bench_json_round_trips_byte_faithfully() {
        let doc = sample_doc();
        let parsed = parse(&doc.to_json()).expect("own output must parse");
        assert_eq!(parsed, doc);
    }

    /// The regression rule (on the MIN sample): >20% over the floor fires;
    /// 10% does not; a tiny absolute wiggle on a micro-cell never fires; a
    /// vanished cell and a witness drift both fire.
    #[test]
    fn compare_flags_regressions_missing_and_witness_drift_only() {
        let base = sample_doc();

        // 10% slower — clean.
        let mut cur = base.clone();
        cur.cells[0].min_ms = base.cells[0].min_ms * 1.10;
        let d = compare(&base, &cur);
        assert!(d.regressions.is_empty() && d.missing.is_empty() && d.witness_drift.is_empty());

        // 30% slower — regression.
        let mut cur = base.clone();
        cur.cells[0].min_ms = base.cells[0].min_ms * 1.30;
        let d = compare(&base, &cur);
        assert_eq!(d.regressions.len(), 1);
        assert_eq!(d.regressions[0].0, "S");

        // 30% slower but under the absolute floor — clean (micro-cell noise).
        let mut base_micro = base.clone();
        base_micro.cells[1].min_ms = 0.010;
        let mut cur = base_micro.clone();
        cur.cells[1].min_ms = 0.013;
        assert!(compare(&base_micro, &cur).regressions.is_empty());

        // A slower MEDIAN alone (machine load) — clean: the gate reads the min.
        let mut cur = base.clone();
        cur.cells[0].median_ms = base.cells[0].median_ms * 1.50;
        cur.cells[0].p90_ms = base.cells[0].p90_ms * 2.0;
        assert!(compare(&base, &cur).regressions.is_empty());

        // A vanished cell — missing.
        let mut cur = base.clone();
        cur.cells.pop();
        let d = compare(&base, &cur);
        assert_eq!(d.missing, vec![("L".to_string(), "scroll".to_string())]);

        // A witness drift — flagged, and NOT double-reported as a regression.
        let mut cur = base.clone();
        cur.cells[0].witness[0].1 += 1;
        cur.cells[0].min_ms *= 2.0;
        let d = compare(&base, &cur);
        assert_eq!(d.witness_drift, vec![("S".to_string(), "cold_open".to_string())]);
        assert!(d.regressions.is_empty());

        // A 25% improvement — informational.
        let mut cur = base.clone();
        cur.cells[0].min_ms = base.cells[0].min_ms * 0.75;
        let d = compare(&base, &cur);
        assert_eq!(d.improved.len(), 1);
        assert!(d.regressions.is_empty());
    }
}
