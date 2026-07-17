//! WRITING STREAKS' App-side wiring (native only — `cfg(not(target_arch =
//! "wasm32"))`, mirroring the odometer / daemon / session-restore gate): the
//! per-buffer word-delta SAMPLING on the autosave flush triggers, the LOCAL-
//! calendar-day read, and the live year-view PUSH into the pipeline.
//! [`crate::streaks`] owns the pure store + calendar/intensity arithmetic + the
//! (de)serializer; this file is the seam that folds it into the live `App`.
//!
//! **The hooks (each config-gated on `stats_on()` — the one LOCAL-usage-tracking
//! kill switch the odometer already carries; both are native-only, private,
//! never-uploaded personal state, so they share the single privacy toggle):**
//!  - [`Self::streaks_flush`] — on the SAME idle/blur/switch/quit triggers the
//!    autosave engine flushes on: sample the active buffer's word count, record
//!    the DELTA since the last sample under today's LOCAL calendar day (clamped
//!    for the day total, raw kept), re-anchor, and persist if anything changed.
//!  - [`Self::streaks_reset_baseline`] — on a buffer SWAP (file open / new note):
//!    drop the anchor so the arriving buffer's existing words are re-anchored,
//!    never counted as freshly written.
//!  - [`Self::streaks_sync_card`] — every `sync_view`: push the live year-view so
//!    a summoned card this frame reads the real heatmap (live-only; a capture
//!    never calls `sync_view`, so the card shows the synthetic placeholder).
//!
//! **Determinism:** all three live ONLY on the live `App`; the headless capture
//! never constructs one, so a `--screenshot`/`--keys` capture is STRUCTURALLY
//! incapable of touching `streaks.toml` — the same boundary the odometer's
//! `headless_replay_never_touches_the_stats_file` tripwire pins.
//!
//! **LOCAL day (flagged):** std exposes no local-timezone offset, so the day
//! boundary is read from the OS via libc's `tm_gmtoff` (`localtime_r`) added to
//! the wall clock, then floored to a civil date by the pure
//! [`crate::streaks::civil_date_from_epoch_secs`]. This is the ONE timezone read;
//! the pure model stays clock-free and unit-testable.

use super::*;

impl App {
    /// Today's LOCAL calendar day as `"YYYY-MM-DD"`. Reads the wall clock
    /// (`system_now`, wasm-safe but this whole file is native-only) plus the OS's
    /// current UTC offset, then floors to a civil date via the pure model. A clock
    /// before the epoch or a null `localtime_r` degrades to a 0 offset (UTC), never
    /// a panic.
    #[cfg(not(target_arch = "wasm32"))]
    fn streaks_local_today(&self) -> String {
        let secs = crate::clock::system_now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        crate::streaks::civil_date_from_epoch_secs(secs + local_utc_offset_secs())
    }

    /// The active buffer's whole-document word count (the same `markdown::word_count`
    /// the readout / held HUD use). A `String` alloc per call, but flushes are
    /// infrequent (idle/blur/switch/quit), so this is cheap.
    #[cfg(not(target_arch = "wasm32"))]
    fn streaks_current_words(&self) -> usize {
        crate::markdown::word_count(&self.buffer.text())
    }

    /// Drop the word-delta ANCHOR across a BUFFER SWAP (file open / new note), so
    /// the arriving document's existing words re-anchor on the next flush rather
    /// than counting as freshly written. Mirrors [`Self::stats_reset_caret_anchor`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn streaks_reset_baseline(&mut self) {
        self.streaks_baseline = None;
    }

    /// Sample the active buffer's word count and fold the DELTA since the last
    /// sample into today's record, then persist if anything changed — on the SAME
    /// idle/blur/switch/quit triggers the autosave flush uses. A no-op when the
    /// feature is off. The FIRST sample of a buffer only ANCHORS (records nothing),
    /// so a file's pre-existing words are never counted; every later sample records
    /// the net words added since the previous one (clamped for the day total, raw
    /// kept). Errors go to stderr, never disrupt.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn streaks_flush(&mut self) {
        if !self.config.stats_on() {
            return;
        }
        let words = self.streaks_current_words();
        match self.streaks_baseline {
            None => {
                // Anchor only — a fresh launch or a just-swapped buffer. Nothing
                // recorded (opening content is not "writing").
                self.streaks_baseline = Some(words);
            }
            Some(prev) => {
                let delta = words as i64 - prev as i64;
                self.streaks_baseline = Some(words);
                if delta != 0 {
                    let day = self.streaks_local_today();
                    self.streaks.record_delta(&day, delta);
                    self.streaks_dirty = true;
                }
            }
        }
        if self.streaks_dirty {
            if let Err(e) = crate::streaks::save(&crate::streaks::streaks_path(), &self.streaks) {
                eprintln!("streaks save failed: {e}");
            }
            self.streaks_dirty = false;
        }
    }

    /// Push the live year-VIEW into the pipeline so a summoned Writing streaks card
    /// this frame reads the real heatmap. Called every `sync_view` (LIVE-ONLY); a
    /// headless capture never calls this, so the pipeline field stays `None` and the
    /// card renders the synthetic [`crate::streaks::placeholder`] — the determinism
    /// boundary keeping a `--streaks` capture byte-stable. When the feature is OFF
    /// we push `None` too, so the card honestly shows the placeholder rather than a
    /// misleading empty grid. Cheap: the view is a small pure computation over the
    /// (catalog-sized) day map, like `stats_sync_hud`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn streaks_sync_card(&mut self) {
        // Compute the view BEFORE borrowing the GPU (both read `self`).
        let view = if self.config.stats_on() {
            Some(self.streaks.view(&self.streaks_local_today()))
        } else {
            None
        };
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.pipeline.set_streaks(view);
        }
    }
}

