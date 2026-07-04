# ARCHITECTURE.md — awl's module map

How the pieces fit. For the *feel* see `DESIGN.md`; for *what's in/out of v1* see
`SCOPE.md`; for *how to verify headlessly* see `CAPTURE.md`. This doc is the wiring.

awl is a single Rust binary (crate `awl`): Rust + wgpu (2D only) + winit.
mac = Metal, linux = Vulkan. A personal prose-writing instrument with Emacs/`mg`
keybindings (see `SCOPE.md` — "Who this is for").

## The input → action → apply spine

The one path everything flows through:

```
key event ──▶ keymap ──▶ Action ──▶ apply_core ──▶ buffer / selection / search
 (winit       keymap.rs            actions.rs       buffer.rs, selection.rs,
  or --keys)                                         search.rs
```

- A keystroke (a live winit event, or a chord from `--keys`) resolves to a single
  `Action`.
- `Action` is the editor's command vocabulary — motions, edits, region ops, view
  ops, file ops. It is the seam between "what key was pressed" and "what the
  editor does."
- `apply_core` is the pure, GPU-/winit-/clipboard-free function that mutates
  document state for an `Action`. Both the live app and headless replay call it,
  so live and verified behavior cannot drift.

## Modules

Several of the larger modules are **directory modules**: the original `foo.rs`
stays as the module root (it keeps `mod foo;` in `main.rs`) and declares
`mod <topic>;` submodules that live in a sibling `foo/` directory — the same
precedent that split `render.rs` into `render/{caret,chrome,geometry,…}.rs`.
The split is a pure file-relocation (items lifted verbatim, visibility widened to
`pub(crate)`/`pub(super)` only where a sibling needs them, re-exported by bare
name); behavior is byte-identical. Submodules are listed under each root below.

**Entry / control**
- `main.rs` — entry point + CLI. Parses `Mode` (interactive window vs. headless
  `--screenshot` / `--screenshot-motion[-v]`, with optional `--keys`). For
  headless modes it loads the buffer, `replay_keys`, then hands off to capture.
  → `main/`: `args` (CLI / `Mode` parsing + folder resolution), `run` (the
  interactive + headless run paths).
- `app.rs` — the winit `ApplicationHandler`: window + event loop, owns the GPU
  renderer, mouse handling, and the live glue around `apply_core` (clipboard
  mirroring, GPU-measured page sizing, animation/redraw scheduling).
  → `app/`: `gpu` (device/surface setup), `files` (open/save/project glue),
  `viewstate` (view sync + paging), `input` (mouse/key event handling), `apply`
  (the `App::apply` wrapper around `apply_core` + app-only effects), `daemon`
  (the App-side half of the single-instance daemon below).
