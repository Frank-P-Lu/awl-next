# Review: Codex GPU failure-path hardening (2026-07-16, claude adversarial review, 5 dimensions + verify)

## Headline: the zero-drawable "defect" is NOT a code bug — code exonerated, mystery solved

**Root cause (CONFIRMED by bidirectional experiment):** wgpu-hal 29.0.3's macOS occlusion
gate (`metal/surface.rs:116-152`, the wgpu#8309 workaround) returns `SurfaceError::Occluded`
BEFORE calling `nextDrawable()` whenever the NSWindow lacks `NSWindowOcclusionStateVisible` —
i.e. whenever the window is never actually composited (hidden, display asleep/locked, or a
non-interactive launch context). Codex's probe ran in exactly such a context.

**Proof both directions:** (1) the SAME patch run in a live interactive session: 6s,
`frames=601 acquires=597 presents=597`, ALL THREE injected faults recovered
(oom=27.9ms, surface_lost=6.2ms, device_lost=324.0ms) — the acquire/present/recovery code
is sound. (2) One diagnostic line `.with_visible(false)`: `frames=820, acquires=0,
presents=0, skipped=820` — byte-for-byte Codex's 1207-skip/0-present signature.

**Action:** just re-run `--soak-gpu` (15 min) in a session where the window is actually
on screen. No code fix needed for the mystery itself.

## Integration verdict: LOW risk, empirically proven
The uncommitted patch **applies cleanly onto current main** (24 commits ahead of the
1a44e30 base), builds, and the FULL suite passes. 4 file overlaps with main, all trivial.
Recommended: rebase onto main HEAD (not merge), re-run the suite, land via the normal train.
The feared lava-tick collision was REFUTED — the coupling (`last_frame`) exists and is
handled correctly; the double-schedule hazard cannot occur.

## Fix-before-merge list (all minor, none blocking)
1. **Per-kind skip counters** (`soak_gpu.rs` `observe_frame` collapses Timeout/Occluded/
   Reconfigured/Recreated/PrepareFailed into one `skipped`): 30s of pure occlusion is
   indistinguishable from a timeout storm — per-kind counters would have self-diagnosed
   this whole investigation. The strongest recommendation.
2. **PLAUSIBLE live gap:** `Occluded → WaitForWake` arms no timer and there is NO
   `WindowEvent::Occluded(false)` handler — an un-occluded window may not repaint until
   some other event arrives. Add the handler (or prove winit delivers an equivalent wake).
3. Single-slot `soak_recovery_pending` / `recovery_started[]`: back-to-back same-kind
   injections drop the first recovery timing.
4. `soak_gpu.rs` = 536 lines: past the ~500 ceiling, undeclared — trim or declare.
5. Stimulus starvation: resize demand monopolizes the schedule (themes=0, overlays=0 at
   6s / 597 presents) — interleave.
6. Name the `on_focus_gained` widening (lava-gated → unconditional redraw) in the commit
   message: benign but a bundled behavior change.
7. Note (acceptable): no error scopes — OOM lands via the async uncaptured handler,
   possibly one frame late.

## Clean bill on everything else
Capture gate structurally live-only (verified) · byte-identity double-runs hold ·
println_audit rebalanced correctly · hidden-flag conventions followed · editor state
provably survives GPU rebuild (buffers live on App, not Gpu) · every unrecoverable arm
reaches the real `exiting()` teardown · the self-reported teardown-test gap is genuine
but LOW risk (it IS the normal-quit path, 49 tests green).