/// The OS's CURRENT UTC offset in seconds (east positive) — the one timezone read
/// the streaks day boundary needs. std has no local-offset API, so this reads
/// libc's `tm_gmtoff` via `localtime_r` on the current time. A null return (never
/// expected) degrades to UTC (0). Unsafe FFI is contained here; the result feeds
/// the pure civil-date conversion.
#[cfg(not(target_arch = "wasm32"))]
fn local_utc_offset_secs() -> i64 {
    // SAFETY: `time` takes a null pointer (returns the current time) and
    // `localtime_r` writes into our stack `tm`, which we zero first. Both are the
    // documented calling conventions; `tm_gmtoff` is a stable field on macOS +
    // Linux libc.
    unsafe {
        let t: libc::time_t = libc::time(std::ptr::null_mut());
        let mut tmv: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tmv).is_null() {
            return 0;
        }
        tmv.tm_gmtoff as i64
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn writing_words_records_the_net_delta_after_anchoring() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            // First flush ANCHORS the empty scratch buffer — records nothing.
            app.streaks_flush();
            assert!(app.streaks.days.is_empty(), "the anchor flush records nothing");
            let today = app.streaks_local_today();

            // Write some words, then flush: the net delta is recorded under today.
            app.buffer.set_text("hello there friend");
            app.streaks_flush();
            assert_eq!(app.streaks.words_on(&today), 3, "three net words added");

            // Cut back to two words, flush: a net-cut flush never erodes the day
            // total (raw net still drops).
            app.buffer.set_text("hello there");
            app.streaks_flush();
            assert_eq!(app.streaks.words_on(&today), 3, "a cut never lowers the day total");
            assert!(app.streaks.days.get(&today).unwrap().raw_net <= 3);

            // Persisted to (and reloaded from) streaks.toml.
            let saved = crate::streaks::load(&crate::streaks::streaks_path());
            assert_eq!(saved.words_on(&today), 3);
        });
    }

    #[test]
    fn a_buffer_swap_reset_anchors_the_new_buffer() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let mut app = App::new(None, PathBuf::from("/n"), None, None, Config::empty());
            app.buffer.set_text("one two three four");
            app.streaks_flush(); // anchors (None → Some(4)), records nothing
            let today = app.streaks_local_today();
            let before = app.streaks.words_on(&today);
            // Simulate a swap: reset the anchor, replace the buffer with a big doc.
            app.streaks_reset_baseline();
            app.buffer = crate::buffer::Buffer::from_str("a b c d e f g h i j");
            app.streaks_flush(); // must ANCHOR the arriving words, not count them
            assert_eq!(
                app.streaks.words_on(&today),
                before,
                "opening a doc's existing words is anchored, never counted as written"
            );
        });
    }

    #[test]
    fn kill_switch_off_records_nothing_and_never_writes() {
        crate::fs::with_fs(Arc::new(crate::fs::InMemoryFs::new()), || {
            let cfg = Config { stats: Some(false), ..Config::empty() };
            let mut app = App::new(None, PathBuf::from("/n"), None, None, cfg);
            app.buffer.set_text("some words here now");
            app.streaks_flush();
            assert!(app.streaks.days.is_empty(), "off: no recording");
            assert!(
                crate::fs::active().read(&crate::streaks::streaks_path()).is_err(),
                "off: never writes streaks.toml"
            );
        });
    }
}
