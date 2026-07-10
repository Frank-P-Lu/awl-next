//! The MAS (Mac App Store / App Sandbox) flavor — a `cargo feature "mas"`,
//! **OFF by default** so the default build stays byte-identical (see
//! `mas::tests::default_build_never_reaches_this_file` in `main.rs`'s law
//! test and `CLAUDE.md`'s engineering-principles section). Everything here
//! compiles out entirely unless `--features mas` is passed — this whole file
//! is gated `#![cfg(feature = "mas")]`.
//!
//! **THE CONTAINER REALITY CHECK (done first, per the round's own
//! instruction, before writing any code):** under App Sandbox, macOS
//! redirects the sandboxed process's OWN `$HOME` environment variable to
//! point INSIDE the app's container
//! (`~/Library/Containers/<bundle-id>/Data`) — this is standard, documented
//! App Sandbox behavior, not something awl has to arrange. Reading
//! `src/fs.rs` and `src/config/model.rs` confirms every path this app writes
//! on its own initiative already resolves through `$HOME`/`$XDG_DATA_HOME`/
//! `$XDG_CONFIG_HOME` env reads alone:
//!   - `fs::data_root()` (the scratch stash + local history root) —
//!     `$XDG_DATA_HOME/awl` else `$HOME/.local/share/awl`.
//!   - `config::config_path()` — `$AWL_CONFIG` else `$XDG_CONFIG_HOME/awl/…`
//!     else `$HOME/.config/awl/config.toml`.
//! Neither touches a hard-coded absolute path or any sandbox-unaware API —
//! so under App Sandbox both land INSIDE the container automatically, with
//! ZERO grant machinery involved and ZERO code change needed for the
//! scratch-first launch story (no-path buffer, autosave, local history, the
//! config file). [`within_home`] below is the SAME set of env reads, in the
//! SAME precedence, as a portable predicate — the zero-grants law tests
//! assert against it directly.
//!
//! **What this module ADDS, then:** only the machinery for the moment awl
//! reaches OUTSIDE that automatic safety — opening a file / switching a
//! project root / jumping to `notes_root` / `workspace` that live outside
//! the container. That is the iA Writer / powerbox model: the FIRST touch of
//! an ungranted root prompts the system `NSOpenPanel` (a folder picker,
//! restricted + pre-navigated near the target); the chosen folder is
//! persisted as a SECURITY-SCOPED BOOKMARK (`NSURL` bookmark data, resolved +
//! `startAccessingSecurityScopedResource`d on every later launch); and once
//! granted, every one of awl's OWN pickers (go-to, browse, project switch)
//! roams freely inside that root — never a second prompt, never a forked
//! picker.
//!
//! **The two halves:**
//!   - [`GrantStore`] / [`load`] / [`save`] / [`to_toml`] / [`from_toml`] —
//!     the PURE grant-persistence model (hand-rolled TOML writer + the
//!     crate's existing `toml` PARSER for reading back, mirroring
//!     `session.rs`'s exact idiom), unit-testable on every platform with no
//!     AppKit at all.
//!   - `mac::ensure_access` / `mac::restore_all_grants` (`cfg(target_os =
//!     "macos")` on top of `cfg(feature = "mas")`) — the actual `NSOpenPanel`
//!     + security-scoped-bookmark AppKit calls, LIVE-ONLY (a real modal panel
//!     is OS UI the headless harness cannot drive — mirrors `mac_chrome.rs`'s
//!     own documented boundary) and so NOT unit-tested; [`fence`] carries the
//!     one piece of that logic worth testing on its own (which root a path
//!     resolves under), with the panel itself injected/mocked out.
//!
//! **Daemon compiled out:** under `mas`, `crate::daemon` (the single-instance
//! Unix-socket daemon) does not exist — Launch Services already prevents a
//! second launch of a Mac App Store app, and a sandboxed app has no CLI
//! story to hand a path to a running instance anyway. See `crate::daemon`'s
//! own `#![cfg(...)]` line and every `not(feature = "mas")` gate this round
//! added around its call sites in `app.rs` / `app/daemon.rs`.
#![cfg(feature = "mas")]

