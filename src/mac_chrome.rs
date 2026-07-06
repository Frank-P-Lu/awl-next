//! Native macOS chrome for the two MENU items whose macOS convention is a
//! real AppKit panel rather than an in-app overlay: File ▸ "Open…" (the
//! standard `NSOpenPanel` file picker) and About (the standard
//! `NSApplication` About window). Both live ONLY here, behind
//! `cfg(target_os = "macos")` — every other platform keeps the existing
//! in-app behavior (the `Action::OpenBrowse` overlay / the `about.rs` card),
//! so this module is the single place the objc2/AppKit surface is touched.
//!
//! **Main-thread law:** every function here MUST be called from the process
//! main thread (`MainThreadMarker::new()` returns `None` otherwise and the
//! call becomes a calm no-op). Both call sites satisfy this — a menu event is
//! posted into winit's `user_event`, which runs on the winit/main thread
//! (`App::handle_menu_event`), and `Action::About` is intercepted in
//! `App::apply`, also main-thread.
//!
//! **LIVE-ONLY:** none of this is reachable from the headless capture harness
//! (a real NSMenu click / NSOpenPanel modal / NSAboutPanel is AppKit chrome
//! the harness cannot drive), so nothing here is unit-tested — it is
//! structural-by-construction and flagged for human confirmation.
#![cfg(target_os = "macos")]

use std::path::PathBuf;

use objc2::{AnyThread, MainThreadMarker};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion,
    NSAboutPanelOptionCredits, NSApplication, NSBitmapFormat, NSBitmapImageRep,
    NSCompositingOperation, NSDeviceRGBColorSpace, NSFontWeightRegular, NSGraphicsContext, NSImage,
    NSImageSymbolConfiguration, NSImageSymbolScale, NSModalResponseOK, NSOpenPanel,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSInteger, NSPoint, NSRect, NSSize, NSString};

/// Run the standard macOS OPEN panel (files only, single selection) modally
/// and return the chosen path, or `None` on Cancel / off-main-thread. The
/// caller feeds the result into the SAME `App::load_path` an in-app open uses.
///
/// This is the macOS-only replacement for File ▸ "Open…" routing through
/// `Action::OpenBrowse`: the native file picker is the platform convention and
/// dodges the in-app-overlay repaint path entirely.
pub fn pick_file_to_open() -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    // Application-modal: blocks here until the user closes the panel. We are on
    // the main thread (see the module doc), which is where `runModal` must run.
    let response = panel.runModal();
    if response != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    let path = url.path()?;
    Some(PathBuf::from(path.to_string()))
}

/// Show the standard macOS About window (`orderFrontStandardAboutPanel…`),
/// populated with the app NAME, VERSION, and a short credits line via the
/// options dictionary. The macOS-only replacement for the in-app About card
/// (`about.rs`) — the native panel is the platform convention.
///
/// NOTE: the panel's ICON comes from the `.app` bundle's `CFBundleIconFile`,
/// which does not exist yet for the bare CLI binary — so the icon stays a
/// generic placeholder until the bundle chore lands. Name/version/credits
/// populate fine from the options dict regardless.
pub fn show_about_panel() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);

    let name = NSString::from_str("Awl");
    let version = NSString::from_str(env!("CARGO_PKG_VERSION"));
    // The Credits key expects an NSAttributedString (it renders in the panel's
    // info area); a plain NSString would be the wrong type there.
    let credits = NSAttributedString::from_nsstring(&NSString::from_str(
        "A calm, opinionated plain-text editor for prose and light code.",
    ));

    // SAFETY: these are AppKit's own `&'static NSString` option keys — reading
    // them is a plain static-ref load; they are immutable, never data-raced.
    let keys: [&NSString; 3] = unsafe {
        [
            NSAboutPanelOptionApplicationName,
            NSAboutPanelOptionApplicationVersion,
            NSAboutPanelOptionCredits,
        ]
    };
    let values: [&AnyObject; 3] = [name.as_ref(), version.as_ref(), credits.as_ref()];
    let options: Retained<NSDictionary<NSString, AnyObject>> =
        NSDictionary::from_slices(&keys, &values);

    // SAFETY: the options dictionary holds the exact key/value types the
    // About panel expects (name/version = NSString, credits = NSAttributedString).
    unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
}

/// The square pixel edge of a rasterized menu icon. 36px = an ~18pt menu-item
/// slot at 2x (retina), so the glyph stays crisp when AppKit scales it down.
const ICON_PX: usize = 36;
/// SF-Symbol point size fed to the symbol configuration — chosen to fill the
/// [`ICON_PX`] canvas at a comfortable menu-item weight (the actual on-canvas
/// fit is then aspect-normalized in [`render_symbol_rgba`], so this only sets
/// the symbol's rendered stroke proportions, not its final pixel extent).
const ICON_SYMBOL_POINT_SIZE: f64 = 15.0;
/// Fraction of the canvas the glyph is drawn at, leaving a small transparent
/// margin so adjacent menu-item text/edges don't crowd it.
const ICON_FILL_FRACTION: f64 = 0.86;
/// Flat mid-gray the glyph is recolored to (see [`render_symbol_rgba`]) —
/// legible in BOTH light and dark menu-bar appearances without a "template
/// image" (muda's `Icon` has no template-image mode), matching the taste call
/// the procedural fallback in `menu_icons.rs` already made.
const ICON_GRAY: u8 = 140;

