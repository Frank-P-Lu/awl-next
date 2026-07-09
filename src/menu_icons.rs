//! src/menu_icons.rs — a SAFE icon-construction guard, plus a small in-house
//! procedurally-drawn monochrome icon set for a few File + View menu items.
//!
//! **The safety guard is the whole point of this file existing separately.**
//! The crash this round root-caused (see `menu.rs`'s module doc) manifested,
//! in one repro, as exactly this class of bug: `muda`'s macOS backend builds
//! an `NSImage` from an `Icon` by re-encoding it to PNG
//! (`PlatformIcon::to_png`), and `png::Encoder::new(.., width, ..)` REJECTS a
//! zero-width image with `FormatError::ZeroWidth` — which muda's own
//! `to_png()` then `.unwrap()`s, turning a bad icon into a hard process
//! abort. [`safe_icon`] is the ONE door this module's icons (and any future
//! ones) are built through: it validates `width > 0 && height > 0` and that
//! the buffer is EXACTLY `width * height * 4` bytes BEFORE ever handing the
//! data to `muda::Icon::from_rgba` (which itself only checks the buffer-length
//! invariant, not the zero-dimension one — confirmed by reading
//! `muda::icon::RgbaIcon::from_rgba`), and returns `None` instead of ever
//! calling `.unwrap()` on the fallible path. A caller that ignores `None`
//! simply gets a label-only menu item (`menu.rs`'s `to_menu_item` falls back
//! to a plain `MenuItem`) — never a panic.
//!
//! **The icons themselves (TASTE CALL, logged):** Apple's own stock apps keep
//! menus text-mostly — icons are the exception, not the rule — so this is a
//! deliberately SMALL, minimal set: File → New note + Save; View → Switch theme.
//! Each renders as a real macOS **SF Symbol** (the TextEdit/Zed look) via
//! `mac_chrome::render_symbol_rgba` — [`symbol_for`] names the symbol per id
//! (`square.and.pencil` / `square.and.arrow.down` / `paintpalette`). The symbol is
//! rasterized to a straight-alpha RGBA bitmap and recolored to a flat mid-gray
//! (`muda::Icon` itself has no "template image" constructor — this module's own
//! bytes are ALWAYS flat gray regardless of appearance). Reading the correct
//! ADAPTIVE tint in both appearances + the correct on-highlight tint is instead
//! a SEPARATE, later step: `mac_chrome::mark_menu_icons_as_templates` walks the
//! real installed `NSMenu`/`NSMenuItem` tree (after `crate::menu::install`) and
//! sets each item's `NSImage.isTemplate = YES` directly via objc2 — a template
//! image discards its own pixel COLOR and lets AppKit repaint it from the
//! current label ink, so this module's gray is just the harmless PRE-template
//! bytes, never the final on-screen color. If SF-Symbol rendering is
//! unavailable (off the main thread — a `cargo test` worker — or any AppKit
//! step failing), [`icon_for`] falls back to the pre-SF-Symbol PROCEDURAL glyph
//! ([`draw_for`]): plain pixel math (rectangles/circles/strokes over a
//! transparent canvas), the same flat gray, no font or embedded PNG — also
//! marked template by the same later walk. So an iconed id always resolves
//! SOMETHING, and the two enumerations ([`symbol_for`] / [`draw_for`]) stay in
//! lockstep.
//! **LIVE-ONLY (needs human confirmation):** the actual SF-Symbol glyphs
//! appearing at menu scale in a real NSMenu, correctly tinted in both
//! appearances and under a highlighted row.
#![cfg(target_os = "macos")]

use muda::Icon;

/// Flat mid-gray fill for every icon glyph (0..255 per channel, full alpha on
/// drawn pixels) — legible without a "template image" in either OS appearance
/// (see the module doc's taste-call note).
const ICON_GRAY: [u8; 4] = [140, 140, 140, 255];

/// THE crash-safety guard (see the module doc): validate a raw RGBA buffer's
/// shape BEFORE ever handing it to `muda::Icon::from_rgba`, and never
/// `.unwrap()` the fallible construction. Returns `None` — a silent,
/// icon-less fallback, never a panic — on a zero dimension or a
/// length/dimension mismatch.
pub fn safe_icon(rgba: Vec<u8>, width: u32, height: u32) -> Option<Icon> {
    if width == 0 || height == 0 {
        return None;
    }
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return None;
    }
    Icon::from_rgba(rgba, width, height).ok()
}

/// A tiny square RGBA canvas, transparent by default — the shared drawing
/// surface every glyph below fills with basic pixel math (no font, no
/// external asset).
struct Canvas {
    size: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(size: u32) -> Self {
        Self { size, px: vec![0u8; size as usize * size as usize * 4] }
    }