use std::path::{Path, PathBuf};

/// One persisted folder grant: the ROOT the user chose in the powerbox panel,
/// plus the security-scoped bookmark data macOS needs to re-resolve access to
/// it on a later launch. `bookmark` is opaque bytes as far as this module's
/// pure half is concerned — only `mac::` (macOS-only) ever creates/resolves
/// one via AppKit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub root: PathBuf,
    pub bookmark: Vec<u8>,
}

/// The whole persisted grant set — every folder the user has ever granted
/// awl access to outside the sandbox container, across every launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantStore {
    pub grants: Vec<Grant>,
}

impl GrantStore {
    /// The MOST SPECIFIC granted root that is an ancestor of (or equal to)
    /// `path`, if any — "most specific" so a grant on `/Users/me/code`
    /// doesn't shadow a later, narrower grant on `/Users/me/code/sub` (not
    /// that awl ever needs the narrower one specifically; it just picks
    /// deterministically rather than arbitrarily on overlapping grants).
    pub fn granted_root_for(&self, path: &Path) -> Option<&Grant> {
        self.grants
            .iter()
            .filter(|g| path.starts_with(&g.root))
            .max_by_key(|g| g.root.as_os_str().len())
    }

    /// Record (or replace) the grant for `root`. A re-grant of an already-
    /// known root (e.g. the bookmark went stale and was re-minted) replaces
    /// its bookmark bytes in place rather than growing a duplicate entry.
    pub fn upsert(&mut self, root: PathBuf, bookmark: Vec<u8>) {
        match self.grants.iter_mut().find(|g| g.root == root) {
            Some(existing) => existing.bookmark = bookmark,
            None => self.grants.push(Grant { root, bookmark }),
        }
    }
}

/// Where the grant store lives: beside the scratch stash and the session
/// file, under the app's data root (which — see the module doc's container
/// finding — already resolves inside the sandbox container for free).
pub fn grants_path() -> PathBuf {
    crate::fs::data_root().join("grants.toml")
}

/// Load the persisted grant store from `path` through the active
/// `FileSystem` backend. A missing or unparseable file degrades to an EMPTY
/// [`GrantStore`] — never a crash — mirroring `Config::load` / `session::load`.
pub fn load(path: &Path) -> GrantStore {
    match crate::fs::active().read_to_string(path) {
        Ok(src) => from_toml(&src),
        Err(_) => GrantStore::default(),
    }
}

/// Persist `store` to `path` ATOMICALLY (temp-sibling + rename via
/// [`crate::fs::write_atomic`] — the same primitive the autosave engine, the
/// scratch stash, and `session::save` all use).
pub fn save(path: &Path, store: &GrantStore) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = crate::fs::active().create_dir_all(parent);
    }
    crate::fs::write_atomic(path, to_toml(store).as_bytes())
}

/// Serialize `store` to the on-disk TOML shape — pure, no fs. Hand-rolled
/// (mirrors `session.rs::to_toml`) since the crate's `toml` dependency loads
/// with only the `parse` feature; reading it back goes through the real
/// `toml` parser via [`from_toml`], so the two halves never have to
/// hand-agree on escaping. Bookmark bytes (opaque, binary, may contain any
/// byte) are stored as a lowercase HEX string — the simplest encoding that
/// survives a plain TOML basic string with zero escaping edge cases.
pub fn to_toml(store: &GrantStore) -> String {
    let mut out = String::new();
    for g in &store.grants {
        out.push_str("[[grant]]\n");
        out.push_str(&format!("root = {}\n", quote_path(&g.root)));
        out.push_str(&format!("bookmark = \"{}\"\n\n", hex_encode(&g.bookmark)));
    }
    out
}

