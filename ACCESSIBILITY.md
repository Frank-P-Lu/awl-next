# Accessibility — where awl stands (tier 1, 2026-07)

A calm, honest statement of what works today, what the known gap is, and where
the seams are for a future tier. No promised dates — this is a running note,
not a roadmap. See `CLAUDE.md`'s doc list for the rest of the contract docs
this one sits alongside.

## What works today

- **Fully keyboard-drivable.** Every command is reachable from the Cmd-P
  command palette by name — there is no mouse-only affordance anywhere in
  awl (DESIGN.md's button-free rule: a chord or a summoned palette command,
  never a floating toolbar). The two-binding keymap (native ⌘ as the
  advertised layer, a quiet Emacs second slot) means the whole editor — open,
  edit, save, search, format, switch project, rebind keys, everything — is
  operable without ever touching a pointing device.
- **Zoom.** Cmd-=/Cmd–/Cmd-0 (and the Settings "Zoom" row) scale the whole
  document glyph-for-glyph, independent of the page column measure.
- **14 curated theme worlds, each contrast-law-tested.** Every world's ink
  ladder (`base_content` → `muted`) and role tints are checked by an
  automated law test for pairwise distinguishability and an amber guard (see
  THEMES.md) — a world can't ship with illegible or indistinguishable text.
  This is a contrast *floor*, not a dedicated high-contrast mode; there is no
  separate "increase contrast" toggle beyond picking a world.
- **Reduce Motion (this round).** A real accessibility preference, not a
  cosmetic toggle: `Settings → Editor → Reduce motion`, or the config key
  `reduce_motion = true`. When on, every juice animation (the caret's
  spring/glide, its squash-pop flinches and trailing streak, the copy-pulse
  selection brighten, the caret-style picker's choreographed preview loop)
  settles INSTANTLY to its exact final state instead of easing over time —
  same position, same color, same everything; motion-off is a pure time
  compression, never a feature change. Absent config means `auto`: awl reads
  the OS-level preference where one is reachable (macOS System Settings →
  Accessibility → Display → Reduce Motion; the web build's
  `prefers-reduced-motion` media query), consulted once at launch. Native
  Linux has no reliable cross-desktop accessibility API wired here yet, so
  `auto` reads as off there — the config key is the door: set
  `reduce_motion = true` by hand.
- **The window title names the document.** The OS window (and its title-bar
  text, which a screen reader's window list announces) always reads
  `awl - <path or "scratch"/"*scratch*"> [<world name>]` — never a bare
  "awl". This was true before this round; it is now driven by one shared,
  unit-tested function (`app::files::window_title`) instead of two
  independently-hand-written copies, so the very first frame after launch
  and every later open/switch/theme-change agree.

## The known gap, stated plainly

**awl has no screen-reader / VoiceOver support.** The editor draws its own
text with wgpu straight onto a GPU surface — there is no `NSTextView`, no
DOM, no platform text-widget underneath any of it, so there is no
accessibility tree for VoiceOver (or any other screen reader) to read. A
blind or low-vision user relying on a screen reader cannot use awl today:
the document contents, the summoned overlays (palette, pickers, Settings),
and the caret position are all invisible to assistive technology, full stop.

This is not a small gap and it is not hidden here: it is the single largest
accessibility limitation in the app, and it is a direct consequence of the
same custom-rendering architecture that gives awl its calm, GPU-drawn feel
(ARCHITECTURE.md). Closing it is a real engineering project, not a toggle.

**The named path: [AccessKit](https://accesskit.dev/).** AccessKit is the
crate the Rust GUI ecosystem (egui, Xilem, and others) has converged on for
exactly this problem — a cross-platform accessibility-tree abstraction a
custom-rendered app can populate (node roles, labels, text ranges, focus)
and that AccessKit itself bridges to each OS's real accessibility API
(NSAccessibility on macOS, UIA on Windows, AT-SPI2 on Linux, the DOM-based
web story separately). It is **banked, not built**: no AccessKit dependency
exists in this tree yet, and wiring it is a genuine round of its own —
deciding what awl's node tree even *is* (the document as one giant text
node? a node per line? per paragraph?), keeping it in sync with every edit
without becoming a second source of truth, and exposing the summoned
overlays' own transient structure. Logged here so it is a known, named
destination rather than a silently-dropped idea.

## Hold-gestures note

Two features in awl are **holds** — press-and-hold-to-peek, game-map style,
not a toggle:

- **The stats HUD** (Option-Cmd-I) — file-created date, session time, word
  count/reading time, percent through document — shows while held, vanishes
  on release.
- **The Cmd-P "peek"-style summon flow** for a picker preview (e.g. the
  caret-style picker's live choreographed demo) likewise only animates while
  its picker is open.

Neither hold gates information that is otherwise unreachable: every figure
the stats HUD shows is also derivable without holding anything (word count
and reading time render in the quiet bottom-right readout for any markdown
buffer; the file's path and the active theme are always in the window
title; session time is the one figure with no non-hold equivalent today, a
narrow, logged gap). A hold-only affordance is a genuine keyboard operation
(a single chord, held) rather than a mouse gesture, so it does not itself
block keyboard-only use — but a hold does require the physical ability to
keep a key depressed, which is worth naming honestly rather than assuming
away.

## Where this leaves tier 2

Reduce Motion (this round) and the pre-existing keyboard-first design close
the two accessibility needs awl's own architecture makes cheap: motion is a
render-side settle-instantly gate, and full keyboard operability was already
the whole design (DESIGN.md, SCOPE.md). Screen-reader support is the
opposite kind of gap — expensive, architectural, and honestly the reason a
"tier 2" is a real future round rather than a follow-up patch. This document
will be updated when that round lands, not before.