    fn set(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
            return;
        }
        let i = (y as usize * self.size as usize + x as usize) * 4;
        self.px[i..i + 4].copy_from_slice(&color);
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.set(x, y, color);
            }
        }
    }

    /// A rectangular OUTLINE `thickness` px wide.
    fn stroke_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thickness: i32, color: [u8; 4]) {
        self.fill_rect(x0, y0, x1, y0 + thickness, color); // top
        self.fill_rect(x0, y1 - thickness, x1, y1, color); // bottom
        self.fill_rect(x0, y0, x0 + thickness, y1, color); // left
        self.fill_rect(x1 - thickness, y0, x1, y1, color); // right
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: [u8; 4]) {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r {
                    self.set(cx + x, cy + y, color);
                }
            }
        }
    }

    /// A circular RING between `r_inner` and `r_outer` (inclusive/exclusive
    /// respectively) — a target/ring glyph without a separate stroke-width path.
    fn stroke_circle(&mut self, cx: i32, cy: i32, r_outer: i32, r_inner: i32, color: [u8; 4]) {
        for y in -r_outer..=r_outer {
            for x in -r_outer..=r_outer {
                let d2 = x * x + y * y;
                if d2 <= r_outer * r_outer && d2 >= r_inner * r_inner {
                    self.set(cx + x, cy + y, color);
                }
            }
        }
    }

    fn into_rgba(self) -> (Vec<u8>, u32, u32) {
        (self.px, self.size, self.size)
    }
}

const SIZE: i32 = 32;

/// File → "New note": a plus sign (the universal "new" glyph).
fn draw_new_note() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    let mid = SIZE / 2;
    let arm = 10; // half-length of each bar
    let thick = 4; // half-thickness
    c.fill_rect(mid - arm, mid - thick / 2, mid + arm, mid + thick / 2, ICON_GRAY);
    c.fill_rect(mid - thick / 2, mid - arm, mid + thick / 2, mid + arm, ICON_GRAY);
    c.into_rgba()
}

/// File → "Save": a floppy-disk silhouette (outer square + a smaller inset
/// "label" rectangle in the lower half — the classic simplified glyph).
fn draw_save() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    c.stroke_rect(5, 5, SIZE - 5, SIZE - 5, 3, ICON_GRAY);
    c.fill_rect(10, 17, SIZE - 10, SIZE - 9, ICON_GRAY);
    c.fill_rect(10, 5, SIZE - 14, 11, ICON_GRAY); // the corner notch's shutter tab
    c.into_rgba()
}

/// File → "Browse files…" (Open…): a folder silhouette (body + a raised tab).
fn draw_open() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    c.fill_rect(5, 8, 15, 12, ICON_GRAY); // the raised tab
    c.stroke_rect(5, 11, SIZE - 5, SIZE - 6, 3, ICON_GRAY); // the folder body
    c.into_rgba()
}

/// File → "Switch project…": two offset rectangle outlines (a stack you switch
/// between — the "swap the active project" affordance).
fn draw_switch_project() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    c.stroke_rect(6, 6, SIZE - 10, SIZE - 10, 3, ICON_GRAY);
    c.stroke_rect(10, 10, SIZE - 6, SIZE - 6, 3, ICON_GRAY);
    c.into_rgba()
}

/// File → "Finish file": a checkmark inside a ring ("done with this file",
/// the emacsclient server-edit convention). The check is two thick diagonal
/// runs of small squares (no dedicated line primitive needed).
fn draw_finish_buffer() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    let mid = SIZE / 2;
    c.stroke_circle(mid, mid, 13, 10, ICON_GRAY);
    for t in 0..6 {
        // short down-right stroke
        c.fill_rect(mid - 6 + t, mid + t, mid - 3 + t, mid + 3 + t, ICON_GRAY);
    }
    for t in 0..9 {
        // long up-right stroke
        c.fill_rect(mid - 1 + t, mid + 5 - t, mid + 2 + t, mid + 8 - t, ICON_GRAY);
    }
    c.into_rgba()
}

/// View → "Switch theme…": a filled circle (a plain "swatch" — no per-world
/// tint; see the module doc's taste-call note).
fn draw_switch_theme() -> (Vec<u8>, u32, u32) {
    let mut c = Canvas::new(SIZE as u32);
    let mid = SIZE / 2;
    c.fill_circle(mid, mid, 11, ICON_GRAY);
    c.into_rgba()
}

/// The SF Symbol NAME each iconed menu id renders as — the real macOS look
/// (the TextEdit/Zed convention). `None` for every id NOT in the small,
/// deliberately short set (`menu.rs`'s `to_menu_item` then falls back to a
/// plain, label-only `MenuItem`). This is also the enumeration the procedural
/// fallback ([`draw_for`]) mirrors id-for-id, so the two can't drift.
pub(crate) fn symbol_for(id: &str) -> Option<&'static str> {
    match id {
        "awl.new_note" => Some("square.and.pencil"),        // the compose / new-note glyph
        "awl.open" => Some("folder"),                       // the Finder-style "open a file" glyph
        "awl.switch_project" => Some("folder.badge.gearshape"), // switch the active project folder
        "awl.save" => Some("square.and.arrow.down"),        // the standard save/download glyph
        "awl.finish_buffer" => Some("checkmark.circle"),    // "done with this buffer" (server-edit)
        "awl.switch_theme" => Some("paintpalette"),         // a palette of swatches
        _ => None,
    }
}