- `daemon.rs` — the SINGLE-INSTANCE DAEMON (native only,
  `cfg(not(target_arch = "wasm32"))`): a Unix domain socket beside the scratch
  stash (`fs::data_root().join("awl.sock")`). Owns the bind-or-handoff startup
  dance (`startup`/`bind_or_connect` — the stale-socket truth table), the
  dumb newline-delimited wire protocol (`format_open`/`parse_open`/
  `format_done`), and the accept-loop thread (`spawn_accept_thread`) that
  posts a `DaemonEvent` into the live winit event loop via
  `EventLoopProxy::send_event`. `app/daemon.rs` reacts to that event
  (`App::handle_daemon_event` → `load_path` + raise the window), and owns
  `Action::FinishBuffer` (C-x #, `commands.rs`'s "Finish Buffer") — save,
  notify any daemon `--wait` client, switch to the previous buffer. Lives
  ONLY on the live App's startup path (`app::run`), never on any headless
  `--screenshot`/`--bench-*` mode — see `daemon.rs`'s module doc for the full
  capture-gate argument and CLAUDE.md's Daemon section for the doors.

**Editor core (renderer-agnostic logic)**
- `actions.rs` — `ActionCtx` + `apply_core`: the shared apply seam (above).
  → `actions/`: `edit` (markdown smart-Enter), `flinch` (caret-feedback
  triggers), `motion` (oracle-aware motions + page scroll + search open),
  `overlay_nav` (modal overlay intercept + browse-path + live preview), `rebind`
  (the game-style rebind-menu key handling).
- `keymap.rs` — `KeymapState::resolve(key, mods) → Action`; defines the `Action`
  enum + `is_motion` / `is_edit`; table-driven, including the `C-x` prefix.
- `keyspec.rs` — `parse_keys("C-n M-> …") → Vec<Action>`: parses emacs key-spec
  strings by driving the *real* keymap. The headless analog of typing; powers
  `--keys`.
- `buffer.rs` — the document: a ropey rope, edit ops, cursor, undo/redo grouping,
  mark/anchor primitives.
  → `buffer/`: `edit`, `selection`, `motion`, `undo`, `focus`, `notes`, `tests`.
- `buffers.rs` — the MULTI-BUFFER REGISTRY: `BufferKey` (a buffer's stable
  identity — a path, or the one `Scratch` sentinel) + `BufferRegistry<T>` (the
  MRU-ordered, capped park/take store for every BACKGROUNDED buffer), shared
  verbatim by the live `App` (`app/files.rs`'s `BufferExtra` payload) and the
  headless `--keys` replay (`main/run.rs`'s `replay_keys`, payload `()`) — one
  owner of "open a file that's already open switches to its live buffer,"
  never two aligned copies. The ACTIVE buffer stays outside this module
  (`App::buffer` / the replay's `buffer` local, unchanged).
- `selection.rs` — the selection / region model (C-Space mark, kill/copy, drag).
- `search.rs` — incremental search (isearch) state + match finding.
- `spell.rs` / `spellunderline.rs` — spellcheck (spellbook) + underline data.

**Rendering / presentation**
- `render.rs` — all wgpu drawing: glyph atlas + shaping (glyphon), buffer text,
  the caret block, selection highlights, spell underlines, and the isearch panel
  card. The big file (still the largest in the tree).
  → `render/`: `caret`, `chrome` (status strip / HUD card / readout), `geometry`,
  `rowgeom` (per-row geometry table for variable heading heights), `spans`
  (md/CJK/syntax/focus `AttrsList` layering), `text`, `focus`, `rects`, `layers`.
- `caret.rs` — caret position + its springy motion/glide animation (the "streak"
  / motion work).
  → `caret/`: `spring`, `morph`, `juice`, `preview`, `pipeline`, `tests`.
- `theme.rs` — palette tokens (BASE_* greys, the single amber accent).

**Verification**
- `capture.rs` — headless one-frame capture: render to an offscreen texture, read
  back pixels → PNG + JSON sidecar (`awl-capture/2`). The agent-facing contract;
  see `CAPTURE.md`.
  → `capture/`: `opts` (capture options), `modes` (the capture entry paths),
  `gpu` (offscreen device/readback), `animated` (motion-frame capture), `oracle`,
  `sidecar` (the JSON sidecar emitter), `tests`.
- `bench.rs` — microbenchmarks.

## Two flows, one engine

1. **Live:** winit event → `app.rs` → `keymap::resolve` → `Action` →
   `actions::apply_core` (+ app-only concerns) → `render.rs` draws the next frame.
2. **Headless verify:** `--keys "spec"` → `keyspec::parse_keys` → `Vec<Action>` →
   `replay_keys` / `apply_core` (same seam) → `capture.rs` renders one
   deterministic frame → PNG + sidecar.

Because both flows share `keymap` + `apply_core`, a headless capture exercises the
real edit logic rather than a mock.

## Known gaps — behavior that lives ONLY in `app.rs`, so it doesn't replay yet

- **Search-query input** routes to the buffer in headless replay (the
  query-routing still lives in `App::apply`, not `apply_core`).
- **Save side effect:** replaying `C-x C-s` writes the file to disk during a
  capture.
- **Finish Buffer (C-x #):** replaying it writes the file to disk (same
  `Buffer::save` call as `C-x C-s`, above), but the daemon-notify + buffer-swap
  half is App-only — a headless replay treats the `Effect::FinishBuffer` it
  signals as a no-op (mirrors `LastBuffer`; no daemon, no 2-deep buffer history
  in a one-shot replay). See `daemon.rs`'s module doc.
- **Clipboard + GPU-measured paging** intentionally stay in `app.rs` (they need
  the OS / window); headless paging uses a fixed page size.
