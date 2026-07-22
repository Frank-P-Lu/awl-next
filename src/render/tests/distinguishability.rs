//! THE OUTCOME SWEEP — the LAW ROUND's answer to the Wagtail
//! invisible-picker-row bug (six render surfaces shipped invisible across
//! three rounds because every landed test asserted the MECHANISM —
//! `instance_count == 1`, `dither() > 0.0` — never the OUTCOME; a fully
//! transparent quad satisfies every one of those assertions). This file
//! enumerates every genuinely STATEFUL render surface this codebase knows
//! about behind a NO-WILDCARD [`Surface`] match — a future surface fails to
//! compile here until it joins the sweep — and proves, per surface, that
//! its "state on" rendering is PERCEPTIBLY DIFFERENT from "state off" (or
//! from an adjacent state, e.g. row 0 selected vs row 2 selected).
//!
//! TWO TIERS, for runtime honesty (see this round's own `suite_runtime_delta`
//! note in the round report — this file is deliberately NOT doubling the
//! suite):
//!
//! - **(a) COLOR-MATH, all 16 worlds, cheap.** For a surface whose treatment
//!   reduces to a computable color (a value band, a wash tint, the caret
//!   accent), assert the CONTRACT holds by redmean color distance — the same
//!   `role_style_laws_hold_for_every_world` pattern `syntax_roles.rs`
//!   already uses. A 1-bit world's selected-row treatment is now a solid
//!   `InverseFill { band, ink }` PAIR (base_content fill + base_300 glyphs,
//!   the crisp black-on-white that replaced the old framebuffer invert), so
//!   this tier checks BOTH color distances — band-vs-ground AND ink-vs-band —
//!   the same as an ordinary value band. The real-pixel proof that the
//!   renderer actually HONORS it is tier (b) + `one_bit.rs`.
//! - **(b) REAL PIXELS, capability-driven sampling.** Every world carrying
//!   ANY non-default `RenderCaps` (today exactly Wagtail — the sampling rule
//!   is capability-driven, so a FUTURE deviant world automatically joins
//!   this tier with zero edits here) plus ONE default-caps control world,
//!   rendered for real through the pixel-diff helper (`pixeldiff.rs`) and
//!   diffed at the pixel level. This is the tier that would have caught the
//!   original bug: tier (a) alone would have happily asserted Wagtail's
//!   `HighlightTreatment` pair was the DECLARED contract while the
//!   renderer still uploaded a `[0,0,0,0]` band and called it done.

use super::super::*;
use super::pixeldiff::{self, DistinguishFloor, Region};
use super::view;

/// MEASURED redmean RGB distance — a small, deliberate duplication of
/// `syntax_roles.rs`'s own copy (the same accepted shape as
/// `srgba_u8_to_linear` living twice in this codebase; see that file's doc).
fn redmean(a: theme::Srgb, b: theme::Srgb) -> f32 {
    let rbar = (a.r as f32 + b.r as f32) * 0.5;
    let dr = a.r as f32 - b.r as f32;
    let dg = a.g as f32 - b.g as f32;
    let db = a.b as f32 - b.b as f32;
    ((2.0 + rbar / 256.0) * dr * dr
        + 4.0 * dg * dg
        + (2.0 + (255.0 - rbar) / 256.0) * db * db)
        .sqrt()
}

/// A translucent wash quad composited over an opaque ground (straight alpha,
/// u8 rounding) — what the eye actually sees, mirroring `syntax_roles.rs`'s
/// own `composite`.
fn composite(wash: theme::Srgb, ground: theme::Srgb) -> theme::Srgb {
    let a = wash.a as f32 / 255.0;
    let ch = |w: u8, g: u8| (g as f32 + (w as f32 - g as f32) * a).round() as u8;
    theme::Srgb::rgb(ch(wash.r, ground.r), ch(wash.g, ground.g), ch(wash.b, ground.b))
}

/// The AVERAGE color over `region` of a single rendered frame (clamped to
/// bounds) — used only by [`Surface::CaretVsGround`], the one surface this
/// sweep checks as a region-vs-region comparison WITHIN one frame rather
/// than a two-frame state diff (the caret is always drawn; there is no
/// "caret off" state to diff against).
fn average_color(pixels: &[[u8; 4]], width: i64, height: i64, region: Region) -> theme::Srgb {
    let x0 = region.x.max(0);
    let y0 = region.y.max(0);
    let x1 = (region.x + region.w).min(width);
    let y1 = (region.y + region.h).min(height);
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = pixels[(y * width + x) as usize];
            sum[0] += px[0] as u64;
            sum[1] += px[1] as u64;
            sum[2] += px[2] as u64;
            n += 1;
        }
    }
    assert!(n > 0, "average_color: empty region {region:?}");
    theme::Srgb::rgb((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8)
}

