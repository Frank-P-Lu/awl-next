//! The LIVE PROBE HARNESS's App-side wiring (native only): react to a posted
//! [`crate::probe::ProbeEvent`] on the winit thread. A scripted chord rides
//! the SAME dispatch tail a physical key press takes
//! (`App::dispatch_pressed_key` — one owner, two callers, so the probe can
//! never drift from the real input path); a shot asks the WINDOW SERVER for
//! its current composited image of our own window (the compositor's side of
//! the present, which is where the live-only bug classes show); quit routes
//! through the same `Action::Quit` a Cmd-Q takes (real teardown). See
//! `crate::probe`'s module doc for the harness contract + capture gate.

use super::*;

impl App {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn handle_probe_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: crate::probe::ProbeEvent,
    ) {
        match event {
            crate::probe::ProbeEvent::Chord(chord) => {
                // A parsed chord carries its own modifier state (a physical
                // press would have delivered it as `ModifiersChanged` first);
                // it is already un-composed, so `raw` and `bare` coincide.
                self.mods = chord.mods;
                self.dispatch_pressed_key(event_loop, chord.key.clone(), chord.key, false);
                self.mods = winit::event::Modifiers::default();
            }
            crate::probe::ProbeEvent::Shot(path) => self.probe_shot(&path),
            crate::probe::ProbeEvent::Quit => {
                let exited = self.apply(
                    crate::keymap::Action::Quit,
                    false,
                    event_loop,
                    crate::stats::Door::Chord,
                );
                // A modal surface (an open picker consumes ordinary actions)
                // may swallow the Quit; a probe run must still terminate, and
                // `event_loop.exit()` runs the same `exiting()` teardown.
                if !exited {
                    event_loop.exit();
                }
            }
        }
    }

    /// Take one screenshot of the live window. Backend chain, most-real first:
    ///
    /// 1. **window-server** (macOS): `CGWindowListCreateImage` of our own
    ///    window — the compositor's current pixels, which sees even a pure
    ///    present-race. VALIDATED against the real surface size: without the
    ///    Screen Recording TCC grant macOS silently returns a tiny generic
    ///    placeholder instead of failing, so an undersized image falls through.
    /// 2. **frame-mirror**: the probe's persistent copy of the LAST PRESENTED
    ///    frame (`Gpu::read_probe_mirror`) — no TCC needed, and it still
    ///    catches the stale-cache + missed-redraw classes because it is never
    ///    refreshed by the shot itself (a scheduling gap leaves the OLD frame
    ///    in the mirror, exactly what the assertion should then fail on).
    ///
    /// Every outcome prints ONE `LIVE-PROBE shot …` protocol line to stdout —
    /// the wrapping script (`scripts/live-probe.sh`) treats a missing/failed
    /// line as a run failure, so a silently-broken capture can never
    /// masquerade as a pass. (Print fate (c): CLI harness output, audited in
    /// `println_audit`.)
    #[cfg(not(target_arch = "wasm32"))]
    fn probe_shot(&self, path: &std::path::Path) {
        let (sw, sh) = self
            .gpu
            .as_ref()
            .map(|g| (g.config.width, g.config.height))
            .unwrap_or((0, 0));
        #[cfg(target_os = "macos")]
        {
            let via_ws = crate::mac_chrome::own_window_number()
                .ok_or_else(|| "no window number".to_string())
                .and_then(crate::probe::capture_window_image)
                .and_then(|img| {
                    // The window-server image spans the whole frame (>= the
                    // surface in both axes at the same scale); the TCC-denied
                    // placeholder is tiny. Reject anything smaller than the
                    // surface we know we configured.
                    if img.width() >= sw && img.height() >= sh && sw > 0 {
                        Ok(img)
                    } else {
                        Err(format!(
                            "window-server image {}x{} < surface {sw}x{sh} (TCC placeholder?)",
                            img.width(),
                            img.height()
                        ))
                    }
                });
            match via_ws {
                Ok(img) => {
                    match img.save(path) {
                        Ok(()) => println!("LIVE-PROBE shot {} ok backend=window-server", path.display()),
                        Err(e) => println!("LIVE-PROBE shot {} FAILED: png write: {e}", path.display()),
                    }
                    return;
                }
                Err(reason) => {
                    // Fall through to the frame mirror, naming why on the line.
                    self.probe_shot_mirror(path, &reason);
                    return;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        self.probe_shot_mirror(path, "no window-server backend on this platform");
    }

    /// The frame-mirror fallback half of [`Self::probe_shot`] (one printer for
    /// both platforms' fall-through arms).
    #[cfg(not(target_arch = "wasm32"))]
    fn probe_shot_mirror(&self, path: &std::path::Path, why_not_ws: &str) {
        let read = self
            .gpu
            .as_ref()
            .ok_or_else(|| "no gpu".to_string())
            .and_then(Gpu::read_probe_mirror);
        match read {
            Ok(img) => match img.save(path) {
                Ok(()) => println!(
                    "LIVE-PROBE shot {} ok backend=frame-mirror ({why_not_ws})",
                    path.display()
                ),
                Err(e) => println!("LIVE-PROBE shot {} FAILED: png write: {e}", path.display()),
            },
            Err(e) => println!("LIVE-PROBE shot {} FAILED: {e} ({why_not_ws})", path.display()),
        }
    }
}