/// Parse the on-disk TOML shape back into a [`GrantStore`] — pure, no fs.
/// LENIENT throughout (mirrors `session::from_toml` / `Config::load`): an
/// unparseable document, a missing field, or an un-hex-decodable bookmark
/// string is simply skipped rather than erroring, so a half-written or
/// hand-edited grants file never blocks a launch.
pub fn from_toml(src: &str) -> GrantStore {
    let mut store = GrantStore::default();
    let Ok(table) = src.parse::<toml::Table>() else {
        return store;
    };
    let Some(arr) = table.get("grant").and_then(|v| v.as_array()) else {
        return store;
    };
    for entry in arr {
        let Some(t) = entry.as_table() else { continue };
        let Some(root) = t.get("root").and_then(|v| v.as_str()) else { continue };
        let Some(bm) = t.get("bookmark").and_then(|v| v.as_str()) else { continue };
        let Some(bytes) = hex_decode(bm) else { continue };
        store.grants.push(Grant { root: PathBuf::from(root), bookmark: bytes });
    }
    store
}

/// A path as a quoted + escaped TOML basic string (identical rule to
/// `session.rs::quote`).
fn quote_path(p: &Path) -> String {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Lowercase-hex-encode `bytes` — pure, allocation-only, no external crate
/// (a bookmark blob is a few hundred bytes at most; not worth a dependency).
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The inverse of [`hex_encode`]. `None` on odd length or any non-hex digit
/// (a hand-edited or corrupted grants file) — the caller drops that one
/// entry rather than failing the whole load.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

/// True if `path` lies under the SAME set of env-resolved roots
/// [`crate::fs::data_root`] / [`crate::config::config_path`] already use —
/// under App Sandbox this IS the container (see the module doc), so no grant
/// is ever needed for it. Pure (env-var reads only), so unit-testable
/// without any AppKit / sandbox involved — the zero-grants-first-run law
/// tests assert against this directly with an injected `$HOME`.
pub fn within_home(path: &Path) -> bool {
    for var in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "HOME"] {
        if let Some(v) = std::env::var_os(var) {
            if path.starts_with(PathBuf::from(v)) {
                return true;
            }
        }
    }
    false
}

