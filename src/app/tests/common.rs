use super::*;

// ── The AUTOSAVE ENGINE (App-level, over the InMemoryFs seam) ───────────
//
// Each test installs a fake FS via FsGuard so App::new / the flush paths
// never touch the real disk (or the developer's real scratch stash).

/// An App over the installed fake FS, opened on `file` with project `root`.
pub(super) fn app_on(file: Option<PathBuf>, root: &str, config: Config) -> App {
    App::new(file, PathBuf::from(root), None, None, config)
}

impl App {
    /// TEST DRIVER: route one action through the SHARED core against this
    /// App's own buffer/overlay/search — the same seam a live keypress
    /// reaches after `App::apply`'s window-level intercepts (no event loop,
    /// no GPU). Minimal builder closures: an overlay-opening action would
    /// no-op, which the preview-path read-only tests never need.
    pub(super) fn apply_core_for_test(&mut self, action: &Action) -> crate::actions::Effect {
        let mut shift = self.shift_selecting;
        let mut zoom = self.zoom;
        let mut make = |_k: crate::overlay::OverlayKind| None;
        let mut browse = |_k: crate::overlay::OverlayKind, _p: Option<String>| None;
        let mut ctx = crate::actions::ActionCtx {
            buffer: &mut self.buffer,
            shift_selecting: &mut shift,
            zoom: &mut zoom,
            search: &mut self.search,
            scroll_page_lines: 10,
            overlay: &mut self.overlay,
            make_overlay: &mut make,
            browse_to: &mut browse,
            oracle: None,
        };
        crate::actions::apply_core(&mut ctx, action, false)
    }
}
