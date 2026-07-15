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
//!   already uses. A true-inverse-video world's treatment isn't a plain
//!   color at all (it's a per-pixel invert), so for those worlds this tier
//!   checks the STRUCTURAL fact (the declared treatment really is `Invert`)
//!   instead of a color distance — the real-pixel proof that the renderer
//!   actually HONORS that structural fact is tier (b).
//! - **(b) REAL PIXELS, capability-driven sampling.** Every world carrying
//!   ANY non-default `RenderCaps` (today exactly Wagtail — the sampling rule
//!   is capability-driven, so a FUTURE deviant world automatically joins
//!   this tier with zero edits here) plus ONE default-caps control world,
//!   rendered for real through the pixel-diff helper (`pixeldiff.rs`) and
//!   diffed at the pixel level. This is the tier that would have caught the
//!   original bug: tier (a) alone would have happily asserted Wagtail's
//!   `HighlightTreatment::Invert` was the DECLARED contract while the
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
            match th.render_caps.highlight_treatment(band) {
                // Structural half of the contract; the real-pixel proof that
                // the renderer actually honors it lives in tier (b) below.
                theme::HighlightTreatment::Invert => {}
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
            match th.render_caps.highlight_treatment(th.selection) {
                theme::HighlightTreatment::Invert => {}
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
            theme::CaretBlockStyle::Normal => {
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