/// Rasterize a named SF Symbol to a straight-alpha RGBA buffer (a square
/// [`ICON_PX`]×[`ICON_PX`] image), recolored to a flat mid-gray, for a
/// `muda::IconMenuItem`. Returns `(rgba, width, height)` on success, or `None`
/// off the main thread / if any AppKit step fails — the caller
/// (`menu_icons::icon_for`) then falls back to its procedural glyph, so a
/// missing symbol never yields a missing menu item.
///
/// This is the SF-Symbol half of the "real macOS look" for the small menu-icon
/// set; the bytes are validated by `menu_icons::safe_icon` (the crash-class
/// guard) before ever reaching `muda::Icon::from_rgba`.
pub fn render_symbol_rgba(symbol: &str) -> Option<(Vec<u8>, u32, u32)> {
    // Main-thread gate: NSImage rasterization is AppKit UI work. Off the main
    // thread (e.g. a `cargo test` worker) this returns `None` and the caller
    // uses its procedural fallback — so `menu_icons::icon_for` stays total.
    let _mtm = MainThreadMarker::new()?;

    let name = NSString::from_str(symbol);
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)?;

    // A regular-weight, medium-scale configuration at a menu-appropriate point
    // size, so the glyph's stroke proportions match a real menu item.
    // SAFETY: reading the AppKit `&'static NSFontWeight` weight constant is a
    // plain static load (immutable, never data-raced), like the About keys above.
    let weight = unsafe { NSFontWeightRegular };
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
        ICON_SYMBOL_POINT_SIZE,
        weight,
        NSImageSymbolScale::Medium,
    );
    let image = image.imageWithSymbolConfiguration(&config).unwrap_or(image);

    let px = ICON_PX as i32;
    let bytes_per_row = ICON_PX * 4;

    // A 32-bit straight-alpha RGBA backing store the framework owns (null planes
    // => it allocates). Straight (non-premultiplied) alpha matches what
    // `muda::Icon::from_rgba` expects.
    // SAFETY: standard NSBitmapImageRep designated initializer; the width/height/
    // bps/spp/bytesPerRow/bitsPerPixel are internally consistent (8bps × 4spp =
    // 32bpp, bytesPerRow = width×4), and `NSDeviceRGBColorSpace` is a valid
    // `&'static NSColorSpaceName`.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            px as NSInteger,
            px as NSInteger,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::AlphaNonpremultiplied,
            bytes_per_row as NSInteger,
            32,
        )
    }?;

    // The backing store is uninitialized; zero it to fully-transparent so the
    // margin around the drawn glyph reads as clear, not garbage.
    // SAFETY: `bytesPerRow × height` bytes are owned by the rep we just built.
    let data_ptr = rep.bitmapData();
    if data_ptr.is_null() {
        return None;
    }
    let stride = rep.bytesPerRow() as usize;
    unsafe { std::ptr::write_bytes(data_ptr, 0, stride * ICON_PX) };

    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));

    // Aspect-fit the (possibly non-square) symbol into the canvas, centered,
    // at ICON_FILL_FRACTION of the edge.
    let size = image.size();
    let (iw, ih) = (size.width, size.height);
    let canvas = ICON_PX as f64;
    let (dw, dh) = if iw > 0.0 && ih > 0.0 {
        let s = (canvas * ICON_FILL_FRACTION / iw).min(canvas * ICON_FILL_FRACTION / ih);
        (iw * s, ih * s)
    } else {
        (canvas * ICON_FILL_FRACTION, canvas * ICON_FILL_FRACTION)
    };
    let dest = NSRect::new(
        NSPoint::new((canvas - dw) / 2.0, (canvas - dh) / 2.0),
        NSSize::new(dw, dh),
    );
    image.drawInRect_fromRect_operation_fraction(
        dest,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        NSCompositingOperation::SourceOver,
        1.0,
    );
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();

    // Extract the drawn pixels, recoloring every covered pixel to flat gray while
    // preserving its coverage alpha (SF Symbols draw as black; the gray is what
    // reads in both menu appearances — see `ICON_GRAY`).
    let mut out = vec![0u8; ICON_PX * ICON_PX * 4];
    // SAFETY: `data_ptr` addresses `stride × ICON_PX` valid bytes (the rep's
    // backing store, drawn into above); we read only within row `y`'s first
    // `ICON_PX × 4` bytes.
    unsafe {
        for y in 0..ICON_PX {
            let row = data_ptr.add(y * stride);
            for x in 0..ICON_PX {
                let alpha = *row.add(x * 4 + 3);
                let o = (y * ICON_PX + x) * 4;
                out[o] = ICON_GRAY;
                out[o + 1] = ICON_GRAY;
                out[o + 2] = ICON_GRAY;
                out[o + 3] = alpha;
            }
        }
    }
    Some((out, ICON_PX as u32, ICON_PX as u32))
}