/// The PROCEDURAL fallback glyph for an iconed id — the pre-SF-Symbol hand-drawn
/// set, kept as the graceful degradation when SF-Symbol rasterization is
/// unavailable (off the main thread — e.g. a `cargo test` worker — or if any
/// AppKit step fails). `None` for every id `symbol_for` also declines, so the
/// two enumerations stay in lockstep.
fn draw_for(id: &str) -> Option<(Vec<u8>, u32, u32)> {
    Some(match id {
        "awl.new_note" => draw_new_note(),
        "awl.open" => draw_open(),
        "awl.switch_project" => draw_switch_project(),
        "awl.save" => draw_save(),
        "awl.finish_buffer" => draw_finish_buffer(),
        "awl.switch_theme" => draw_switch_theme(),
        _ => return None,
    })
}

/// Resolve the SMALL, enumerated icon for a routed menu item's id — `None`
/// for every id not in the deliberately short set ([`symbol_for`]); `menu.rs`'s
/// `to_menu_item` then falls back to a plain, label-only `MenuItem`.
///
/// Tries a real SF Symbol first (`mac_chrome::render_symbol_rgba`, the macOS
/// TextEdit/Zed look) and DEGRADES to the procedural glyph ([`draw_for`]) when
/// that returns `None` (off the main thread, or any AppKit step failing) — so
/// an iconed id ALWAYS resolves something, and the roster's icon-flag law holds
/// on a test worker thread exactly as it does live. Either way the raw bytes
/// pass through [`safe_icon`] (the crash-class guard) before
/// `muda::Icon::from_rgba` ever sees them.
pub fn icon_for(id: &str) -> Option<Icon> {
    if let Some(symbol) = symbol_for(id) {
        if let Some((rgba, w, h)) = crate::mac_chrome::render_symbol_rgba(symbol) {
            if let Some(icon) = safe_icon(rgba, w, h) {
                return Some(icon);
            }
        }
    }
    let (rgba, w, h) = draw_for(id)?;
    safe_icon(rgba, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE LAW this whole module exists to enforce: `safe_icon` must NEVER be
    /// handed (or itself construct) a zero-width/zero-height image, and must
    /// reject a length/dimension mismatch too — both `None`, never a panic.
    #[test]
    fn safe_icon_rejects_zero_dimensions_and_length_mismatch() {
        assert!(safe_icon(vec![], 0, 0).is_none(), "zero width/height must be rejected");
        assert!(safe_icon(vec![0; 4], 0, 1).is_none(), "zero width alone must be rejected");
        assert!(safe_icon(vec![0; 4], 1, 0).is_none(), "zero height alone must be rejected");
        assert!(
            safe_icon(vec![0; 3], 1, 1).is_none(),
            "a buffer shorter than width*height*4 must be rejected"
        );
        assert!(
            safe_icon(vec![0; 4], 1, 1).is_some(),
            "a correctly-shaped 1x1 buffer must succeed"
        );
    }

    /// Every hand-drawn glyph is itself a valid, non-degenerate RGBA image —
    /// the exact invariant `safe_icon` polices, proven for real generated data
    /// rather than only synthetic buffers above.
    #[test]
    fn every_drawn_glyph_is_a_valid_nonzero_icon() {
        for (name, f) in [
            ("new_note", draw_new_note as fn() -> (Vec<u8>, u32, u32)),
            ("open", draw_open),
            ("switch_project", draw_switch_project),
            ("save", draw_save),
            ("finish_buffer", draw_finish_buffer),
            ("switch_theme", draw_switch_theme),
        ] {
            let (rgba, w, h) = f();
            assert_eq!(rgba.len(), (w * h * 4) as usize, "{name}: buffer length must match w*h*4");
            assert!(safe_icon(rgba, w, h).is_some(), "{name}: must produce a valid Icon");
        }
    }

    /// `icon_for` resolves exactly the enumerated ids and nothing else — a
    /// stray/foreign id is a harmless `None`, never a panic.
    #[test]
    fn icon_for_resolves_only_the_enumerated_ids() {
        for id in [
            "awl.new_note",
            "awl.open",
            "awl.switch_project",
            "awl.save",
            "awl.finish_buffer",
            "awl.switch_theme",
        ] {
            assert!(icon_for(id).is_some(), "{id} should resolve an icon");
        }
        assert!(icon_for("awl.quit").is_none());
        assert!(icon_for("awl.nonexistent").is_none());
    }
}