// --- macOS-only: the actual powerbox + security-scoped-bookmark calls ------
//
// LIVE-ONLY (needs human confirmation): a real `NSOpenPanel` modal and a real
// `startAccessingSecurityScopedResource` round-trip are exactly the kind of
// OS-UI/sandbox-kernel interaction the headless capture harness cannot
// construct (mirrors `mac_chrome.rs`'s identical, already-documented
// boundary). Nothing here is unit-tested; [`fence::ensure_access_decision`]
// factors the one PURE decision this file makes (container / already-granted
// / needs-a-panel) out into something that IS unit-tested with the panel
// call itself injected.
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{
        NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
    };
    use std::sync::{Mutex, OnceLock};

    /// The in-process cache of the grant store, loaded once and kept warm for
    /// the rest of THIS launch (every `ensure_access` call re-checks against
    /// it instead of re-reading disk). Persisted back to [`super::grants_path`]
    /// on every successful new grant.
    fn store_lock() -> &'static Mutex<GrantStore> {
        static STORE: OnceLock<Mutex<GrantStore>> = OnceLock::new();
        STORE.get_or_init(|| Mutex::new(super::load(&super::grants_path())))
    }

    /// Resolve `bookmark` back to a URL and START security-scoped access to
    /// it, returning the resolved path on success. `None` on ANY failure
    /// (a stale/revoked bookmark, off-main-thread, …) — the caller then
    /// treats the root as ungranted again (it will be dropped from the live
    /// cache and, on the next touch, re-prompted via the panel).
    fn start_accessing(bookmark: &[u8]) -> Option<PathBuf> {
        let data = NSData::with_bytes(bookmark);
        // SAFETY: `is_stale` is passed null (we don't act on staleness in v1 —
        // a stale-but-still-resolvable bookmark still grants access; a fully
        // revoked one simply fails to resolve at all, handled below).
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                std::ptr::null_mut(),
            )
        }
        .ok()?;
        // SAFETY: standard security-scoped-resource access start; balanced by
        // `stopAccessingSecurityScopedResource` never being needed here since
        // awl holds access for its whole process lifetime (no explicit stop —
        // the OS reclaims it on process exit).
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return None;
        }
        let path = url.path()?;
        Some(PathBuf::from(path.to_string()))
    }

    /// Called ONCE at native macOS launch (see `App::run`'s wiring): resolve
    /// + start accessing every persisted grant, so a relaunch's FIRST touch
    /// of a previously-granted root needs no panel. A grant whose bookmark no
    /// longer resolves (revoked in System Settings, the folder moved/deleted)
    /// is silently dropped from the LIVE cache (never from disk here — the
    /// next successful re-grant of that root will overwrite it via
    /// `ensure_access`'s own `save`) rather than panicking or blocking launch.
    pub fn restore_all_grants() {
        let mut store = store_lock().lock().unwrap_or_else(|e| e.into_inner());
        store.grants.retain(|g| start_accessing(&g.bookmark).is_some());
    }

    /// Run the standard OPEN panel restricted to FOLDERS ONLY, pre-navigated
    /// near `target`, and return the chosen folder + its freshly-minted
    /// security-scoped bookmark. `None` on Cancel / off-main-thread (mirrors
    /// `mac_chrome::pick_file_to_open`'s exact shape, folder-only instead of
    /// file-only).
    fn pick_folder_grant(target: &Path) -> Option<(PathBuf, Vec<u8>)> {
        let mtm = MainThreadMarker::new()?;
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(false);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setMessage(Some(&NSString::from_str(
            "awl needs your permission to open files in this folder.",
        )));
        // Pre-navigate near the target: itself if it's already a directory,
        // else its nearest existing ancestor.
        let start = if target.is_dir() {
            Some(target.to_path_buf())
        } else {
            target.parent().map(|p| p.to_path_buf())
        };
        if let Some(s) = start.as_deref().and_then(|p| p.to_str()) {
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(s))));
        }
        if panel.runModal() != NSModalResponseOK {
            return None;
        }
        let url = panel.URL()?;
        let bookmark = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithSecurityScope,
                None,
                None,
            )
            .ok()?;
        let path = url.path()?;
        Some((PathBuf::from(path.to_string()), bookmark.to_vec()))
    }

    /// THE GATE every "reach outside the container" door calls before
    /// touching `target`: (1) inside the sandbox container — always fine,
    /// zero grant machinery; (2) inside an already-granted root (persisted
    /// from an earlier launch/touch, resolved + accessing since
    /// [`restore_all_grants`] or an earlier call this session) — fine;
    /// (3) otherwise, powerbox the user via the folder panel, persist the
    /// resulting bookmark, and start accessing it right away. Returns
    /// `false` ONLY when the user genuinely CANCELLED the panel — every call
    /// site aborts the open/switch in that case rather than let it fail
    /// against a sandbox `EPERM`.
    ///
    /// **Off-main-thread degrades to ALLOW, not deny (deliberate, and safe):**
    /// every real call site runs on the winit main thread by construction
    /// (`App::load_path`/`App::set_root`, themselves only ever reached from
    /// `resumed()`/`user_event()`/a keymap dispatch) — `MainThreadMarker`
    /// failing can therefore ONLY happen inside a `cargo test` worker thread,
    /// never live. Falling through to "allow" there does NOT weaken the real
    /// sandbox: this function is an ADVISORY layer deciding whether to show a
    /// panel, not the actual access grant — the kernel-enforced sandbox
    /// (App Sandbox + the entitlements in `packaging/mas/entitlements.plist`)
    /// still refuses the real disk read/write regardless of what this
    /// returns, so an ungranted path simply surfaces as a normal read error
    /// downstream. Denying here instead would silently break every existing
    /// hermetic `App` test that happens to open a path outside `$HOME` — a
    /// worse outcome for zero extra safety.
    pub fn ensure_access(target: &Path) -> bool {
        let decision = {
            let store = store_lock().lock().unwrap_or_else(|e| e.into_inner());
            super::fence::ensure_access_decision(target, &store)
        };
        match decision {
            super::fence::AccessDecision::Allowed => true,
            super::fence::AccessDecision::NeedsGrant => {
                if MainThreadMarker::new().is_none() {
                    return true;
                }
                let Some((root, bookmark)) = pick_folder_grant(target) else {
                    return false;
                };
                let mut store = store_lock().lock().unwrap_or_else(|e| e.into_inner());
                store.upsert(root, bookmark);
                let _ = super::save(&super::grants_path(), &store);
                true
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{ensure_access, restore_all_grants};

/// The PURE decision `mac::ensure_access` makes, factored out so it is
/// unit-testable with the real `NSOpenPanel` call never in the loop (the
/// panel itself stays LIVE-ONLY — see the module doc). Exists on every
/// platform `mas` compiles for (not just macOS) so the fence logic can be
/// exercised in a plain `cargo test --features mas` run even off a Mac.
pub mod fence {
    use super::{within_home, GrantStore};
    use std::path::Path;

    /// What [`ensure_access_decision`] found for a given target, BEFORE any
    /// panel is involved.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AccessDecision {
        /// Already fine — inside the container, or inside an already-granted
        /// root — proceed with no user interaction at all.
        Allowed,
        /// Outside both — the caller must show the folder-picker panel next.
        NeedsGrant,
    }

    /// The pure fence check: is `target` inside the container, or inside a
    /// root `store` already has a grant for? Everything else needs a fresh
    /// grant. This is the ENTIRE decision `mac::ensure_access` makes before
    /// ever touching AppKit — kept here, panel-free, so it is provable with
    /// nothing more than a `GrantStore` and a path.
    pub fn ensure_access_decision(target: &Path, store: &GrantStore) -> AccessDecision {
        if within_home(target) {
            return AccessDecision::Allowed;
        }
        if store.granted_root_for(target).is_some() {
            return AccessDecision::Allowed;
        }
        AccessDecision::NeedsGrant
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_store_round_trips_through_toml() {
        let store = GrantStore {
            grants: vec![
                Grant { root: PathBuf::from("/Users/me/Documents/notes"), bookmark: vec![0, 1, 2, 253, 254, 255] },
                Grant { root: PathBuf::from("/Volumes/External/code"), bookmark: vec![0xde, 0xad, 0xbe, 0xef] },
            ],
        };
        let text = to_toml(&store);
        assert_eq!(from_toml(&text), store);
    }

    #[test]
    fn hex_round_trips_every_byte_value() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)), Some(bytes));
    }

    #[test]
    fn hex_decode_rejects_odd_length_and_non_hex() {
        assert_eq!(hex_decode("abc"), None); // odd length
        assert_eq!(hex_decode("zz"), None); // not hex digits
        assert_eq!(hex_decode(""), Some(vec![]));
    }

    #[test]
    fn from_toml_is_lenient_about_garbage() {
        assert_eq!(from_toml("this is = = not valid toml [[["), GrantStore::default());
        // A well-formed table but a bad bookmark hex string: the entry is
        // dropped, not the whole load.
        let store = from_toml("[[grant]]\nroot = \"/a\"\nbookmark = \"zz\"\n");
        assert!(store.grants.is_empty());
        // A missing root or bookmark key: same, dropped not fatal.
        let store = from_toml("[[grant]]\nroot = \"/a\"\n");
        assert!(store.grants.is_empty());
    }

    #[test]
    fn load_missing_file_degrades_to_empty() {
        // Through the InMemoryFs seam: no file at all.
        use std::sync::Arc;
        let fs = Arc::new(crate::fs::InMemoryFs::new());
        crate::fs::with_fs(fs, || {
            assert_eq!(load(Path::new("/nonexistent/grants.toml")), GrantStore::default());
        });
    }

    #[test]
    fn save_then_load_round_trips_through_a_fake_disk() {
        use std::sync::Arc;
        let fs = Arc::new(crate::fs::InMemoryFs::new().with_dir("/data"));
        crate::fs::with_fs(fs, || {
            let store = GrantStore {
                grants: vec![Grant { root: PathBuf::from("/proj"), bookmark: vec![1, 2, 3] }],
            };
            let p = PathBuf::from("/data/grants.toml");
            save(&p, &store).unwrap();
            assert_eq!(load(&p), store);
        });
    }

    #[test]
    fn granted_root_for_picks_the_most_specific_ancestor() {
        let store = GrantStore {
            grants: vec![
                Grant { root: PathBuf::from("/Users/me"), bookmark: vec![1] },
                Grant { root: PathBuf::from("/Users/me/code/proj"), bookmark: vec![2] },
            ],
        };
        let g = store.granted_root_for(Path::new("/Users/me/code/proj/src/main.rs")).unwrap();
        assert_eq!(g.root, PathBuf::from("/Users/me/code/proj"), "the narrower, more specific grant wins");
        // A sibling outside the narrower grant still resolves the broader one.
        let g = store.granted_root_for(Path::new("/Users/me/other/file.md")).unwrap();
        assert_eq!(g.root, PathBuf::from("/Users/me"));
        // Nothing granted covers this path at all.
        assert!(store.granted_root_for(Path::new("/Volumes/External/x")).is_none());
    }

    #[test]
    fn upsert_replaces_an_existing_roots_bookmark_in_place() {
        let mut store = GrantStore::default();
        store.upsert(PathBuf::from("/a"), vec![1, 2, 3]);
        store.upsert(PathBuf::from("/b"), vec![9]);
        assert_eq!(store.grants.len(), 2);
        store.upsert(PathBuf::from("/a"), vec![9, 9]);
        assert_eq!(store.grants.len(), 2, "re-granting an existing root replaces, never duplicates");
        assert_eq!(store.granted_root_for(Path::new("/a")).unwrap().bookmark, vec![9, 9]);
    }

    // --- THE ZERO-GRANTS-FIRST-RUN LAW ------------------------------------
    //
    // `within_home` mirrors `fs::data_root()` / `config::config_path()`'s own
    // env-var precedence EXACTLY (see the module doc's container-reality
    // finding), so these tests double as the "scratch/autosave/history/config
    // never need a grant" law: every one of those four systems' paths is
    // built from `data_root()`/`config_path()` alone, both of which resolve
    // under this same env-var ladder.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn within_home_true_for_paths_under_home() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let snap = ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "HOME"].map(|k| (k, std::env::var_os(k)));
        // SAFETY: serialized by ENV_LOCK, mirroring config::tests' identical pattern.
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("HOME", "/Users/sandboxed-container-home");
        }
        assert!(within_home(Path::new(
            "/Users/sandboxed-container-home/.local/share/awl/scratch.md"
        )));
        assert!(within_home(Path::new(
            "/Users/sandboxed-container-home/.config/awl/config.toml"
        )));
        assert!(!within_home(Path::new("/Users/other-app-or-volume/notes")));
        for (k, v) in &snap {
            // SAFETY: serialized by ENV_LOCK.
            unsafe {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn zero_grants_law_data_root_and_config_path_always_resolve_within_home() {
        // The actual LAW: whatever data_root()/config_path() resolve to under
        // the current process env, `within_home` agrees it needs no grant —
        // i.e. the scratch stash, local history, and config file NEVER
        // trigger the powerbox panel, on any launch, sandboxed or not.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(within_home(&crate::fs::data_root()));
        assert!(within_home(&crate::config::config_path(None)));
        assert!(within_home(&crate::fs::scratch_stash_path()));
    }

    #[test]
    fn ensure_access_decision_allows_home_with_an_empty_store() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("HOME", "/Users/me") };
        let store = GrantStore::default();
        assert_eq!(
            fence::ensure_access_decision(Path::new("/Users/me/anything"), &store),
            fence::AccessDecision::Allowed
        );
        assert_eq!(
            fence::ensure_access_decision(Path::new("/Volumes/External/x"), &store),
            fence::AccessDecision::NeedsGrant
        );
    }

    #[test]
    fn ensure_access_decision_allows_an_already_granted_root_without_a_panel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("HOME", "/Users/me") };
        let mut store = GrantStore::default();
        store.upsert(PathBuf::from("/Volumes/External/code"), vec![1, 2, 3]);
        assert_eq!(
            fence::ensure_access_decision(Path::new("/Volumes/External/code/src/main.rs"), &store),
            fence::AccessDecision::Allowed,
            "a path inside a granted root resolves without ever signaling the panel"
        );
        assert_eq!(
            fence::ensure_access_decision(Path::new("/Volumes/External/other/x"), &store),
            fence::AccessDecision::NeedsGrant,
            "a sibling folder OUTSIDE the granted root still needs its own grant"
        );
    }
}
