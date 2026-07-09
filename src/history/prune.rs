//! src/history/prune.rs — the AGED RETENTION LADDER: [`prune_ladder`] (the
//! pure policy [`super::store::record_at`] calls after every append) and its
//! rung-by-rung [`ladder_keep`] worker. Split out of the former
//! `history.rs` monolith (2026-07 code-organization pass) — see `store`
//! for the [`Entry`](super::store::Entry) type this prunes and the log
//! file it's pruned inside of.

use super::store::Entry;

// --- The AGED RETENTION LADDER's rungs (all millis) ------------------------

/// Keep EVERYTHING at most this old (15 min): the undo-adjacent recent past
/// stays at full resolution.
const FRESH_MS: u64 = 15 * 60_000;

/// Two snapshots closer than this belong to the same WORK SESSION (15 min of
/// quiet ends a session); between the fresh window and a day old, only each
/// session's LAST snapshot survives.
const SESSION_GAP_MS: u64 = 15 * 60_000;

/// One day, the session-band horizon: older than this, resolution drops to
/// one snapshot per day.
pub(super) const DAY_MS: u64 = 86_400_000;

/// The daily band's horizon (~30 days): older than this, resolution drops to
/// one snapshot per week.
const OLD_HORIZON_MS: u64 = 30 * DAY_MS;

/// One week, the oldest band's bucket width.
const WEEK_MS: u64 = 7 * DAY_MS;

/// The BACKSTOP total cap per file. Enforced by climbing the ladder HARDER
/// (each level doubles the session gap + bucket widths and halves the fresh
/// window) — never by FIFO-dropping the oldest memory.
pub(super) const MAX_TOTAL: usize = 150;

/// The AGED RETENTION LADDER: prune `entries` (newest-first) so RESOLUTION
/// thins with age while MEMORY is kept. A PURE function of `(entries, now_ms)`
/// — the clock is injected, so the whole policy is deterministic and
/// unit-testable without the store. Level 0 keeps: everything fresher than
/// [`FRESH_MS`]; one snapshot per WORK SESSION (gaps < [`SESSION_GAP_MS`])
/// up to a day; one per DAY up to [`OLD_HORIZON_MS`]; one per WEEK beyond.
/// If the keep-set still exceeds [`MAX_TOTAL`], the ladder is climbed HARDER
/// (each level halves the fresh window and doubles the gap + bucket widths)
/// until it fits — NEVER FIFO: the file's oldest snapshot always survives.
/// The just-recorded snapshot (stamped `now`) is always fresh, so it survives.
///
/// THE CONSCIOUS MARK (built): a `pinned` entry is prune-EXEMPT — it is UNIONED
/// into the keep-set at every level (it always survives, whatever the aged bands
/// or the cap say), AND it does NOT count against [`MAX_TOTAL`]: the cap governs
/// only the un-pinned prunable set, so kept versions never force the ladder to
/// thin more or get FIFO'd away. A file with more pins than the cap keeps all of
/// them (deliberately — a pin means "keep this, always").
pub(crate) fn prune_ladder(entries: &mut Vec<Entry>, now_ms: u64) {
    let ts: Vec<u64> = entries.iter().map(|e| e.ts).collect();
    let pinned: Vec<bool> = entries.iter().map(|e| e.pinned).collect();
    let mut chosen = ladder_keep(&ts, now_ms, 0);
    for level in 1..=32u32 {
        // Count only the UN-PINNED survivors against the cap — pins are exempt and
        // ride along free, so climbing the ladder only thins the prunable set.
        let unpinned_kept = chosen
            .iter()
            .zip(&pinned)
            .filter(|(keep, p)| **keep && !**p)
            .count();
        if unpinned_kept <= MAX_TOTAL {
            break;
        }
        chosen = ladder_keep(&ts, now_ms, level);
    }
    // Retain a row iff it is PINNED (the conscious mark, unconditional) OR the
    // ladder chose it. `i` walks the parallel masks in the retained order.
    let mut i = 0;
    entries.retain(|_| {
        let keep = pinned.get(i).copied().unwrap_or(false)
            || chosen.get(i).copied().unwrap_or(true);
        i += 1;
        keep
    });
}

/// One LEVEL of the retention ladder: which of the newest-first timestamps
/// `ts` survive, as a parallel keep-mask. `level` scales the rungs — the fresh
/// window HALVES (`FRESH_MS >> level`) and the session gap / day / week bucket
/// widths DOUBLE (`<< level`) each step, so a higher level keeps strictly less
/// and the cap loop in [`prune_ladder`] terminates. Band boundaries (a day, 30
/// days) stay FIXED; only the resolution inside each band coarsens. Survivor
/// of a session/day/week = its LAST (newest) snapshot; the OLDEST timestamp is
/// always kept outright (memory, not resolution). Pure.
fn ladder_keep(ts: &[u64], now_ms: u64, level: u32) -> Vec<bool> {
    let fresh = FRESH_MS >> level.min(63);
    let gap = SESSION_GAP_MS << level.min(20);
    let day_w = DAY_MS << level.min(20);
    let week_w = WEEK_MS << level.min(20);
    let n = ts.len();
    let mut keep = vec![false; n];
    // Walk OLD → NEW (reverse of the stored order) so session clustering reads
    // gaps forward in time. Track the previous member's band + key to decide
    // survivors: in the session band a new cluster starts when the forward gap
    // reaches `gap`; in the bucketed bands a new bucket starts when `ts / width`
    // changes. The NEWEST member of each cluster/bucket survives — i.e. the last
    // index visited before the cluster/bucket changes (indices shrink as time
    // advances, so "newest of the group" = the final i in that group).
    #[derive(PartialEq)]
    enum Band {
        Fresh,
        Session,
        Daily(u64),
        Weekly(u64),
    }
    let band_of = |t: u64| -> Band {
        let age = now_ms.saturating_sub(t);
        if age <= fresh {
            Band::Fresh
        } else if age <= DAY_MS {
            Band::Session
        } else if age <= OLD_HORIZON_MS {
            Band::Daily(t / day_w)
        } else {
            Band::Weekly(t / week_w)
        }
    };
    let mut prev: Option<(usize, Band, u64)> = None; // (index, band, ts)
    for i in (0..n).rev() {
        let t = ts[i];
        let band = band_of(t);
        if let Some((pi, pband, pt)) = &prev {
            let same_group = match (&band, pband) {
                (Band::Fresh, Band::Fresh) => true, // fresh keeps all anyway
                (Band::Session, Band::Session) => t.saturating_sub(*pt) < gap,
                (Band::Daily(b), Band::Daily(pb)) => b == pb,
                (Band::Weekly(b), Band::Weekly(pb)) => b == pb,
                _ => false,
            };
            if !same_group {
                // The previous group closed: its newest member survives.
                keep[*pi] = true;
            }
        }
        if band == Band::Fresh {
            keep[i] = true; // the fresh band keeps everything
        }
        prev = Some((i, band, t));
    }
    if let Some((pi, _, _)) = prev {
        keep[pi] = true; // the final (newest) group's survivor
    }
    // MEMORY over resolution: the file's ORIGIN — its oldest snapshot — is never
    // pruned away, whatever bucket alignment says.
    if let Some(last) = keep.last_mut() {
        *last = true;
    }
    keep
}
