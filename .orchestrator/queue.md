# awl — live build queue

> Live execution state only. Completed and superseded work is in git history
> (`git log -p .orchestrator/queue.md`). Protocol, claiming, worktrees, and
> execution hygiene live in `.orchestrator/README.md`.

## Ready — current user-visible wave

71. **Bowerbird’s summoned cards gain a quiet woven jagged-wave material (user-inspired; world assignment delegated).** Extend item 70’s closed `CardTexture` capability with `JaggedWave { … }` and assign it to Bowerbird; its glossy blue-black bower is the natural home for the interlocking angular rhythm, while Quokka keeps its separate printed halftone identity. Draw two or three broad nested chevron/wave tiers across the complete card-local field, horizontally phase-offset so they read as one woven surface rather than tiny repeating zigzag wallpaper. Use only Bowerbird’s derived card-surface ladder at low contrast: no amber, raw colour, randomness, animation, new border shape, or change to text/hit geometry. The pattern is clipped by the existing rectangular card silhouette, shares coordinates across Split Pane’s two surfaces and open gap, and becomes quieter beneath the content-heavy middle so query, rows, shortcuts, muted/faint ink, and selection remain immediately legible; it stays inert on Bars/no-card worlds, page backgrounds, and non-overlay HUD/menu/which-key floats. The implementer generates real Bowerbird captures for current plus two- and three-tier variants at quiet/medium contrast, Pane split, small popup, Theme-picker crossing, wide/narrow, 1×/2× DPI, and Bowerbird beside Quokka/Mopoke. Fable inspects only those images and returns the tier count, tooth scale/phase, contrast, and content-rolloff taste call; Fable does no implementation or file editing, and the implementer applies the call. Verify the exhaustive texture roster, every non-Bowerbird world byte-identical, card/text contrast at representative pattern phases, coordinate continuity/no paint in the split gap, deterministic/WebGL2 captures, release frame cost/no idle CPU work, full suite/native+wasm, and `WORLDS.md`/`THEMES.md` in the landing round. **DISPATCH after item 70; keep as a separate taste/assignment round.**

## Ready — shared ownership and performance

56. **Move active buffer + buffer-scoped state as one owned slot, then remove the shadow active path.** In sequential phases: (A) move buffer identity, folds/view state, and caches atomically between active state and the registry, retiring manual snapshot/restore ownership; (B) make `Buffer::path` authoritative and remove duplicate `App.file`, retaining separately named metadata only for genuinely different concepts. Do not add a generic session framework or broad live/headless driver. Verify exhaustive A→B→A sentinels, version-zero cache isolation, fresh defaults, and path laws across open/new-note first save/autoname/rename/move/duplicate/close/autosave/history/session restore; native + wasm gates. Decompose `app/files.rs` along the new private ownership boundaries. **DISPATCH after the user-visible wave; high-risk sequential round.**

## Timed — not blocked

20. **Pre-tag taste pass.** At the user’s explicit tag/release start, the implementation/release owner generates one current world screenshot export, then Fable judges only those images for per-world bullets, squiggle size/baseline including Bilby, dash padding, and Saltpan font outcomes; Fable never implements or edits. Ordinary pushes do not trigger it.

24. **Release-adjacent user-facing docs refresh.** After the current user-visible wave settles and before release preparation, update GUIDE, welcome/tour, and site guide for the current product, chords, and features. Matter-of-fact voice; facts verified. Site copy may change; deployment remains separately user-gated.

## Parked — explicit gate or future design

- **Export save-dialog scope:** macOS + Linux, one live-only cross-platform seam; capture uses an explicit path. Decided, not scheduled.
- **Per-world living-band choreography:** audition TwoShape/Slam/Soft against Morph; live feel is the oracle. Needs a design session.
- **Per-world copy-pulse differentiation:** possible future motion tweak; needs a design session.
- **Site deployment:** only on the user’s explicit word.

## Monitoring — non-blocking

- **Hands-on checks still useful:** Dawn/Bilby world feel; writer-diff panel/Tab + zoom readout; phantom image resize handle; upward scrolling past images in release; right-click Add-to-dictionary summon; 2px Wagtail stipple taste.
- **GPU memory:** no action unless the 6 GB symptom recurs; then probe the live surface with the window foregrounded.

## Release blockers and reminders

- App icon.
- Dictionary/font/license notices plus code copyright/NOTICE review.
- Apple signing secrets and Fly deployment token; see `RELEASING.md`.
- Tags and releases require the user’s explicit word. A dry run may precede them.
