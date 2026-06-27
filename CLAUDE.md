# CLAUDE.md — working on awl-next

awl is a calm, native (Rust + wgpu + winit + glyphon) editor for **prose and
light code editing**, with Emacs/`mg` keybindings. Personal tool — audience: one.

Read these first; they are the contract:
- **SCOPE.md** — what's in/out of scope; the audience decision; find / themes / nav / notes model.
- **DESIGN.md** — the *feel*: Swiss discipline + game-juice, one warm living thing, figure/ground by value.
- **CAPTURE.md** — the headless verification harness (your primary verification path).
- **ARCHITECTURE.md** — the module map.

## Build & test (ALWAYS prefix the toolchain PATH)
```sh
export PATH="/Users/frank/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo build      # run from /Users/frank/code2026/awl-next
cargo test
```
- **Do NOT `cargo clean`** — incremental builds are fine; a clean rebuild is slow and wasteful.
- **Edit in place.** Match the surrounding style (table-driven, allocation-light, doc comments in each file's voice).

## Verify headlessly — the JSON sidecar is the source of truth
```sh
cargo run -- --screenshot OUT.png [file]   # writes OUT.png AND OUT.json (sidecar)
```
Flags compose:
- `--keys "C-n C-n M->"` — replay emacs chords through the **real keymap** before the capture.
- `--theme <World>` — Tawny | Potoroo | Outback | Undertow | Gumtree | Bilby | Saltpan | Quokka.
- `--caret-mode block|morph|ibeam|auto`
- `--measure <chars>` — page-mode column width (use a NARROW value, e.g. 40, to see the margins on the 1200px canvas).
- `--screenshot-motion[-v|-d]` — one mid-glide frame (horizontal | vertical | diagonal).
- `--root <dir> --workspace <dir> --notes-root <dir>` — project / notes context.

Read `OUT.json` (schema `awl-capture/N`, documented in CAPTURE.md) for state:
`cursor, selection, search, project, overlay, theme, page, caret_mode, focus`.
**Prefer the sidecar over eyeballing the PNG**; use the PNG only for visual/geometry confirmation.

## What the harness can and can't verify
- **CAN:** state, geometry, layout, colors, and deterministic single-frame *trajectories* (via `--screenshot-motion`).
- **CANNOT (today):** timing/feel over real time, and subjective taste. A frozen frame can't show a glide's *speed* or a fade's *progression*. Flag those for **live human confirmation** — do not claim them "verified."

## Conventions
- **Determinism:** the headless path has NO clock / animation / random. Don't add one. Live-only animation must render its *settled* state in capture.
- **Input path:** keys → `keymap.rs` (`Action`) → `actions.rs::apply_core`. Keep every new interaction drivable by `--keys` AND reflected in the sidecar, so it stays agent-verifiable.
- **Design discipline (DESIGN.md):** one accent (the caret/primary); figure/ground by value; transient *summoned* overlays, never persistent chrome.
