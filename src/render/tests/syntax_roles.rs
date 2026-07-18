//! The Alabaster syntax role-color laws (fg distinguishability, comment-tier
//! identity, wash whisper bounds, the amber guard), the ink-ladder/selection
//! contrast laws, and the two-tier comment classifier -- split out of the
//! former monolithic `render::tests` (2026-07 code-organization pass). See
//! `syntax_ligatures` for the ligature-feature tests.

use super::super::*;

/// MEASURED redmean RGB distance (u8 scale) — the perceptual-ish weighting the
/// role-style law thresholds were calibrated against.
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

/// A translucent wash quad composited over an opaque ground — what the eye
/// actually sees behind a washed span (straight alpha, u8 rounding).
fn composite(wash: theme::Srgb, ground: theme::Srgb) -> theme::Srgb {
    let a = wash.a as f32 / 255.0;
    let ch = |w: u8, g: u8| (g as f32 + (w as f32 - g as f32) * a).round() as u8;
    theme::Srgb::rgb(ch(wash.r, ground.r), ch(wash.g, ground.g), ch(wash.b, ground.b))
}

/// Circular hue distance in degrees.
fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// THE ROLE-STYLE LAW TEST — sweeps EVERY world in `theme::THEMES` (a future
/// world is enrolled automatically) × every syntax role, and asserts the laws
/// on the EFFECTIVE style (`role_style_for`, overrides included). LOCK-FREE:
/// `role_style_for` takes `&Theme`, never the process-global active theme.
///
/// The laws (thresholds calibrated from the measured 14-world table):
/// (a) every pair among {Definition, Constant, Str, CommentCode(=muted)} is
///     redmean ≥ 40 apart (measured floor 51.6, Tawny Def–Const);
/// (b) prose-Comment fg == `base_content` EXACTLY (comments are the prose in
///     the code — decision 2) and CommentCode fg == `muted` EXACTLY;
/// (c) the comment wash composited over `base_100` is a WHISPER: ΔL in
///     [0.03, 0.12] and redmean ≥ 35 (measured 0.063–0.11 / 51–89) — a wash
///     is structurally incapable of reading as the accent;
/// (d) dark worlds wash strings too (comment-wash vs string-wash effective
///     redmean ≥ 20, measured 28–29); light worlds carry NO string wash;
///     Definition / Constant / CommentCode are NEVER washed;
/// (e) AMBER GUARD: every derived fg tint with sat > 0.15 sits ≥ 30° of hue
///     from the world's `primary` AND at sat ≤ 0.50 (the comment tiers are
///     the existing inks — exempt by identity, never equal to primary).
///     EXEMPTION — an INK-CARET world (`Theme::ink_caret`, primary ==
///     base_content: Wagtail, Cassowary) skips the ≥30° hue gap: its caret
///     carries no accent HUE (presence is the inverting/filled block), so no
///     role can steal it. The exemption is paired, law-enforced, with a
///     non-Normal `caret_block_style` — an opaque ink-coloured block would
///     erase the letter; the sat cap + all the distinctness laps still apply;
/// (f) presence ordering is monotone per mode: Definition sits closest to
///     the full ink, then Constant, then Str;
/// (g) PERCEPTIBILITY FLOOR: every tinted role's fg (Definition, Constant,
///     Str) sits redmean ≥ 70 from `base_content` on EVERY world — a floor
///     picked from the measured 14-world table: dark `Definition` at its old
///     t=0.12/sat=0.32 measured 36.4–65.2 (Currawong screenshot-confirmed as
///     plain-looking ink, the bug this law exists to catch structurally);
///     every OTHER role/world combination already measured ≥ 76 (worst:
///     dark Constant at 76.2, Undertow). 70 sits safely below that 76.2 floor
///     (room for future re-tuning) while sitting well above the old broken
///     Definition range, so a future regression of this exact shape — a role
///     tint that clears the pairwise ≥40 law but reads as invisible against
///     the page's own ink — fails this test immediately instead of needing a
///     human screenshot to notice;
/// (h) LUMINANCE FLOOR: every tinted role's fg sits WCAG relative-luminance
///     ΔY ≥ 0.05 from `base_content` on every world — redmean ALONE is proven
///     insufficient by (g)'s own history: light `Definition` cleared redmean
///     70+ (Saltpan measured 148) while carrying almost all of that distance
///     in the BLUE channel, which Rec.709 luminance weighs at only 0.0722 (vs
///     0.7152 on green) — the eye resolves LUMINANCE first (sparse S-cones),
///     so a color can be "far" in redmean and still read as plain ink. Floor
///     picked from the retuned 14-world table (`measure_role_luminance`, an
///     ignored scratch test): worst case (post the round-2 ground-contrast
///     retune below) is light Definition/Constant at ΔY 0.056 (Gumtree); 0.05
///     sits with margin below every measured value and comfortably above the
///     old broken range (ΔY 0.027–0.042) — so a future regression of this
///     exact shape (redmean-passing, luminance-invisible) fails structurally
///     instead of needing a screenshot.
/// (i) GROUND-CONTRAST FLOOR: every tinted role's fg clears a WCAG contrast
///     RATIO of ≥ 4.5:1 against `base_100` (the page's own background) on
///     every world — the axis (h) does not cover. (h) only measures distance
///     from the INK; a fix that satisfies (h) by pushing a role's lightness
///     toward `muted` can, on a light world, push it most of the way toward
///     the pale GROUND instead — distinct-from-ink is not the same claim as
///     readable-on-page. This is exactly what happened: the round-1 light
///     retune (`T_LIGHT = [0.84, 0.90, 0.94]`) cleared (g)/(h) on every world
///     yet a live taste-gate verdict on Saltpan called the result "too hard
///     to read" — strings/constants/definitions as washed-out pastels on the
///     pale ground. Measured: Saltpan `Str` at the round-1 rungs contrasted
///     only 4.62:1 against `base_100` (Quokka worse, 3.66:1) — well under
///     body-text-grade legibility (WCAG AA normal text = 4.5:1) despite
///     clearing every prior law. 4.5:1 is the standard body-text floor (not
///     loosened for glyph-scale mono/serif — the user's own complaint was
///     about reading code prose, i.e. body text). Dark worlds were ALREADY
///     clearing this floor by a wide margin (measured 9.4–13.5:1 — a dark
///     ground is far from every usable tint) and are asserted here
///     unchanged, never retuned. Round 2's retune (`T_LIGHT = [0.76, 0.78,
///     0.80]`, `S_FG_LIGHT = 0.18`, found by `sweep_light_ladder` now
///     searching for BOTH floors (h) and (i) simultaneously) measures
///     worst-case ground contrast 4.84:1 (Quokka `Str`) while keeping (h)'s
///     worst-case ΔY at 0.056 — both floors clear with margin on every
///     light world.
#[test]
fn role_style_laws_hold_for_every_world() {
    use crate::syntax::SynKind;
    // The explicit role roster, backed by a NO-WILDCARD match: a future
    // SynKind variant fails to compile here until it is enrolled in the sweep.
    const ROLES: [SynKind; 5] = [
        SynKind::Comment,
        SynKind::CommentCode,
        SynKind::Str,
        SynKind::Constant,
        SynKind::Definition,
    ];
    fn enrolled(k: SynKind) -> usize {
        match k {
            SynKind::Comment => 0,
            SynKind::CommentCode => 1,
            SynKind::Str => 2,
            SynKind::Constant => 3,
            SynKind::Definition => 4,
        }
    }
    for (i, k) in ROLES.iter().enumerate() {
        assert_eq!(enrolled(*k), i, "ROLES roster out of sync with SynKind");
    }

    for th in theme::THEMES.iter() {
        let style = |k: SynKind| role_style_for(th, k);

        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`, Wagtail's 2026-07 rework):
        // laws (a)/(c)/(d)/(f)/(g)/(h)/(i) below all assume a continuous ink
        // ladder to derive DISTINCT tints/washes from — a 1-bit world has
        // exactly ONE ink value (pure white) and nothing else to derive a
        // second tint from, so those checks are STRUCTURALLY INAPPLICABLE
        // (declared exemption, not a weakening — there is no ladder left to
        // weaken). Replaced by the FLAT LAW this world's whole design
        // statement demands: every role's effective foreground is EXACTLY
        // `base_content` (identity, not merely "a similar grey" —
        // "comments/strings undifferentiated" is deliberate) and NO role
        // carries a wash (any non-0/255-alpha quad over pure black would
        // composite a forbidden grey). (b) still holds — it's the trivial
        // identity this exemption is built on. (e)'s "never BE the accent"
        // sub-check is skipped here too: a true 1-bit world's ink and caret
        // are NECESSARILY the same pure-white value (there is nowhere else
        // for either to live), which the general law's own exempt-by-identity
        // branch already treats as a non-issue for the comment tiers — this
        // is that same fact, total across every role, not a new gap.
        if th.is_one_bit() {
            for k in ROLES {
                let s = style(k);
                assert_eq!(
                    s.fg, th.base_content,
                    "{}: one-bit {k:?} fg must be EXACTLY base_content (flat, no per-role tint)",
                    th.name
                );
                assert!(
                    s.wash.is_none(),
                    "{}: one-bit {k:?} must carry NO wash (translucent-over-black would be a forbidden grey)",
                    th.name
                );
            }
            continue;
        }

        // (b) The two comment tiers ARE the existing inks, exactly.
        assert_eq!(style(SynKind::Comment).fg, th.base_content,
            "{}: prose comments render at FULL content ink", th.name);
        assert_eq!(style(SynKind::CommentCode).fg, th.muted,
            "{}: commented-out code stays the muted grey", th.name);

        // (a) Pairwise distinguishability of the four ink-distinct roles.
        let four = [SynKind::Definition, SynKind::Constant, SynKind::Str, SynKind::CommentCode];
        for i in 0..four.len() {
            for j in i + 1..four.len() {
                let d = redmean(style(four[i]).fg, style(four[j]).fg);
                assert!(
                    d >= 40.0,
                    "{}: {:?} vs {:?} fg redmean {d:.1} < 40 (memory test fails)",
                    th.name, four[i], four[j]
                );
            }
        }

        // (c) The comment wash: present on every world, a value whisper.
        let cw = style(SynKind::Comment).wash
            .unwrap_or_else(|| panic!("{}: every world carries the comment wash", th.name));
        let ceff = composite(cw, th.base_100);
        let dl = (ceff.to_hsl().2 - th.base_100.to_hsl().2).abs();
        assert!(
            (0.03..=0.12).contains(&dl),
            "{}: comment-wash ΔL {dl:.3} outside the whisper band [0.03, 0.12]",
            th.name
        );
        assert!(
            redmean(ceff, th.base_100) >= 35.0,
            "{}: comment wash too faint (redmean {:.1} < 35)",
            th.name, redmean(ceff, th.base_100)
        );

        // (d) Strings: washed on dark worlds (distinct from the comment wash),
        // fg-tint-only on light; Definition/Constant/CommentCode never washed.
        if th.dark {
            let sw = style(SynKind::Str).wash
                .unwrap_or_else(|| panic!("{}: dark worlds wash strings", th.name));
            let seff = composite(sw, th.base_100);
            let sdl = (seff.to_hsl().2 - th.base_100.to_hsl().2).abs();
            assert!(
                (0.03..=0.12).contains(&sdl),
                "{}: string-wash ΔL {sdl:.3} outside [0.03, 0.12]", th.name
            );
            assert!(
                redmean(ceff, seff) >= 20.0,
                "{}: comment vs string wash effective redmean {:.1} < 20",
                th.name, redmean(ceff, seff)
            );
        } else {
            assert!(style(SynKind::Str).wash.is_none(),
                "{}: light worlds carry NO string wash", th.name);
        }
        assert!(style(SynKind::Definition).wash.is_none()
            && style(SynKind::Constant).wash.is_none()
            && style(SynKind::CommentCode).wash.is_none(),
            "{}: only prose comments (+ dark strings) are washed", th.name);

        // (e) AMBER GUARD over every enrolled role's effective fg.
        //
        // INK-CARET EXEMPTION (the generalized Wagtail precedent — see
        // `Theme::ink_caret`): a world whose caret is the INK's OWN colour
        // (`primary == base_content`) carries NO chromatic accent for a role to
        // steal — its caret's presence is the inverting/filled BLOCK, not a hue —
        // so the ≥30° role-hue gap is moot and SKIPPED for its tinted roles. Two
        // precedents: Wagtail (pure-white ink caret; also caught by the is_one_bit
        // flat exemption above) and Cassowary (phosphor-green caret, so Str at
        // ~140° may sit ~1° from the green ink). Everything ELSE still holds — the
        // ink-caret roles keep their sat ≤ 0.50 cap here AND the full pairwise
        // (a)/perceptibility (g)/luminance (h)/ground-contrast (i) laps below, so
        // Cassowary's tints stay mutually distinguishable ON the green ink.
        //
        // The exemption is SAFE only because such a world MUST invert or fill its
        // block caret — a plain opaque block in the ink's own colour would erase
        // the letter and leave no findable caret. That pairing is LAW-ENFORCED
        // here so it can never drift (ink-caret ⇒ non-Normal block style):
        let ink_caret = th.ink_caret();
        if ink_caret {
            assert!(
                th.render_caps.caret_block_style.folds_morph_to_block(),
                "{}: an ink-caret world (primary == base_content) MUST invert or fill \
                 its block caret (a non-Normal CaretBlockStyle) — a plain opaque block \
                 in the ink's own colour would erase the letter with no findable caret",
                th.name
            );
        }
        let (ph, _, _) = th.primary.to_hsl();
        for k in ROLES {
            let fg = style(k).fg;
            if fg == th.base_content || fg == th.muted {
                continue; // the comment tiers ride the existing inks (exempt by identity)
            }
            // A TINTED role must never coincidentally BE the accent. (On an
            // ink-caret world `primary == base_content`, but a tinted role is held
            // ≥70 redmean off `base_content` by law (g), so it can never equal it.)
            assert_ne!(fg, th.primary, "{}: {k:?} must never BE the accent", th.name);
            let (h, s, _) = fg.to_hsl();
            assert!(s <= 0.5, "{}: {k:?} fg sat {s:.2} > 0.50 (too loud)", th.name);
            if s > 0.15 && !ink_caret {
                let d = hue_dist(h, ph);
                assert!(
                    d >= 30.0,
                    "{}: {k:?} fg hue {h:.0}° only {d:.0}° from primary {ph:.0}°",
                    th.name
                );
            }
        }

        // (f) Presence ordering: Definition closest to full ink, then Constant,
        // then Str — monotone in BOTH modes (lightness distance from base_content).
        let lf = th.base_content.to_hsl().2;
        let dist_l = |k: SynKind| (style(k).fg.to_hsl().2 - lf).abs();
        assert!(
            dist_l(SynKind::Definition) < dist_l(SynKind::Constant),
            "{}: Definition must be more present than Constant", th.name
        );
        assert!(
            dist_l(SynKind::Constant) < dist_l(SynKind::Str),
            "{}: Constant must be more present than Str", th.name
        );

        // (g) PERCEPTIBILITY FLOOR — every tinted role's fg must read as
        // clearly distinct from the page's own ink, not just from its
        // sibling roles (the bug this law exists to catch: Definition
        // cleared the pairwise ≥40 floor at redmean ~43 vs base_content on
        // Currawong yet read as plain white in a live screenshot).
        const PERCEPTIBILITY_FLOOR: f32 = 70.0;
        for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
            let d = redmean(style(k).fg, th.base_content);
            assert!(
                d >= PERCEPTIBILITY_FLOOR,
                "{}: {k:?} fg redmean {d:.1} vs base_content < floor {PERCEPTIBILITY_FLOOR} (imperceptible tint)",
                th.name
            );
        }

        // (h) LUMINANCE FLOOR — redmean alone passed the exact bug this law
        // exists to catch (light Definition, almost all its redmean distance
        // sitting in the low-luminance-weight blue channel). Every tinted
        // role's fg must clear a WCAG relative-luminance ΔY from `base_content`.
        const LUMINANCE_FLOOR: f32 = 0.05;
        let y0 = rel_luminance(th.base_content);
        for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
            let dy = (rel_luminance(style(k).fg) - y0).abs();
            assert!(
                dy >= LUMINANCE_FLOOR,
                "{}: {k:?} fg relative-luminance ΔY {dy:.3} vs base_content < floor {LUMINANCE_FLOOR} (redmean-passing, luminance-invisible)",
                th.name
            );
        }

        // (i) GROUND-CONTRAST FLOOR — (h) alone passed the exact bug this law
        // exists to catch (a light-world fix that satisfies "distinct from
        // ink" by pushing lightness toward `muted`, which is itself already
        // most of the way toward the pale `base_100` ground — camouflage
        // against the PAGE, not the ink). Every tinted role's fg must clear
        // a WCAG contrast RATIO against `base_100` — body-text grade.
        const GROUND_CONTRAST_FLOOR: f32 = 4.5;
        for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
            let cr = contrast_ratio(style(k).fg, th.base_100);
            assert!(
                cr >= GROUND_CONTRAST_FLOOR,
                "{}: {k:?} fg contrast-vs-ground {cr:.2}:1 < floor {GROUND_CONTRAST_FLOOR}:1 (luminance-distinct-from-ink but camouflaged against the page)",
                th.name
            );
        }
    }
}

/// THE HIGHLIGHT-WASH LAW TEST — sweeps EVERY world and asserts the dedicated
/// markdown `==highlight==` wash ([`highlight_wash`]) obeys its own contract,
/// distinct from the comment wash's whisper contract above. The `==highlight==`
/// band was DECOUPLED from the warm comment wash (a deliberate, narrow break of
/// the one-warm-wash owner — a highlighter and a comment wash are different
/// intents): the old shared cream read MUDDY on the cool pale light grounds
/// (Gumtree pale-green, Bilby pale-cyan, Saltpan ecru), a faint warm-over-cool
/// blend with almost no hue contrast, so a highlighter that should POP nearly
/// vanished. THIS ROUND made the hue PER-WORLD — derived from each world's own
/// accent (`hue(primary) + 165°`, a split-complementary), superseding the fixed
/// foreign violet (which read as un-native, the same on every world). The laws,
/// all on the EFFECTIVE `highlight_wash` (lock-free — it takes `&Theme`, never
/// the process-global active theme):
/// - (a) DISTINCT FROM THE COMMENT WASH: the highlight quad rgba is never equal
///   to the world's comment wash — the whole point of the decouple.
/// - (b) AMBER GUARD (DESIGN §3): the per-world hue sits ≥ 30° off that world's
///   `primary` (the 165° split-complement rotation makes it exactly 165° on
///   every world — the caret's amber stays its own).
/// - (c) IT POPS: composited over `base_100` it clears a redmean floor (70) far
///   above the comment wash's own 35 floor, AND out-pops the comment wash on
///   EVERY world (highlight composited redmean > comment composited redmean) —
///   the direct proof it reads louder than the whisper it replaced.
/// - (d) STILL CALM: the composited VALUE step (ΔL vs `base_100`) stays under a
///   ceiling (0.20) — a wash, not a neon slab. (No ΔL FLOOR: on a cool ground
///   the pop is entirely HUE-driven, so its value step is deliberately modest —
///   redmean, not ΔL, is the pop axis for a hue-shift highlight.)
/// - (e) PER-WORLD HUE (the point of this round): the highlight hue VARIES
///   across the worlds — at least 8 distinct hues among the CHROMATIC worlds
///   (proof it is no longer a single fixed value) — while each stays ≥ 15° off
///   its OWN world's ground hue (`base_100`), so no world's highlight muddies
///   against its page.
///
/// **MONOCHROME WORLDS (Wagtail — `Theme::is_monochrome`), adapted HONESTLY,
/// not faked:** an achromatic `primary` has no hue for a split-complementary
/// rotation to land ON, so (b)'s "real chroma" + hue-distance sub-checks and
/// (e)'s ground-hue-distance sub-check are STRUCTURALLY INAPPLICABLE — there is
/// no hue to measure. In their place: `highlight_wash` must report EXACTLY
/// saturation `0.0` (the monochrome law's own floor, enforced again here at
/// this specific call site) — the amber guard is trivially satisfied (a
/// zero-chroma color cannot collide with any hue, `primary`'s included) without
/// needing a fake hue reading. Laws (a)/(c)/(d) — decoupled-from-comment / pops
/// / stays calm — are UNCHANGED and still fully checked: a monochrome
/// highlight must still read as a highlight, just by VALUE instead of HUE.
/// Monochrome worlds are excluded from the per-world variation count (e) —
/// they carry no hue to count.
#[test]
fn highlight_wash_laws_hold_for_every_world() {
    // Pop floor — a highlight composited over the page must clear this, far
    // above the comment wash's own faint-floor of 35 (law (c) above).
    const HIGHLIGHT_POP_FLOOR: f32 = 70.0;
    // Calm ceiling — the composited value step stays a wash, not a slab.
    const HIGHLIGHT_CALM_DL_CEIL: f32 = 0.20;
    // Ground-separation floor — the per-world hue never lands on its own page's
    // hue (measured worst 20.8°, Bilby); anything under this reads muddy.
    const HIGHLIGHT_GROUND_HUE_FLOOR: f32 = 15.0;
    // Per-world variation floor — proof the hue is derived, not fixed.
    const HIGHLIGHT_MIN_DISTINCT_HUES: usize = 8;
    let mut distinct_hues = std::collections::HashSet::new();
    for th in theme::THEMES.iter() {
        let hw = highlight_wash(th);

        // TRUE 1-BIT WORLDS: unlike the merely-monochrome case below (a
        // grey-lightness wash, still present) OR the pre-dither-round answer
        // (fully OFF), a 1-bit world's highlight wash is now the ONE color
        // THE ONE WAGTAIL HIGHLIGHT TEXTURE's dither mode paints — pure
        // OPAQUE white. The dither mechanism itself (every drawn pixel pure,
        // never a fractional alpha) is what keeps this 1-bit-legal despite
        // being opaque, not the alpha token — see `dither.rs`'s real-pixel
        // proof + `wagtail_dither_density`'s doc. Declared exemption from
        // "the highlight wash is always present" (a) through (e) below all
        // assume an ORDINARY translucent wash, which a one-bit world never
        // draws (it dithers instead).
        if th.is_one_bit() {
            assert_eq!(
                (hw.r, hw.g, hw.b, hw.a),
                (0xFF, 0xFF, 0xFF, 0xFF),
                "{}: a true 1-bit world's highlight wash must be the dither's pure opaque white",
                th.name
            );
            continue;
        }
        assert!(hw.a > 0, "{}: the highlight wash is always present", th.name);

        // (a) distinct from the comment wash — the decouple.
        let cw = role_style_for(th, crate::syntax::SynKind::Comment)
            .wash
            .unwrap_or_else(|| panic!("{}: every world carries the comment wash", th.name));
        assert_ne!(
            hw.rgba_bytes(), cw.rgba_bytes(),
            "{}: the highlight wash must be DECOUPLED from (never equal to) the comment wash",
            th.name
        );

        let (hh, hs, _) = hw.to_hsl();
        if th.is_monochrome() {
            // MONOCHROME: no hue to guard or vary — assert the zero-saturation
            // floor directly instead of faking a hue reading.
            assert_eq!(
                hs, 0.0,
                "{}: a monochrome world's highlight wash must carry ZERO saturation \
                 (the no-warm-thing law) — not a faked/derived hue", th.name
            );
        } else {
            // (b) amber guard: the per-world hue sits ≥ 30° off primary.
            let (ph, _, _) = th.primary.to_hsl();
            assert!(hs > 0.15, "{}: highlight wash should carry real chroma", th.name);
            let d = hue_dist(hh, ph);
            assert!(
                d >= 30.0,
                "{}: highlight wash hue {hh:.0}° only {d:.0}° from primary {ph:.0}°",
                th.name
            );
        }

        // (c) it POPS: composited over the page it clears the pop floor AND
        // out-pops the comment wash on this world.
        let heff = composite(hw, th.base_100);
        let ceff = composite(cw, th.base_100);
        let h_pop = redmean(heff, th.base_100);
        let c_pop = redmean(ceff, th.base_100);
        assert!(
            h_pop >= HIGHLIGHT_POP_FLOOR,
            "{}: highlight wash too faint (composited redmean {h_pop:.1} < floor {HIGHLIGHT_POP_FLOOR})",
            th.name
        );
        assert!(
            h_pop > c_pop,
            "{}: the highlight wash must out-pop the comment whisper (highlight redmean {h_pop:.1} <= comment {c_pop:.1})",
            th.name
        );

        // (d) still calm: the composited value step stays under the ceiling.
        let dl = (heff.to_hsl().2 - th.base_100.to_hsl().2).abs();
        assert!(
            dl <= HIGHLIGHT_CALM_DL_CEIL,
            "{}: highlight wash ΔL {dl:.3} over the calm ceiling {HIGHLIGHT_CALM_DL_CEIL} (reads as a slab, not a wash)",
            th.name
        );

        if th.is_monochrome() {
            continue; // no hue: excluded from (e)'s ground-hue + variation count
        }
        // (e) per-world: the hue never muddies against this world's OWN ground.
        let (gh, _, _) = th.base_100.to_hsl();
        let dg = hue_dist(hh, gh);
        assert!(
            dg >= HIGHLIGHT_GROUND_HUE_FLOOR,
            "{}: highlight wash hue {hh:.0}° only {dg:.0}° from its ground {gh:.0}° (muddy)",
            th.name
        );
        distinct_hues.insert(hh.round() as i32);
    }
    // (e) per-world variation: the hue is derived, not a single fixed value.
    assert!(
        distinct_hues.len() >= HIGHLIGHT_MIN_DISTINCT_HUES,
        "highlight hue must VARY per world: only {} distinct hues across {} worlds (< {})",
        distinct_hues.len(), theme::THEMES.len(), HIGHLIGHT_MIN_DISTINCT_HUES
    );
}

/// SCRATCH measurement harness (not a law): prints redmean + relative-luminance
/// distance from `base_content` for every tinted role on every world, to
/// calibrate the luminance floor. Run with
/// `cargo test measure_role_luminance -- --nocapture --ignored`.
#[test]
#[ignore]
fn measure_role_luminance() {
    use crate::syntax::SynKind;
    for th in theme::THEMES.iter() {
        let y0 = rel_luminance(th.base_content);
        let ym = rel_luminance(th.muted);
        eprintln!("{:10} dark={:5} MUTED dY={:.4}", th.name, th.dark, (ym - y0).abs());
        for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
            let style = role_style_for(th, k);
            let d = redmean(style.fg, th.base_content);
            let dy = (rel_luminance(style.fg) - y0).abs();
            eprintln!(
                "{:10} dark={:5} {:10?} redmean={:6.1} dY={:.4} fg={:?}",
                th.name, th.dark, k, d, dy, style.fg
            );
        }
    }
}

/// Relative luminance per WCAG (gamma-decoded, Rec.709 weights). Alpha ignored.
/// SCRATCH helper for `measure_role_luminance` / `sweep_light_ladder`.
fn rel_luminance(c: theme::Srgb) -> f32 {
    let lin = |v: u8| {
        let x = v as f32 / 255.0;
        if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast RATIO between two colors ((L1+0.05)/(L2+0.05), L1 the
/// lighter). SCRATCH helper for `measure_ground_contrast` / `sweep_light_ladder`.
fn contrast_ratio(a: theme::Srgb, b: theme::Srgb) -> f32 {
    let (ya, yb) = (rel_luminance(a), rel_luminance(b));
    let (hi, lo) = if ya > yb { (ya, yb) } else { (yb, ya) };
    (hi + 0.05) / (lo + 0.05)
}

/// SCRATCH measurement (not a law): WCAG contrast ratio of every tinted role's
/// fg against `base_100` (the GROUND, not the ink) on every world — the axis
/// the pre-(i) law suite never checked (ink-distance alone permits
/// background-camouflage; see THEMES.md). Run with `cargo test
/// measure_ground_contrast -- --nocapture --ignored`.
#[test]
#[ignore]
fn measure_ground_contrast() {
    use crate::syntax::SynKind;
    for th in theme::THEMES.iter() {
        for k in [SynKind::Definition, SynKind::Constant, SynKind::Str] {
            let style = role_style_for(th, k);
            let cr = contrast_ratio(style.fg, th.base_100);
            eprintln!("{:10} dark={:5} {:10?} contrast-vs-ground={:5.2}:1", th.name, th.dark, k, cr);
        }
    }
}

/// SCRATCH param sweep (not a law): tries a grid of `(t_def, t_const, t_str, s)`
/// light-ladder candidates directly against `role_style_for`'s formula (mirrored
/// here since the constants aren't parameterized). Round 2 (the ground-contrast
/// retune): a candidate must clear EVERY existing law (pairwise ≥40,
/// perceptibility ≥70, ink-luminance ΔY ≥0.05) PLUS the new ground-contrast
/// floor (≥4.5:1 vs `base_100`) simultaneously — reports the winner ranked by
/// worst-case ground contrast (the axis round 1 never searched for; see
/// THEMES.md and the `T_LIGHT` doc comment in `render/spans.rs`). Run with
/// `cargo test sweep_light_ladder -- --nocapture --ignored`.
#[test]
#[ignore]
fn sweep_light_ladder() {
    const HUE_DEF: f32 = 220.0;
    const HUE_CONST: f32 = 290.0;
    const HUE_STR: f32 = 140.0;
    const GROUND_FLOOR: f32 = 4.5; // WCAG body-text-grade contrast ratio vs base_100
    const LUM_FLOOR: f32 = 0.05;
    let light_worlds: Vec<_> = theme::THEMES.iter().filter(|t| !t.dark).collect();

    let mut best: Option<(f32, (f32, f32, f32, f32))> = None;
    let mut t_def = 0.20;
    while t_def <= 0.85 {
        let mut t_const = t_def + 0.01;
        while t_const <= 0.90 {
            let mut t_str = t_const + 0.01;
            while t_str <= 0.95 {
                let mut s = 0.15;
                while s <= 0.50 {
                    let mut ok = true;
                    let mut worst_ground = f32::INFINITY;
                    for th in &light_worlds {
                        let (_, _, l_full) = th.base_content.to_hsl();
                        let (_, _, l_dim) = th.muted.to_hsl();
                        let fg_at = |anchor: f32, ti: f32| {
                            theme::Srgb::from_hsl(anchor, s, l_full + (l_dim - l_full) * ti)
                        };
                        let def = fg_at(HUE_DEF, t_def);
                        let cst = fg_at(HUE_CONST, t_const);
                        let st = fg_at(HUE_STR, t_str);
                        let muted = th.muted;
                        let base = th.base_content;
                        let pairs = [
                            redmean(def, cst), redmean(def, st), redmean(def, muted),
                            redmean(cst, st), redmean(cst, muted), redmean(st, muted),
                        ];
                        if pairs.iter().any(|d| *d < 40.0) { ok = false; break; }
                        let floors = [redmean(def, base), redmean(cst, base), redmean(st, base)];
                        if floors.iter().any(|d| *d < 70.0) { ok = false; break; }
                        let y0 = rel_luminance(base);
                        let dys = [
                            (rel_luminance(def) - y0).abs(),
                            (rel_luminance(cst) - y0).abs(),
                            (rel_luminance(st) - y0).abs(),
                        ];
                        if dys.iter().any(|d| *d < LUM_FLOOR) { ok = false; break; }
                        let grounds = [
                            contrast_ratio(def, th.base_100),
                            contrast_ratio(cst, th.base_100),
                            contrast_ratio(st, th.base_100),
                        ];
                        for g in grounds {
                            worst_ground = worst_ground.min(g);
                        }
                    }
                    if ok && worst_ground >= GROUND_FLOOR {
                        if best.map(|(b, _)| worst_ground > b).unwrap_or(true) {
                            best = Some((worst_ground, (t_def, t_const, t_str, s)));
                        }
                    }
                    s += 0.01;
                }
                t_str += 0.01;
            }
            t_const += 0.01;
        }
        t_def += 0.01;
    }
    eprintln!("BEST (worst-case ground contrast, subject to every law): {:?}", best);
    eprintln!("SHIPPED (rounded, chosen for margin on BOTH the luminance and ground floors): \
        T_LIGHT=[0.76,0.78,0.80] S_FG_LIGHT=0.18 — worst ground 4.84:1 (Quokka Str), worst ink dY 0.056 (Gumtree Def/Const)");
}

/// THE INK-LADDER + SELECTION LAW TEST — sweeps every world in `theme::THEMES`
/// and asserts the non-role-tint half of the audit: the ink ladder
/// (`base_content` → `muted` → `faint`) steps monotonically toward the
/// background and each step stays perceptibly distinct, `faint` (the dimmest
/// UI-metadata rung — gutter line numbers, debug panel, stats HUD captions)
/// stays legible against its own `base_100`, and `selection` is a QUIET
/// highlight — visible but never reading as a paint bucket. Thresholds
/// calibrated from the measured 14-world table (`measure_ink_ladder`, an
/// ignored scratch test):
/// (a) `base_content`→`muted` redmean ≥ 100 (worst measured 201.9, Gumtree)
///     and `muted`→`faint` redmean ≥ 80 (worst measured 116.7, Potoroo) —
///     each ladder rung reads as its own distinct step, not a copy of its
///     neighbor;
/// (b) monotone LIGHTNESS: `faint` sits strictly between `muted` and
///     `base_100` in HSL lightness (further toward the background than
///     `muted`, but not AT the background) on every world — the ladder never
///     reverses or collapses;
/// (c) `faint` vs `base_100` redmean ≥ 100 (worst measured 166.6, Mopoke) —
///     the faintest rung still reads as present ink, not invisible;
/// (d) selection COMPOSITED over `base_100` at its authored alpha (what the eye
///     actually sees — NOT the opaque tint, which flattered a sub-glance
///     highlight) clears a CONTRAST FLOOR: composited-vs-ground redmean ≥ 35 AND
///     ΔL ≥ 0.10, so a selection can never read as "you can't tell it's
///     highlighted" (the reported Undertow/Mangrove bug: those two composited to
///     only ΔL 0.090 / 0.076, invisible enough to fail this law before their
///     tints were lifted in-hue). Still CALM: ΔL ≤ 0.35 (a quiet highlight, never
///     a solid paint fill — worst 0.231, Outback). Floor calibrated to fail the
///     two worst offenders; every world now clears ΔL 0.118 (Currawong).
#[test]
fn ink_ladder_and_selection_laws_hold_for_every_world() {
    for th in theme::THEMES.iter() {
        // TRUE 1-BIT WORLDS (`Theme::is_one_bit`): (a)/(b)/(c) below assume a
        // three-rung ink ladder (base_content -> muted -> faint, each a
        // DISTINCT step) to derive a "still legible but receded" faint rung
        // from — a 1-bit world has exactly ONE ink value, so the ladder
        // COLLAPSES by design (declared exemption, not a weakening: there is
        // no ladder left to step through). (d)'s calm-ΔL-ceiling law also
        // assumes a TRANSLUCENT selection wash — a 1-bit world's selection is
        // instead a fully OPAQUE white `selection` token (ΔL is necessarily
        // 1.0, a "paint fill" by the old law's own words), with legibility
        // carried by a SEPARATE render-side mechanism entirely (the DITHER
        // round's TRUE inverse-video pipeline, `TextPipeline::selection_invert`
        // — drawn AFTER text with a `OneMinusDst` blend, flipping black<->white
        // wherever it covers), not by tuning this token's alpha at all.
        // Replaced by the flat + selection laws this world's design actually
        // demands.
        if th.is_one_bit() {
            assert_eq!(th.base_content, th.muted, "{}: one-bit ink ladder collapses to one value", th.name);
            assert_eq!(th.muted, th.faint, "{}: one-bit ink ladder collapses to one value", th.name);
            assert_eq!(
                (th.base_content.r, th.base_content.g, th.base_content.b),
                (0xFF, 0xFF, 0xFF),
                "{}: one-bit ink is pure white", th.name
            );
            assert_eq!(
                (th.selection.r, th.selection.g, th.selection.b, th.selection.a),
                (0xFF, 0xFF, 0xFF, 0xFF),
                "{}: one-bit selection is pure OPAQUE white (legibility is the punch quad's job, not this token's alpha)",
                th.name
            );
            continue;
        }
        // (a) Distinct steps.
        let step1 = redmean(th.base_content, th.muted);
        assert!(step1 >= 100.0, "{}: content->muted redmean {step1:.1} < 100", th.name);
        let step2 = redmean(th.muted, th.faint);
        assert!(step2 >= 80.0, "{}: muted->faint redmean {step2:.1} < 80", th.name);

        // (b) Monotone lightness: faint strictly between muted and base_100.
        let l_muted = th.muted.to_hsl().2;
        let l_faint = th.faint.to_hsl().2;
        let l_bg = th.base_100.to_hsl().2;
        if th.dark {
            // Dark world: ink lightens toward background as it dims... no —
            // background is DARKEST, ink is light; faint recedes TOWARD the
            // dark background, so l_faint sits between l_bg and l_muted.
            assert!(
                l_faint < l_muted && l_faint > l_bg,
                "{}: faint lightness {l_faint:.3} not between bg {l_bg:.3} and muted {l_muted:.3}",
                th.name
            );
        } else {
            assert!(
                l_faint > l_muted && l_faint < l_bg,
                "{}: faint lightness {l_faint:.3} not between muted {l_muted:.3} and bg {l_bg:.3}",
                th.name
            );
        }

        // (c) Faint stays legible against its own background.
        let fvb = redmean(th.faint, th.base_100);
        assert!(fvb >= 100.0, "{}: faint vs base_100 redmean {fvb:.1} < 100 (too faint to read)", th.name);

        // (d) Selection COMPOSITED over the ground is a quiet, GLANCEABLE
        // highlight — measured on what the eye sees, not the opaque tint.
        let eff = composite(th.selection, th.base_100);
        let svb = redmean(eff, th.base_100);
        assert!(
            svb >= 35.0,
            "{}: selection composited vs base_100 redmean {svb:.1} < 35 (near-invisible)",
            th.name
        );
        let dl = (eff.to_hsl().2 - l_bg).abs();
        assert!(
            dl >= 0.10,
            "{}: selection composited ΔL {dl:.3} < 0.10 — sub-glance, you can't tell it's highlighted",
            th.name
        );
        assert!(
            dl <= 0.35,
            "{}: selection composited ΔL {dl:.3} > 0.35 — reads as a solid paint fill, not a calm highlight",
            th.name
        );
    }
}

/// SCRATCH measurement (not a law): ink-ladder step sizes (content->muted,
/// muted->faint) and faint-vs-background legibility, plus selection-vs-
/// background distance, for every world. Informs the audit's ladder/selection
/// laws. Run with `cargo test measure_ink_ladder -- --nocapture --ignored`.
#[test]
#[ignore]
fn measure_ink_ladder() {
    for th in theme::THEMES.iter() {
        let y = |c: theme::Srgb| rel_luminance(c);
        eprintln!(
            "{:10} dark={:5} content->muted redmean={:6.1} dY={:.3} | muted->faint redmean={:6.1} dY={:.3} | faint-vs-bg redmean={:6.1} dY={:.3} | selection-vs-bg redmean={:6.1}",
            th.name, th.dark,
            redmean(th.base_content, th.muted), (y(th.base_content) - y(th.muted)).abs(),
            redmean(th.muted, th.faint), (y(th.muted) - y(th.faint)).abs(),
            redmean(th.faint, th.base_100), (y(th.faint) - y(th.base_100)).abs(),
            redmean(theme::Srgb::rgb(th.selection.r, th.selection.g, th.selection.b), th.base_100),
        );
        let sel_eff = composite(th.selection, th.base_100);
        let dl = (sel_eff.to_hsl().2 - th.base_100.to_hsl().2).abs();
        eprintln!("{:10} selection composited ΔL={:.3}", th.name, dl);
    }
}

/// COMMENT PROMINENCE at the attrs seam: a code buffer's prose comment shapes
/// at the FULL content ink (decision 2 made render-real), and a commented-out
/// statement keeps the muted grey.
#[test]
fn syn_attrs_comment_tiers() {
    use crate::syntax::SynKind;
    let _g = crate::testlock::serial();
    theme::set_active_by_name("Tawny").unwrap();
    let base = Attrs::new();
    let th = theme::active();
    assert_eq!(
        syn_attrs(&base, SynKind::Comment).color_opt,
        Some(th.base_content.to_glyphon()),
        "prose comment shapes at FULL content ink"
    );
    assert_eq!(
        syn_attrs(&base, SynKind::CommentCode).color_opt,
        Some(th.muted.to_glyphon()),
        "commented-out code keeps the muted grey"
    );
    theme::set_active(theme::DEFAULT_THEME);
}

/// THE MONOCHROME LAW — the new law Wagtail's existence demands (THEMES.md's
/// logged DESIGN.md §3 "no warm thing" amendment): for every world that
/// `Theme::is_monochrome()` names (Wagtail today; a future monochrome world is
/// enrolled automatically), EVERY color that world renders — the palette
/// struct's own fields, the caret (`primary`) INCLUDED, no exceptions — carries
/// HSL saturation `0.0`. This is what actually PINS the world's whole identity:
/// a future hand-edit that quietly nudges one grey toward a hue (a "just a
/// touch of warmth" temptation, the exact opposite of this world's point) fails
/// HERE, structurally, rather than surviving as an unnoticed drift.
///
/// A no-wildcard sweep over every `Srgb`-valued surface a monochrome world
/// actually paints: the ink ladder (`base_100/200/300`, `base_content`,
/// `muted`, `faint`), the accents (`primary`, `primary_content`, `error`,
/// `selection`), the margin ground (`background`'s `from`/`to`/`tint`
/// endpoints), the EFFECTIVE syntax role styles (`role_style_for`'s fg +
/// wash for all four roles, overrides included — Wagtail's own
/// `RoleOverrides` pins, proven monochrome at the point they're actually
/// consumed, not just eyeballed at the literal), and the dedicated
/// `==highlight==` wash (`highlight_wash`, which needed its own monochrome
/// branch — see its doc comment — to avoid deriving a hue from a hue that
/// doesn't exist). Every check ignores alpha (translucency is orthogonal to
/// hue) and uses an EXACT `== 0.0` comparison, not a threshold — `Srgb::to_hsl`
/// reports saturation `0.0` exactly for any achromatic (`r == g == b`) color,
/// so there is no meaningful "almost zero" case to tolerate.
///
/// If `THEMES` ever ships a SECOND monochrome world, this test enrolls it for
/// free (it iterates `THEMES` filtered by `is_monochrome()`, never a hardcoded
/// name) — and if Wagtail ever stops being monochrome (a `primary` hue creeps
/// in), `is_monochrome()` simply stops selecting it and this test silently
/// covers nothing, which is why the OTHER structural laws (`worlds_nine_dark_
/// six_light`, `role_style_laws_hold_for_every_world`, …) still separately
/// pin Wagtail's exact hex literals — this law is a property test on TOP of
/// those, not a replacement for them.
#[test]
fn every_monochrome_world_renders_zero_saturation_everywhere() {
    fn assert_grey(c: theme::Srgb, world: &str, label: &str) {
        let (_, s, _) = c.to_hsl();
        assert_eq!(
            s, 0.0,
            "{world}: {label} carries saturation {s:.3} (expected exactly 0.0 — \
             {label} = #{r:02x}{g:02x}{b:02x})",
            r = c.r, g = c.g, b = c.b
        );
    }

    let monochrome: Vec<&theme::Theme> =
        theme::THEMES.iter().filter(|t| t.is_monochrome()).collect();
    assert!(
        !monochrome.is_empty(),
        "no monochrome world found — Wagtail should make `theme::THEMES` non-empty here"
    );

    for th in monochrome {
        // The palette struct's own tokens.
        assert_grey(th.base_100, th.name, "base_100");
        assert_grey(th.base_200, th.name, "base_200");
        assert_grey(th.base_300, th.name, "base_300");
        assert_grey(th.base_content, th.name, "base_content");
        assert_grey(th.muted, th.name, "muted");
        assert_grey(th.faint, th.name, "faint");
        assert_grey(th.primary, th.name, "primary (THE CARET — no exceptions)");
        assert_grey(th.primary_content, th.name, "primary_content");
        assert_grey(th.error, th.name, "error");
        assert_grey(th.selection, th.name, "selection");

        // The margin ground.
        assert_grey(th.background.from(), th.name, "background.from");
        assert_grey(th.background.to(), th.name, "background.to");
        assert_grey(th.background.tint(), th.name, "background.tint");

        // The EFFECTIVE syntax role styles (fg + wash), overrides included —
        // the same no-wildcard roster `role_style_laws_hold_for_every_world`
        // sweeps, so a future SynKind variant fails to compile there first.
        use crate::syntax::SynKind;
        for k in [
            SynKind::Comment,
            SynKind::CommentCode,
            SynKind::Str,
            SynKind::Constant,
            SynKind::Definition,
        ] {
            let style = role_style_for(th, k);
            assert_grey(style.fg, th.name, &format!("{k:?} fg"));
            if let Some(wash) = style.wash {
                assert_grey(wash, th.name, &format!("{k:?} wash"));
            }
        }

        // The dedicated `==highlight==` wash.
        assert_grey(highlight_wash(th), th.name, "highlight_wash");
    }
}

/// THE 1-BIT LAW — supersedes the monochrome law above for whichever worlds
/// are ALSO `Theme::is_one_bit()` (Wagtail's 2026-07 rework: greyscale ->
/// true 1-bit, "only black or white, no gray"). The monochrome law tolerates
/// ANY grey (`saturation == 0` alone); this one is strictly narrower — every
/// authored color a one-bit world renders must be EXACTLY `#000000` or
/// `#FFFFFF`, full stop. Ignores ALPHA (translucency is a compositing
/// concern, not a hue/value one — the alpha-composite half of the law is
/// covered separately, see below) and uses exact equality, not a threshold —
/// there is no "almost pure" case to tolerate for an authored literal.
///
/// A no-wildcard sweep over the SAME surfaces the monochrome law above
/// enumerates (palette struct fields, background endpoints, effective role
/// styles, the highlight wash), PLUS two 1-bit-specific additions the
/// monochrome law had no reason to check: (1) `background.from() ==
/// background.to()` — a flat gradient is the ONE `Background` variant
/// mathematically guaranteed to introduce no interpolated grey (any
/// `Dots`/`Starfield`/`Pinstripe`/`Stripes` mark tint, or a real two-endpoint
/// gradient, would); (2) every role's wash is `None` outright (not merely
/// "grey if present" — a translucent wash of ANY color composites a forbidden
/// grey over a differing ground, so 1-bit worlds carry no role washes at
/// all, verified again here at the point they're actually consumed).
///
/// If `THEMES` ever ships a SECOND one-bit world, this test enrolls it for
/// free (filters by `is_one_bit()`, never a hardcoded name).
#[test]
fn every_one_bit_world_renders_only_pure_black_or_white() {
    fn assert_pure_bw(c: theme::Srgb, world: &str, label: &str) {
        assert!(
            matches!((c.r, c.g, c.b), (0, 0, 0) | (255, 255, 255)),
            "{world}: {label} = #{r:02x}{g:02x}{b:02x} is neither pure black nor pure white",
            r = c.r, g = c.g, b = c.b
        );
    }

    let one_bit: Vec<&theme::Theme> = theme::THEMES.iter().filter(|t| t.is_one_bit()).collect();
    assert!(
        !one_bit.is_empty(),
        "no one-bit world found — Wagtail's 2026-07 rework should make this non-empty"
    );

    for th in one_bit {
        // The palette struct's own tokens.
        assert_pure_bw(th.base_100, th.name, "base_100");
        assert_pure_bw(th.base_200, th.name, "base_200");
        assert_pure_bw(th.base_300, th.name, "base_300");
        assert_pure_bw(th.base_content, th.name, "base_content");
        assert_pure_bw(th.muted, th.name, "muted");
        assert_pure_bw(th.faint, th.name, "faint");
        assert_pure_bw(th.primary, th.name, "primary (THE CARET — no exceptions)");
        assert_pure_bw(th.primary_content, th.name, "primary_content");
        assert_pure_bw(th.error, th.name, "error");
        assert_pure_bw(th.selection, th.name, "selection");

        // The margin ground: pure b/w endpoints, AND (1-bit-specific) the two
        // endpoints must be IDENTICAL — a flat gradient is the one variant
        // guaranteed to introduce no interpolated grey between them.
        assert_pure_bw(th.background.from(), th.name, "background.from");
        assert_pure_bw(th.background.to(), th.name, "background.to");
        assert_eq!(
            th.background.from(), th.background.to(),
            "{}: a one-bit world's background gradient must have from == to \
             (any real gradient interpolates through forbidden greys)", th.name
        );

        // The EFFECTIVE syntax role styles: fg pure b/w AND (1-bit-specific)
        // no wash at all — not merely "grey if present".
        use crate::syntax::SynKind;
        for k in [
            SynKind::Comment,
            SynKind::CommentCode,
            SynKind::Str,
            SynKind::Constant,
            SynKind::Definition,
        ] {
            let style = role_style_for(th, k);
            assert_pure_bw(style.fg, th.name, &format!("{k:?} fg"));
            assert!(
                style.wash.is_none(),
                "{}: one-bit {k:?} must carry NO wash (any alpha over a differing ground is a forbidden grey)",
                th.name
            );
        }

        // The dedicated `==highlight==` wash: pure opaque white — THE ONE
        // WAGTAIL HIGHLIGHT TEXTURE's color token (the pixel-purity guarantee
        // itself comes from the DITHER MECHANISM — every drawn pixel is this
        // exact color at full alpha or fully transparent, never a fractional
        // blend — not from this token being transparent; see `dither.rs`'s
        // real-pixel proof).
        let hw = highlight_wash(th);
        assert_pure_bw(hw, th.name, "highlight_wash");
        assert_eq!(
            hw.a, 255,
            "{}: one-bit highlight_wash must be fully OPAQUE (the dither's pure quad color)",
            th.name
        );
    }
}