/// A `(Device, Queue, TextPipeline)` triple, or `None` on a GPU-less
/// machine — mirrors `one_bit.rs`'s own `headless_dqp` (the small, accepted
/// per-file duplication this codebase already carries for GPU test setup).
fn headless_dqp(w: f32, h: f32) -> Option<(wgpu::Device, wgpu::Queue, TextPipeline)> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl distinguishability-test device"),
                ..Default::default()
            })
            .await
            .ok()?;
        let cache = Cache::new(&device);
        let mut p =
            TextPipeline::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8UnormSrgb);
        p.set_size(w, h);
        Some((device, queue, p))
    })
}

/// The full roster of genuinely stateful render surfaces this law sweeps.
/// Extend this enum (and the two NO-WILDCARD matches that consume it below)
/// when a new interactive-state surface lands — the match arms fail to
/// compile until the new variant is enrolled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    PickerSelectedRow,
    MenubarOpenTitle,
    SearchMatch,
    DocumentSelection,
    CaretVsGround,
}

const SURFACES: [Surface; 5] = [
    Surface::PickerSelectedRow,
    Surface::MenubarOpenTitle,
    Surface::SearchMatch,
    Surface::DocumentSelection,
    Surface::CaretVsGround,
];

/// NO-WILDCARD enrollment index — a new `Surface` variant fails to compile
/// here until it picks a slot, mirroring `syntax_roles.rs`'s own
/// `enrolled`/`ROLES` shape.
fn enrolled(s: Surface) -> usize {
    match s {
        Surface::PickerSelectedRow => 0,
        Surface::MenubarOpenTitle => 1,
        Surface::SearchMatch => 2,
        Surface::DocumentSelection => 3,
        Surface::CaretVsGround => 4,
    }
}

#[test]
fn surface_roster_is_self_consistent() {
    for (i, s) in SURFACES.iter().enumerate() {
        assert_eq!(enrolled(*s), i, "SURFACES roster out of sync with Surface's own no-wildcard match");
    }
}

/// TIER (a): color-math contract, cheap and exhaustive over all 16 worlds.
/// The redmean floor mirrors the documented "10-12/255 too faint" note on
/// `theme::derive::SELECTED_BAND_STEPS` — comfortably above a barely-visible
/// step, comfortably below what a real value-band/wash tint actually
/// carries.
#[test]
fn interactive_states_are_visible_in_every_world_color_math() {
    const FLOOR: f32 = 15.0;
    let _g = crate::testlock::serial();

    for th in theme::THEMES.iter() {
        theme::set_active_by_name(th.name).unwrap();
        for s in SURFACES.iter() {
            check_color_math(th, *s, FLOOR);
        }
    }

    theme::set_active(theme::DEFAULT_THEME);
}

fn check_color_math(th: &theme::Theme, s: Surface, floor: f32) {
    match s {
        Surface::PickerSelectedRow => {
            let band = theme::surface_selected();
            match th.highlight_treatment(band) {
                // 1-bit world: the band is a SOLID `base_content` fill and the
                // selected row's own glyphs recolor to `base_300`. The band must
                // read against the card ground AND the recolored text must read
                // against the band (the crisp black-on-white pair that replaced
                // the framebuffer invert). Both hold trivially for pure #000/#FFF,
                // but this pins it so a future 1-bit palette can't ship a pair
                // that collapses. Real-pixel proof lives in tier (b) + one_bit.rs.
                theme::HighlightTreatment::InverseFill { band, ink } => {
                    let d_band = redmean(band, th.base_300);
                    let d_ink = redmean(ink, band);
                    assert!(
                        d_band >= floor && d_ink >= floor,
                        "{}: PickerSelectedRow InverseFill band {:?}/ink {:?} — band-vs-card \
                         {d_band:.1}, ink-vs-band {d_ink:.1} (floor {floor})",
                        th.name, band, ink
                    );
                }
                theme::HighlightTreatment::ValueBand(color) => {
                    let d = redmean(color, th.base_300);
                    assert!(
                        d >= floor,
                        "{}: PickerSelectedRow band {:?} vs card ground {:?} only {d:.1} \
                         redmean apart (floor {floor})",
                        th.name, color, th.base_300
                    );
                }
            }
        }
        Surface::MenubarOpenTitle => {
            match th.highlight_treatment(th.selection) {
                theme::HighlightTreatment::InverseFill { band, ink } => {
                    let d_band = redmean(band, th.base_100);
                    let d_ink = redmean(ink, band);
                    assert!(
                        d_band >= floor && d_ink >= floor,
                        "{}: MenubarOpenTitle InverseFill band {:?}/ink {:?} — band-vs-bar \
                         {d_band:.1}, ink-vs-band {d_ink:.1} (floor {floor})",
                        th.name, band, ink
                    );
                }
                theme::HighlightTreatment::ValueBand(color) => {
                    let d = redmean(color, th.base_100);
                    assert!(
                        d >= floor,
                        "{}: MenubarOpenTitle band {:?} vs bar ground {:?} only {d:.1} \
                         redmean apart (floor {floor})",
                        th.name, color, th.base_100
                    );
                }
            }
        }
        Surface::SearchMatch => match th.render_caps.highlight_texture {
            theme::HighlightTexture::Stipple { density, .. } => {
                assert!(
                    density > 0.0,
                    "{}: SearchMatch Stipple texture must carry nonzero dither density",
                    th.name
                );
            }
            theme::HighlightTexture::Wash => {
                let d = redmean(th.selection, th.base_100);
                assert!(
                    d >= floor,
                    "{}: SearchMatch wash {:?} vs ground {:?} only {d:.1} redmean apart \
                     (floor {floor})",
                    th.name, th.selection, th.base_100
                );
            }
        },
        Surface::DocumentSelection => match th.render_caps.selection_style {
            theme::SelectionStyle::InverseVideo => {}
            theme::SelectionStyle::Fill => {
                let composited = composite(th.selection, th.base_100);
                let d = redmean(composited, th.base_100);
                assert!(
                    d >= floor,
                    "{}: DocumentSelection composited {:?} vs ground {:?} only {d:.1} \
                     redmean apart (floor {floor})",
                    th.name, composited, th.base_100
                );
            }
        },
        Surface::CaretVsGround => match th.render_caps.caret_block_style {
            theme::CaretBlockStyle::InverseVideo => {}
            // Both paint an OPAQUE `primary` cell over the ground, so visibility is
            // the same primary-vs-ground redmean check. Filled additionally re-inks
            // the covered GLYPH in `primary_content` — but that only affects the
            // letter inside the cell, never the cell's own contrast with the page,
            // so the block's findability is still exactly this measurement.
            theme::CaretBlockStyle::Normal | theme::CaretBlockStyle::Filled => {
                let d = redmean(th.primary, th.base_100);
                assert!(
                    d >= floor,
                    "{}: CaretVsGround caret accent {:?} vs ground {:?} only {d:.1} redmean \
                     apart (floor {floor})",
                    th.name, th.primary, th.base_100
                );
            }
        },
    }
}

