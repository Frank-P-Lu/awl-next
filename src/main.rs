//! awl — a fast native editor (skeleton stage).
//!
//! Usage:
//!   awl [file]                              open windowed editor (file optional)
//!   awl --screenshot OUT.png [file]         headless: one frame, caret at rest (rounded square)
//!   awl --screenshot-motion OUT.png [file]  headless: one frame, caret mid-glide (trailing underline)
//!
//! Deterministic verification hooks (compose with --screenshot):
//!   --sel L0:C0-L1:C1   draw a selection highlight from (line0,col0)..(line1,col1)
//!   --zoom F            render at zoom factor F (e.g. 1.6); clamped to [0.5,3.0]
//!   --scroll N          scroll N VISUAL rows off the top (free scroll, clamped)
//!   --preedit STR       render STR as an IME preedit (underlined) at the caret
//!   --theme NAME        set the active color theme/world before capture (e.g. Quokka)
//!   --caret-mode MODE   caret look: block | morph | auto (default: font-derived)
//!   --keys "SPEC"       replay a space-separated emacs key-spec against the freshly
//!                       loaded buffer THROUGH THE REAL KEYMAP, then capture the
//!                       post-replay editor state (e.g. --keys "C-n C-n M->")

mod about;
mod actions;
mod app;
// The two halves of this binary's front matter, split out of a once-monster
// `main.rs` into a `main/` directory (an explicit `#[path]` because `main.rs` is
// the crate root, so its submodules do not auto-resolve into a `main/` dir like a
// non-root module's would). `args` parses the CLI into a `Mode`; `run` executes
// it. `fn main` + the wasm entry below stay thin.
#[path = "main/args.rs"]
mod args;
#[path = "main/run.rs"]
mod run;
mod background;
mod bench;
mod buffer;
mod buffers;
mod capture;
mod caret;
mod caret_glyph;
mod clock;
mod commands;
mod config;
mod cursor_shape;
mod daemon;
mod debug;
mod ease;
mod facets;
mod focus;
mod frontmatter;
mod fs;
mod fuzzy;
mod history;
mod hud;
mod index;
mod keymap;
mod keyspec;
mod mac_chrome;
mod markdown;
mod menu;
mod menu_icons;
mod nits;
mod overlay;
mod page;
mod pointer_hide;
mod project;
mod recent_files;
mod recents;
mod render;
mod script;
mod search;
mod selection;
mod session;
mod spell;
mod spellunderline;
mod syntax;
mod theme;
mod whichkey;
// The CORE web/wasm smoke suite — inert on native (never compiles there), built
// only for the wasm target's `cargo test`. See `scripts/web-smoke.sh`.
#[cfg(all(test, target_arch = "wasm32"))]
mod websmoke;

use anyhow::Result;

// Re-exported across the crate so call sites keep resolving these by their old
// `crate::` paths after the move into `main/`: `app.rs` reads the notes/workspace
// resolvers.
pub(crate) use args::resolve_notes_root;
pub(crate) use run::resolve_workspace;

#[cfg(target_arch = "wasm32")]
use std::path::PathBuf;
#[cfg(target_arch = "wasm32")]
use crate::config::Config;

// --- WASM (browser) entry ---------------------------------------------------
//
// awl is a BINARY crate; `fn main` stays the NATIVE entry. The browser, though,
// can't auto-run a wasm bin's `main`, so the web build enters through this
// wasm-bindgen `start` function instead (Trunk's generated JS loader calls it on
// module instantiation). It installs the panic hook + console logger, then starts
// the same winit/wgpu app `main`'s windowed path runs — only ASYNC + non-blocking
// (see `app::run`'s wasm split). On wasm `fn main` is compiled but never invoked.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_start() {
    // Route Rust panics to the browser console with a readable stack.
    console_error_panic_hook::set_once();
    // `log::*` -> the browser console (Info+; ignore a double-init).
    let _ = console_log::init_with_level(log::Level::Info);
    log::info!("awl: starting (wasm)");

    // Install the BROWSER filesystem (localStorage) as the active backend and
    // seed the bundled sample docs on first load, so the editor opens with real,
    // reload-persistent content instead of the disk-less default `NativeFs`.
    fs::install_web_fs();

    // The sandbox has no CLI / cwd, so the virtual project root is "/" (where the
    // samples are seeded), notes + workspace folders are "/" too (so C-x n / C-x p
    // operate within the seeded fs), and config is empty. Open the seeded welcome
    // note so there is content + markdown styling from the first frame. `app::run`
    // returns immediately on wasm (`spawn_app` hands off to requestAnimationFrame).
    let root = PathBuf::from("/");
    let welcome = Some(PathBuf::from("/welcome.md"));
    if let Err(e) = app::run(
        welcome,
        root,
        Some(PathBuf::from("/")),
        Some(PathBuf::from("/")),
        Config::empty(),
        false, // --wait is a native-only, single-instance-daemon concern
    ) {
        log::error!("awl failed to start: {e}");
    }
}

// The native entry stays thin: parse the CLI into a `Mode`, then execute it. The
// parsing lives in `main::args`, the per-mode work in `main::run`.
fn main() -> Result<()> {
    // `--print-menu-roster`: a hidden, macOS-only diagnostic flag that prints
    // `menu::roster()` as plain lines and exits — NEVER touches a window/event
    // loop, so it works with no display attached. This is the ONE door
    // `scripts/smoke-menus.sh` (the live menu-click smoke tier) uses to
    // enumerate exactly what to click, straight from the same data
    // `menu::build_menu` translates into the real menu bar — so the smoke
    // script's roster can never silently drift from the app's own. Checked
    // BEFORE `args::parse_args` (which has no concept of this flag) so it
    // short-circuits every other mode.
    #[cfg(target_os = "macos")]
    if std::env::args().any(|a| a == "--print-menu-roster") {
        menu::print_roster();
        return Ok(());
    }
    // `--dump-menu-icon <id> <out.png>`: a hidden, macOS-only DEV diagnostic
    // that rasterizes one routed menu id's SF Symbol via the exact live path
    // (`menu_icons::symbol_for` → `mac_chrome::render_symbol_rgba`) and writes
    // the resulting RGBA to a PNG. `main()` runs ON the process main thread, so
    // `MainThreadMarker::new()` succeeds here — the ONE headless-ish door to
    // confirm the SF-Symbol path actually rasterizes a real glyph (a non-empty
    // alpha channel) rather than silently bailing to the procedural fallback.
    #[cfg(target_os = "macos")]
    if let Some(pos) = std::env::args().position(|a| a == "--dump-menu-icon") {
        use anyhow::Context;
        let mut rest = std::env::args().skip(pos + 1);
        let id = rest.next().context("--dump-menu-icon needs <id> <out.png>")?;
        let out = rest.next().context("--dump-menu-icon needs <id> <out.png>")?;
        let symbol =
            menu_icons::symbol_for(&id).with_context(|| format!("no SF Symbol for menu id {id}"))?;
        let (rgba, w, h) = mac_chrome::render_symbol_rgba(symbol)
            .context("render_symbol_rgba returned None (off main thread / AppKit step failed)")?;
        let covered = rgba.chunks_exact(4).filter(|px| px[3] != 0).count();
        image::RgbaImage::from_raw(w, h, rgba)
            .context("failed to build RgbaImage from symbol RGBA")?
            .save(&out)
            .with_context(|| format!("failed to write PNG {out}"))?;
        println!("wrote {out} ({w}x{h}, {covered} covered px) for {id} → {symbol}");
        return Ok(());
    }
    run::run(args::parse_args()?)
}