/// WCAG relative-contrast ratio between two opaque colors, gamma-correct
/// Rec.709 (the same recipe `theme::derive::contrast_ratio` uses at runtime and
/// `syntax_roles.rs` uses for its role floors).
fn wcag_contrast(a: theme::Srgb, b: theme::Srgb) -> f32 {
    fn rel_lum(c: theme::Srgb) -> f32 {
        fn lin(u: u8) -> f32 {
            let s = u as f32 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    let (la, lb) = (rel_lum(a), rel_lum(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// LAW (born from the Bombora-under-Bars taste-gate defect): the selected
/// picker row's TEXT must clear a 3:1 contrast against its own selected-row
/// value band on EVERY world. The band under [`theme::ListStyle::Bars`] is the
/// world's `effective_overlay_selrow_band` (identical to the Pane band — Bars
/// only drops the pane and widens the bar, not the fill VALUE), so a pass here
/// covers both list styles. The exhibit: Bombora rendered light ink
/// (236,232,242) on a mid sage band (132,152,144) = 2.53:1, washing out; the fix
/// is `theme::selected_row_ink`, the ONE derive owner that flips the row's ink
/// to the reading pole when `base_content` fails. NO-WILDCARD over
/// `HighlightTreatment` — a new treatment variant fails to compile here until it
/// declares which pair the row draws — AND over every world in `THEMES`.
#[test]
fn selected_row_text_clears_contrast_floor_on_every_world() {
    const FLOOR: f32 = 3.0;
    let _g = crate::testlock::serial();

    for th in theme::THEMES.iter() {
        theme::set_active_by_name(th.name).unwrap();
        let band = crate::render::effective_overlay_selrow_band();
        // The (band fill, selected-row ink) pair the renderer actually draws,
        // resolved through the SAME owners the overlay's `selected_ink` path uses.
        let (fill, ink) = match th.highlight_treatment(band) {
            theme::HighlightTreatment::ValueBand(color) => (color, theme::selected_row_ink(color)),
            theme::HighlightTreatment::InverseFill { band, ink } => (band, ink),
        };
        let c = wcag_contrast(fill, ink);
        assert!(
            c >= FLOOR,
            "{}: selected-row ink {:?} on band {:?} = {c:.2}:1 (floor {FLOOR}:1) — the row \
             text washes into its own selection fill",
            th.name, ink, fill
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
}

/// LAW (born from the Potoroo taste-gate defect — the Wagtail invisible-row
/// class, SECONDARY edition): the selected picker row's DIM right-column hint
/// (key chord / last-edited time / git tag) must ALSO clear a 3:1 contrast
/// against its own selected-row value band on EVERY world. The primary-ink flip
/// ([`selected_row_text_clears_contrast_floor_on_every_world`]) landed but the
/// secondary column kept riding `muted` unconditionally — on Potoroo's saturated
/// gold band the muted hints washed to an 8.8 luminance delta (invisible), while
/// the unselected rows read at 89.9. The fix is [`theme::selected_row_secondary_ink`],
/// the ONE derive owner that flips the hint to the reading pole when `muted`
/// fails. NO-WILDCARD over `HighlightTreatment` — a new treatment variant fails
/// to compile here until it declares which ink the hint draws — AND over every
/// world in `THEMES`.
#[test]
fn selected_row_secondary_clears_contrast_floor_on_every_world() {
    const FLOOR: f32 = 3.0;
    let _g = crate::testlock::serial();

    for th in theme::THEMES.iter() {
        theme::set_active_by_name(th.name).unwrap();
        let band = crate::render::effective_overlay_selrow_band();
        // The (band fill, SECONDARY hint ink) pair the renderer actually draws for
        // the selected row, resolved through the SAME owners `shape_overlay_right`
        // uses: `muted` unless the band washes it out, then the reading pole.
        let (fill, ink) = match th.highlight_treatment(band) {
            theme::HighlightTreatment::ValueBand(color) => {
                (color, theme::selected_row_secondary_ink(color))
            }
            theme::HighlightTreatment::InverseFill { band, ink } => (band, ink),
        };
        let c = wcag_contrast(fill, ink);
        assert!(
            c >= FLOOR,
            "{}: selected-row SECONDARY hint ink {:?} on band {:?} = {c:.2}:1 (floor \
             {FLOOR}:1) — the dim right-column chord washes into its own selection fill",
            th.name, ink, fill
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
}

/// LAW (born from the SLANT-ON-BARS regression — the Wagtail invisible-row class,
/// SECONDARY-under-a-HUG-PLATE edition): the selected picker row's DIM
/// right-column chord must SURVIVE — stay visible — even when the world is a Bars
/// world AND the wild-menu slant is on. The prior secondary flip
/// ([`theme::selected_row_secondary_ink`]) contrasts the chord against the
/// selected-row BAND, which is correct ONLY when the chord sits ON the band (Pane,
/// FULL-WIDTH bars). Under a HUGGING plate ([`theme::BarExtent::HugLabel`], the
/// poster worlds' hybrid) the bare right chord rides the GROUND, not the plate, so
/// contrasting the band drove it INTO the ground: Firetail's selected `⌘O` washed
/// to a 13.5-maxlum background band while the unselected rows read 135. The color
/// law above checked the chord against the band it never touched, so it stayed
/// green while the pixel vanished — this is the OUTCOME proof over REAL pixels.
///
/// The assertion is SURVIVAL-under-selection: render the same chord row selected
/// and unselected, and require the selected chord's peak contrast against its own
/// local ground to stay within a fraction of the unselected chord's — a flip that
/// erases it drops the selected peak to ~0 while the unselected stays bright.
/// Swept across every world whose `list_style` is `Bars`, WITH the slant probe on
/// (`px_per_row = 12.0`, the gallery's stair), so slant × selection × Bars is one
/// cell per world.
#[test]
fn selected_row_secondary_survives_slant_on_bars_worlds() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping selected_row_secondary_survives_slant_on_bars_worlds: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let w = 1200u32;
    let h = 800u32;

    // The peak luminance deviation from the region's own background (its median
    // luminance) — a glyph is a spike above/below the ground; a chord washed into
    // the ground leaves ~0. Row-major RGBA over `[cx0,cx1) x [ry0,ry1)`.
    fn chord_peak(buf: &[[u8; 4]], width: i64, cx0: i64, cx1: i64, ry0: i64, ry1: i64) -> f32 {
        let lum = |p: [u8; 4]| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        let mut lums: Vec<f32> = Vec::new();
        for y in ry0..ry1 {
            for x in cx0..cx1 {
                lums.push(lum(buf[(y * width + x) as usize]));
            }
        }
        if lums.is_empty() {
            return 0.0;
        }
        let mut sorted = lums.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let ground = sorted[sorted.len() / 2];
        lums.iter().map(|l| (l - ground).abs()).fold(0.0_f32, f32::max)
    }

    let bars_worlds: Vec<&theme::Theme> = theme::THEMES
        .iter()
        .filter(|t| matches!(t.render_caps.list_style, theme::ListStyle::Bars { .. }))
        .collect();
    assert!(
        !bars_worlds.is_empty(),
        "expected at least one Bars world (Firetail/Galah/Magpie/Mangrove)"
    );

    // The gallery's stair — the wild-menu slant that exposed the bug.
    crate::render::set_slant_test_override(Some(crate::render::SlantProbe {
        px_per_row: 12.0,
        italic: false,
    }));

    for th in &bars_worlds {
        theme::set_active_by_name(th.name).unwrap();
        p.sync_theme();

        // A flat picker whose FIRST row carries a chord (the exact `shape_overlay_right`
        // surface). Render it selected, then with the selection moved off it.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Go to file".into(), "Switch project".into(), "Recent".into()];
        v.overlay_bindings = vec!["\u{2318}O".into(), String::new(), String::new()];
        v.overlay_selected = 0;
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        let [cx, _cy, cw, _ch] = p.overlay_card_rect().expect("the Bars picker must have a card");
        let region = overlay_row_region(&p, 0);
        // The right-column chord band: the rightmost slab of the card text column,
        // where the bare `⌘O` right-aligns — over the GROUND under a HugLabel plate.
        let cx1 = (cx + cw - 6.0) as i64;
        let cx0 = (cx + cw - 170.0) as i64;
        let ry0 = region.y;
        let ry1 = region.y + region.h;
        let sel = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
        let sel_peak = chord_peak(&sel, w as i64, cx0, cx1, ry0, ry1);

        v.overlay_selected = 1;
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        let unsel = pixeldiff::render_frame(&mut p, &device, &queue, w, h);
        let unsel_peak = chord_peak(&unsel, w as i64, cx0, cx1, ry0, ry1);

        // Sanity: the chord is genuinely drawn when unselected (the surface is real).
        assert!(
            unsel_peak >= 20.0,
            "{}: the unselected row-0 chord must be visible (peak {unsel_peak:.1} \
             < 20) — test setup did not draw a chord",
            th.name
        );
        // SURVIVAL: selecting row 0 must not wash its chord away. A flip that drives
        // it into the ground collapses the selected peak toward 0.
        assert!(
            sel_peak >= 0.5 * unsel_peak,
            "{}: selected-row chord peak {sel_peak:.1} < half the unselected {unsel_peak:.1} \
             — the selected row's secondary hint washed out under slant-on-bars \
             (the invisible-selected-chord regression)",
            th.name
        );
    }

    crate::render::set_slant_test_override(None);
    theme::set_active(theme::DEFAULT_THEME);
}

/// TIER (b): REAL PIXELS, capability-driven sampling — every world carrying
/// any non-default `RenderCaps` (today exactly Wagtail) plus one
/// default-caps control world (Tawny, or whichever sorts first). This is the
/// tier that reproduces the round's own motivating bug shape: a mechanism
/// (tier a) can be perfectly correct on paper while the renderer still
/// uploads nothing.
#[test]
fn interactive_states_are_visible_in_every_world_real_pixels() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping interactive_states_are_visible_in_every_world_real_pixels: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();

    let control = theme::THEMES
        .iter()
        .find(|t| t.render_caps == theme::RenderCaps::DEFAULT)
        .expect("at least one default-caps control world must exist");
    let mut worlds: Vec<&theme::Theme> =
        theme::THEMES.iter().filter(|t| t.render_caps != theme::RenderCaps::DEFAULT).collect();
    assert!(!worlds.is_empty(), "expected at least one capability-deviant world (Wagtail)");
    if !worlds.iter().any(|t| t.name == control.name) {
        worlds.push(control);
    }

    for th in worlds {
        theme::set_active_by_name(th.name).unwrap();
        p.sync_theme();
        for s in SURFACES.iter() {
            check_real_pixels(&mut p, &device, &queue, *s, th.name);
        }
    }

    theme::set_active(theme::DEFAULT_THEME);
}

/// ITEM 35 — OVERLAY TEXT SITS ON A SURFACE (the Mangrove Bars "floating
/// commands" defect). Under the HugLabel poster HYBRID the label plate hugs the
/// LABEL alone, so the right-aligned SHORTCUT chord floated BARE over the blurred
/// backdrop. The fix lays a per-row CHORD PLATE. This is the OUTCOME proof over
/// REAL pixels, swept across EVERY Bars world: rendering the SAME unselected row
/// WITH vs WITHOUT a chord, the chord-slab's BACKGROUND (median luminance, robust
/// to the minority glyph pixels) must CHANGE — a chord now brings a plate, where
/// before it brought only bare glyphs over the unchanged backdrop.
///
/// Non-vacuous by construction: the only variable is whether row 0 carries a
/// chord. Before the chord plate, both frames showed the same backdrop behind
/// that slab (only a few glyph pixels differed, which the MEDIAN discards), so the
/// median delta was ~0; the plate is exactly what moves it.
#[test]
fn overlay_chord_sits_on_a_plate_on_every_bars_world() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!("skipping overlay_chord_sits_on_a_plate_on_every_bars_world: no wgpu adapter");
        return;
    };
    let _g = crate::testlock::serial();
    let w = 1200u32;
    let h = 800u32;

    let bars_worlds: Vec<&theme::Theme> = theme::THEMES
        .iter()
        .filter(|t| matches!(t.render_caps.list_style, theme::ListStyle::Bars { .. }))
        .collect();
    assert!(
        !bars_worlds.is_empty(),
        "expected at least one Bars world (Firetail/Galah/Magpie/Mangrove)"
    );

    for th in &bars_worlds {
        theme::set_active_by_name(th.name).unwrap();
        p.sync_theme();

        // A flat palette; row 0 is UNSELECTED (select row 1) so the chord rides the
        // QUIET unselected plate — the exact bare-chord surface the defect showed.
        let mut v = view("hello world\n", 0, 0);
        v.overlay_active = true;
        v.overlay_items = vec!["Go to file".into(), "Switch project".into(), "Recent".into()];
        v.overlay_selected = 1;

        // WITH a chord on row 0.
        v.overlay_bindings = vec!["\u{2318}O".into(), String::new(), String::new()];
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        let [cx, _cy, cw, _ch] = p.overlay_card_rect().expect("the Bars picker must have a card");
        let region = overlay_row_region(&p, 0);
        // The right-column chord slab: the rightmost span of the card text column,
        // where `⌘O` right-aligns and (after the fix) its plate hugs it.
        let slab = Region::new(cx + cw - 100.0, region.y as f32, 92.0, region.h as f32);
        let with = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

        // WITHOUT a chord on row 0 (empty binding) — no chord, no plate.
        v.overlay_bindings = vec![String::new(), String::new(), String::new()];
        p.set_view(&v);
        p.prepare(&device, &queue, w, h).unwrap();
        let without = pixeldiff::render_frame(&mut p, &device, &queue, w, h);

        // The chord's PRESENCE changes the slab: with a plate, a LARGE fraction of
        // the slab's pixels differ (the plate is a filled rect); the bare glyphs
        // ALONE (the pre-fix state) touch only their sparse strokes. So a
        // substantial differing fraction is the plate's signature.
        let d = pixeldiff::diff_region(&with, &without, w as i64, h as i64, slab);
        let frac = d.differing_fraction();
        assert!(
            frac >= 0.35,
            "{}: only {:.1}% of the chord slab changed when a chord appeared — the \
             shortcut is floating BARE over the backdrop (bare glyphs alone), not on a \
             plate that hugs it",
            th.name,
            frac * 100.0
        );
    }

    theme::set_active(theme::DEFAULT_THEME);
}

fn overlay_row_region(p: &TextPipeline, row: usize) -> Region {
    let [card_x, card_y, card_w, _] =
        p.overlay_card_rect().expect("the overlay card must be open");
    let lh = p.overlay_lh();
    let text_top = card_y + 12.0; // pad
    // +1 header row (the query line) + the PALETTE-COMPOSITION round's header gap
    // (the divider space after the header), folded in through the SAME owner the
    // renderer uses so the sampled band tracks the shaped row.
    let row_top = text_top + lh + p.overlay_header_gap() + lh * row as f32;
    Region::new(card_x, row_top, card_w, lh)
}

fn check_real_pixels(
    p: &mut TextPipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    s: Surface,
    world: &str,
) {
    let w = 1200u32;
    let h = 800u32;
    match s {
        Surface::PickerSelectedRow => {
            let mut v = view("hello world\n", 0, 0);
            v.overlay_active = true;
            v.overlay_items = vec!["Save".into(), "Undo".into(), "Redo".into()];
            v.overlay_selected = 0;
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let region = overlay_row_region(p, 0);
            let a = pixeldiff::render_frame(p, device, queue, w, h);

            v.overlay_selected = 1;
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let b = pixeldiff::render_frame(p, device, queue, w, h);

            pixeldiff::assert_perceptibly_different(
                &a,
                &b,
                w as i64,
                h as i64,
                region,
                DistinguishFloor::DEFAULT,
                &format!("{world}: PickerSelectedRow (row 0 selected vs row 1 selected)"),
            );
        }
        Surface::MenubarOpenTitle => {
            crate::menubar::set_menu_bar_on(true);
            crate::menubar::set_open(None);
            let v = view("hello world\n", 0, 0);
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let bar_h = p.menubar_bar_h.max(1.0);
            let a = pixeldiff::render_frame(p, device, queue, w, h);

            crate::menubar::set_open(Some(0));
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let b = pixeldiff::render_frame(p, device, queue, w, h);

            let region = Region::new(0.0, 0.0, w as f32, bar_h);
            pixeldiff::assert_perceptibly_different(
                &a,
                &b,
                w as i64,
                h as i64,
                region,
                DistinguishFloor::DEFAULT,
                &format!("{world}: MenubarOpenTitle (closed vs title 0 open)"),
            );

            crate::menubar::set_open(None);
            crate::menubar::set_menu_bar_on(false);
        }
        Surface::SearchMatch => {
            let text = "alpha beta findme gamma";
            let mut v = view(text, 0, 0);
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let a = pixeldiff::render_frame(p, device, queue, w, h);

            v.search_active = true;
            v.search_query = "findme".to_string();
            v.search_matches = vec![((0, 11), (0, 17))];
            v.search_current = Some(0);
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let b = pixeldiff::render_frame(p, device, queue, w, h);

            let region = Region::new(0.0, TEXT_TOP, w as f32, LINE_HEIGHT);
            pixeldiff::assert_perceptibly_different(
                &a,
                &b,
                w as i64,
                h as i64,
                region,
                DistinguishFloor::DEFAULT,
                &format!("{world}: SearchMatch (no match vs one active match)"),
            );
        }
        Surface::DocumentSelection => {
            let text = "alpha beta gamma delta";
            let mut v = view(text, 0, 0);
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let a = pixeldiff::render_frame(p, device, queue, w, h);

            v.selection = Some(((0, 0), (0, 11)));
            p.set_view(&v);
            p.prepare(device, queue, w, h).unwrap();
            let b = pixeldiff::render_frame(p, device, queue, w, h);

            let region = Region::new(0.0, TEXT_TOP, w as f32, LINE_HEIGHT);
            pixeldiff::assert_perceptibly_different(
                &a,
                &b,
                w as i64,
                h as i64,
                region,
                DistinguishFloor::DEFAULT,
                &format!("{world}: DocumentSelection (none vs a real span)"),
            );
        }
        Surface::CaretVsGround => {
            crate::caret::set_mode(CaretMode::Block);
            // Line 0 carries the caret; line 1 is EMPTY, guaranteeing a
            // glyph-free ground sample one row below the caret's own column.
            let text = "hi\n\n";
            let v = view(text, 0, 1);
            p.set_view(&v);
            p.settle_caret();
            p.prepare(device, queue, w, h).unwrap();
            let frame = pixeldiff::render_frame(p, device, queue, w, h);

            let (cx, cy, cw, ch) = p.caret_pixel_rect();
            let inset_w = (cw * 0.5).max(1.0);
            let inset_h = (ch * 0.5).max(1.0);
            let caret_region =
                Region::new(cx + cw * 0.25, cy + ch * 0.25, inset_w, inset_h);
            let ground_region =
                Region::new(cx + cw * 0.25, cy + ch * 0.25 + LINE_HEIGHT, inset_w, inset_h);

            let caret_avg = average_color(&frame, w as i64, h as i64, caret_region);
            let ground_avg = average_color(&frame, w as i64, h as i64, ground_region);
            let d = redmean(caret_avg, ground_avg);
            assert!(
                d >= 15.0,
                "{world}: CaretVsGround caret region {caret_avg:?} vs empty-line ground \
                 {ground_avg:?} only {d:.1} redmean apart (floor 15.0)"
            );

            crate::caret::set_mode(CaretMode::Block);
        }
    }
}

/// THE THEME-PREVIEW PAGE-SURFACE SWEEP — the law born from the user's
/// "arrowing Mangrove→Magpie makes the page disappear" report (2026-07-17) and
/// widened to FULL SOURCE×DEST COVERAGE after the reopened "still missing from
/// mangrove/magpie, switching from wagtail" (2026-07-18). The theme picker's LIVE
/// preview applies only the O(1) COLOR half of a switch
/// ([`TextPipeline::sync_theme_colors`]) and DEFERS the font reshape, so a bug
/// where that retint left ANY world-varying render state (the `base_100` column
/// ground, the margins, the lava teardown, the 1-bit dither/InverseFill uniforms)
/// grounded to the SOURCE world — or blank — would show as the writing surface
/// vanishing mid-arrow while the caller keeps arrowing (the deferred reshape never
/// settles).
///
/// THE STRONGEST FORM the suite can afford: FULL SOURCE COVERAGE with WHOLE-FRAME
/// BYTE-IDENTITY. For EVERY world in [`theme::THEMES`] as the SOURCE (a
/// no-wildcard roster walk — a future world joins the source axis automatically)
/// previewed into a DESTINATION sample chosen to cover every background class
/// (lava, non-lava dark, non-lava light, one-bit — a runtime assertion pins the
/// sample really spans all four, so a future trim can't silently drop a class),
/// the color-only PREVIEW frame is BYTE-IDENTICAL to a cold synchronous load of
/// the destination ([`TextPipeline::sync_theme`]).
///
/// WHY BYTE-IDENTITY IS ASSERTABLE: on an EMPTY buffer there are no glyphs, so the
/// DEFERRED font reshape the preview skips changes zero pixels — every remaining
/// pixel is world-varying render state (page ground, margins, lava field, 1-bit
/// dither/InverseFill uniforms) that the O(1) retint alone must fully re-derive.
/// The headless lava phase is FROZEN (`lava::LAVA_FROZEN_PHASE`), so a lava
/// world's margins render identically every call — the frame is a pure
/// deterministic function of the active theme. Comparing the WHOLE framebuffer
/// (not a sampled region) means ANY source-state leak anywhere fails — including
/// WAGTAIL's 1-bit pipeline state, the source class the original two-source sweep
/// never exercised. (The full 16×16 pair matrix is byte-identical too — verified
/// during the 2026-07-18 diagnosis — but destinations are class-sampled here so
/// the standing law stays within the suite's time budget.)
///
/// NOTE this test is GREEN: the color retint is provably correct from every
/// source. The reopened vanish is therefore NOT a stale-color bug but the
/// live-only present/compositor race, addressed by arming the present bracket
/// UNCONDITIONALLY on every preview step with an event-ordered teardown that
/// holds the bracket through the deferred reshape's present (the retired
/// `preview_crossing` classification left the actual landing frame unbracketed —
/// see `app::tests::every_preview_step_brackets_and_teardown_waits_for_the_reshape_present`).
#[test]
fn theme_preview_retint_regrounds_the_page_surface_on_every_world() {
    let Some((device, queue, mut p)) = headless_dqp(1200.0, 800.0) else {
        eprintln!(
            "skipping theme_preview_retint_regrounds_the_page_surface_on_every_world: no wgpu adapter"
        );
        return;
    };
    let _g = crate::testlock::serial();
    crate::caret::set_mode(CaretMode::Block);
    crate::page::set_page_on(true);
    crate::page::set_measure(40);
    // An EMPTY buffer: no glyph ink, so the DEFERRED font reshape the preview seam
    // skips changes zero pixels — the whole frame is world-varying page/chrome
    // render state, and any of it left stale would show here.
    let v = view("", 0, 0);
    let w = 1200u32;
    let h = 800u32;

    // Render the current active world's whole frame (deterministic — see the doc).
    let frame = |p: &mut TextPipeline, dev: &wgpu::Device, q: &wgpu::Queue| -> Vec<[u8; 4]> {
        p.set_view(&v);
        p.prepare(dev, q, w, h).unwrap();
        pixeldiff::render_frame(p, dev, q, w, h)
    };
    // Report the divergence between two frames: (differing pixels, worst channel
    // delta, first differing (x,y)) — a real diagnostic, never a bare bool.
    let diff = |a: &[[u8; 4]], b: &[[u8; 4]]| -> (usize, u16, Option<(i64, i64)>) {
        let mut count = 0usize;
        let mut worst = 0u16;
        let mut first = None;
        for (i, (pa, pb)) in a.iter().zip(b.iter()).enumerate() {
            if pa != pb {
                count += 1;
                for c in 0..4 {
                    worst = worst.max((pa[c] as i16 - pb[c] as i16).unsigned_abs());
                }
                if first.is_none() {
                    first = Some((i as i64 % w as i64, i as i64 / w as i64));
                }
            }
        }
        (count, worst, first)
    };

    // The DESTINATION class sample: one world per background class. Sources are
    // the FULL roster; keeping destinations to a class sample bounds the render
    // count while covering every kind the retint must re-ground into.
    let dst_worlds: Vec<&theme::Theme> = ["Mangrove", "Currawong", "Magpie", "Wagtail"]
        .iter()
        .map(|n| theme::THEMES.iter().find(|t| t.name == *n).unwrap())
        .collect();
    // Self-verify the sample really spans every class (so a future trim can't
    // silently drop one — the exact "a source/dest-class gap went unseen" failure).
    assert!(dst_worlds.iter().any(|t| t.background.is_lava()), "sample covers a LAVA dest");
    assert!(
        dst_worlds.iter().any(|t| !t.background.is_lava() && t.dark && !t.is_one_bit()),
        "sample covers a NON-LAVA DARK dest"
    );
    assert!(
        dst_worlds.iter().any(|t| !t.background.is_lava() && !t.dark && !t.is_one_bit()),
        "sample covers a NON-LAVA LIGHT dest"
    );
    assert!(dst_worlds.iter().any(|t| t.is_one_bit()), "sample covers the ONE-BIT dest");

    // The COLD frame of each destination (a full synchronous switch) — the ground
    // truth each preview into it must reproduce byte-for-byte.
    let cold: Vec<Vec<[u8; 4]>> = dst_worlds
        .iter()
        .map(|t| {
            theme::set_active_by_name(t.name).unwrap();
            p.sync_theme();
            frame(&mut p, &device, &queue)
        })
        .collect();

    // Every world in the roster as the SOURCE (no-wildcard) — Wagtail included.
    for src in theme::THEMES.iter() {
        for (di, dst) in dst_worlds.iter().enumerate() {
            // Establish the SOURCE as the fully-rendered presented state (this is
            // exactly the frame the user is looking at before they arrow) — a
            // render, so any state only touched at draw time is genuinely present.
            theme::set_active_by_name(src.name).unwrap();
            p.sync_theme();
            let src_frame = frame(&mut p, &device, &queue);

            // PREVIEW SEAM: switch active to the destination and apply the COLOR
            // HALF ONLY — the exact state a picker arrow leaves before the deferred
            // reshape (`App::retint_theme_preview`).
            theme::set_active_by_name(dst.name).unwrap();
            p.sync_theme_colors();
            let preview = frame(&mut p, &device, &queue);

            // (1) BYTE-IDENTICAL to the destination's cold frame — the O(1) retint
            // fully re-grounded the whole surface, leaving NO source state behind.
            let (n, worst, first) = diff(&preview, &cold[di]);
            assert!(
                n == 0,
                "{src} -> {dst} PREVIEW frame diverged from the cold destination frame: \
                 {n} px differ (worst channel Δ {worst}, first at {first:?}) — the O(1) \
                 retint left world-varying render state stale on the preview seam",
                src = src.name,
                dst = dst.name,
            );
            // (2) WITNESS the comparison is non-trivial: when the two worlds have
            // visibly different grounds, their COLD frames genuinely differ, so
            // (1)'s byte-identity means the preview truly re-grounded (not that the
            // worlds happen to render the same).
            if src.name != dst.name && redmean(src.base_100, dst.base_100) > 12.0 {
                let (nc, ..) = diff(&src_frame, &cold[di]);
                assert!(
                    nc > 0,
                    "{src} -> {dst}: the source and destination cold frames are identical, \
                     so the byte-identity check above is trivially satisfied — the witness failed",
                    src = src.name,
                    dst = dst.name,
                );
            }
        }
    }

    theme::set_active(theme::DEFAULT_THEME);
    p.sync_theme();
}
