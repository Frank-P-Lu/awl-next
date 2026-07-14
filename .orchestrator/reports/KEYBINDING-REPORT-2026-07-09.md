# Evening Design Report — awl keyboard idioms: the native-slot audit

*Prepared against the post-identity-round tree (keymap.rs + commands.rs read directly, not from docs; menu.rs, peek.rs, hud seams, and the search intercept verified in source). This is a TASTE-GRADE audit — no code was changed. Every judgment is measured against the deepest macOS idiom: Apple HIG, the Cocoa text system's standard bindings (what every NSTextView gives free), and the macOS-native writing-tool cluster (TextEdit / Notes / Pages / Bear / iA Writer / Craft / Things / BBEdit / Xcode). Everything stays inside awl's contract: two slots max, one accent, calm, summoned-not-furniture, chords-buy-territories.*

---

## 1. TL;DR

1. **One bug-grade finding underneath everything: an unbound ⌘-chord self-inserts its letter.** `keymap.rs`'s own test pins `⌘H → InsertChar('h')` — pressing ⌘G, ⌘K, ⌘W, ⌘D, ⌘, or any unassigned Super chord **types a character into the document**. On macOS an unhandled command-key chord must be inert (at most a beep), never text. Fix this first; every proposal below stands on it.

2. **Three chords are missing that every Mac app has:** **⌘, → Settings** (THE preferences idiom; awl's Settings menu is palette-only today), **⌘G / ⇧⌘G → find next / previous** (today "next" exists only as ⌘F-again *while the panel is open*, and the query dies with the panel), and the **App-menu Hide block** (Hide ⌘H · Hide Others ⌥⌘H · Show All — muda predefined items, the same OS-chrome class as Minimize/Zoom).

3. **The ⌘I question has a real answer:** tap-vs-hold is feasible with machinery awl already ships (the HUD's press/release pair + the hold-⌘ peek's single-`WaitUntil` timer). Tap ⌘I = Italic (fires on release, ~100–200 ms perceived latency), hold past ~250–300 ms = the stats HUD as today. The honest cost is a third dispatch semantics special-cased around one chord — but the HUD is *already* that special case, so this extends an existing exception rather than minting a new class. Recommended, with the threshold flagged as a live taste call.

4. **⌘E stays Inline code.** Cocoa's ⌘E (use-selection-for-find, the system find pasteboard) is the *older* idiom, but the modern writer cluster (Craft, Notion, Bear) spent ⌘E on code, awl advertised it this morning, and the find idiom's spirit is recoverable without the chord: **prefill the find field from the active selection when ⌘F opens** (the Xcode behavior). Keep the chord; add the prefill.

5. **Reserve ⌘K for insert/edit link** (Bear/Craft/Notion/Things/Ulysses — the single strongest writer-cluster chord awl doesn't use). Links v2 is banked; don't spend ⌘K on anything else in the meantime.

6. **Refuse the rest on principle:** ⌘D (no stable meaning anywhere), ⌘U (markdown has no underline — the chord would teach a lie), ⌘L (a developer idiom, not a writer one), any new Option-letter chord (the typographer's layer stays retired), ⇧⌘S save-as (autosave + local history are awl's model; save-as is a foreign document concept).

---

## 2. Current state — the native slot as actually wired (verified in `keymap.rs` / `commands.rs`)

### Chords that fire today

| Chord | Command | Idiom verdict |
|---|---|---|
| ⌘S | Save | ✓ universal |
| ⌘Q | Quit | ✓ universal |
| ⌘Z / ⇧⌘Z | Undo / Redo | ✓ universal |
| ⌘C / ⌘X / ⌘V / ⌘A | Copy / Cut / Paste / Select all | ✓ universal |
| ⌘F / ⇧⌘F | Search forward / backward (in-panel: next / previous) | ✓ ⌘F universal; ⇧⌘F is awl's own (Cocoa uses ⌘G/⇧⌘G for stepping — see P2) |
| ⌘R (+ ⌥⌘F legacy) | Find and replace | ✓ ⌥⌘F **is** the Pages/Xcode idiom — already satisfied; ⌘R is awl's headline door |
| ⌘O | Go to file | ~ (macOS "Open…"; awl's quick-open reading is the editor-cluster norm — settled this morning) |
| ⇧⌘P / ⌘P | Switch project / Palette | ✓ editor-cluster (VS Code/Sublime lineage) |
| ⌘N | New note | ✓ universal (new-document family) |
| ⌘T | Switch theme | ⚠ Cocoa text apps use ⌘T = show Fonts panel. Defensible override: awl has no fonts panel; theme IS its "how it looks" surface. Leave, logged (§6-A5). |
| ⌘B / ⌘E | Bold / Inline code | ✓ ⌘B universal; ⌘E argued in §5-W2 |
| ⌘I (held) | Stats HUD | The platform's Italic chord — §4 is this question |
| ⌃Tab | Last file | ✓ editor/browser cluster (2-deep toggle; ⌃⇧Tab reverse is meaningless at depth 2 — fine) |
| ⌘; | Spell suggestions | ~ near-idiom (Cocoa ⌘; = jump to next misspelling; awl's suggest-at-caret is close kin). Leave. |
| ⇧⌘H | Version history | free territory, no macOS convention exists (Pages buries it in File ▸ Revert To). Fine. |
| ⇧⌘O | Toggle outline | free territory. Fine. |
| ⇧⌘. | Toggle hidden files | ✓ the Finder chord, correctly borrowed |
| ⌘= / ⌘- / ⌘0 | Zoom in / out / reset | ✓ universal |
| ⌘←/→, ⌘↑/↓ | Line start/end, doc start/end | ✓ verified firing (`resolve_named` + tests) |
| ⌥←/→, ⌥⌫, ⌥⌦, ⌘⌫ | Word motion, word deletes, delete-to-line-start | ✓ the full macOS deletion/motion set, verified firing |
| Home/End, PgUp/PgDn | Line start/end, page move (caret moves) | ⚠ deepest Cocoa behavior scrolls *without* moving the caret; every real editor moves it. Accepted divergence — leave (§5-C4). |
| bare-⌘ held 600 ms | Shortcut peek | awl's own; no conflict (cancels on any second key) |
| C-f/b/n/p/a/e, C-d, C-k, C-y, C-v, C-w, C-s, C-r, C-/, C-g, C-Space | quiet slot-2 survivors | ✓ notably: most of these are ALSO Cocoa standard bindings (C-f/b/n/p/a/e/d/k/y/v come free in every NSTextView) — awl's emacs flavor and the platform agree here |
| C-x / C-c | bare prefixes (defaults emptied, machinery kept) | per identity round |

### Free ⌘ real estate
⌘D · ⌘G · ⌘H (plain) · ⌘J · ⌘K · ⌘L · ⌘M (OS, via menu) · ⌘U · ⌘W · ⌘Y · ⌘, · ⌘. · ⌘' · ⌘[ ] \ / ` · ⌘1–9 — all currently **self-insert their character** (the §3-P0 bug).

---

## 3. Proposals — ranked by idiom strength

### Tier 0 — substrate (bug-grade, ship before anything)

**P0 · The unbound-⌘ swallow guard.** An unhandled Super chord must resolve to `Ignore`, never `InsertChar`. Evidence: `keymap.rs` test `cmd_shift_h_opens_history` pins `km.resolve(&ch("h"), &sup()) == Action::InsertChar('h')` as *intended*; `resolve_char` never checks `sup`. Every unassigned ⌘-letter/symbol types into the manuscript — a correctness hole in editing, exactly where awl's principles say to spend. **What moves:** one guard in `resolve_char` (+ the pinned test flips). **Conflict:** none — no user-visible chord changes; a `[keys]` Super rebind still wins (the override map is consulted before dispatch). **Recommendation: do it now, independent of every other decision.**

### Tier 1 — universal idioms (every Mac app; HIG-level)

**P1 · ⌘, → Settings…** THE preferences chord since Mac OS X 10.1. awl has the faceted Settings menu (`OpenSettingsMenu`) reachable only by name. **Binding:** `Cmd-,` native slot on "Settings…". **Also:** an App-menu "Settings…" item above the separator (routed, accelerator `None` per the settled accelerator decision — the chord fires through the keymap). **Conflict:** none (`,` free under Super). **Recommendation: bind now — the highest-value single line in this report.**

**P2 · ⌘G / ⇧⌘G → find next / previous.** TextEdit, Safari, Notes, Pages, Xcode, BBEdit — the stepping idiom is ⌘G, not ⌘F-again. Today stepping exists only inside the open panel (`handle_search_key`: in-panel ⌘F/⇧⌘F), and `start_search` always constructs an empty query — nothing survives Enter/Esc, so there is nothing for a bare ⌘G to repeat. **Binding:** `Cmd-G` → `SearchForward`, `Cmd-S-g` → `SearchBackward` (the actions already mean "advance while searching"), plus the same aliases inside `handle_search_key`. **To be fully idiomatic** it wants a remembered last query so ⌘G *after* the panel closes re-finds (small state on the App, flagged as the real work). **Conflict:** none (`g` free under Super; C-g cancel untouched). **Recommendation: bind now; last-query memory as the fast-follow that makes it honest.**

**P3 · App menu Hide / Hide Others / Show All (⌘H / ⌥⌘H).** The standard App-menu block is absent (`menu.rs` `APP_ITEMS` = About Awl · separator · Quit Awl). These are genuinely OS window-manager commands with no app state — the exact boundary `menu.rs` already draws for predefined Minimize/Zoom, and muda ships all three predefined. AppKit's key-equivalent interception then gives ⌘H/⌥⌘H for free (and stops ⌘H typing 'h' even before P0 lands). **What moves:** roster only — no keymap motion, no catalog entry. **Recommendation: add; this is menu completeness, not a rebind.**

**P4 · ⌘. → Cancel.** The HIG's ancient cancel synonym (⌘-period predates Esc on the Mac); every dialog honors it. `Action::Cancel` exists; `.` is free under plain ⌘ (⇧⌘. is hidden files — distinct chord, no clash). **Recommendation: bind quietly — no advertising needed, it's the kind of chord a Mac hand tries without thinking.**

**P5 · ⌘W → Finish file.** The close-the-document idiom. awl has no "close," but `FinishBuffer` — save → notify any daemon `--wait` client → switch to the previous file — *is* awl's "I'm done with this document," and it's non-destructive under stray muscle memory (it saves; with no other file it's a calm near-no-op). The identity round left Finish file palette-only; ⌘W is its natural native slot. **Risk:** a user expecting window-close gets a file-switch — but awl is single-window and Quit is ⌘Q, so the surprise is small and safe. **Conflict:** none (`w` free; C-w cut untouched). **Recommendation: bind — the strongest awl-specific mapping in this report; genuine user call (§7 Q2).**

### Tier 2 — the ⌘I question (§4, below)

### Tier 3 — writer-tool cluster idioms

**W1 · ⌘K = insert/edit link — RESERVE.** Bear, Craft, Notion, Things, Ulysses, Slack — the most uniform writer-cluster chord that exists. awl has no link-insert yet (Links v2 + paste-URL-over-selection are banked). **Recommendation: reserve ⌘K now — spend it on nothing else — and bind it the day Links v2 lands.** (Follow link stays ⌘-click / C-c C-o; ⌘K is *authoring*, not following.)

**W2 · ⌘E stays Inline code — argued, not assumed.** The honest conflict: Cocoa's ⌘E is "use selection for find" (`NSFindPboard`, system-wide — set in one app, ⌘G in another). That is the deeper OS idiom and losing it is a real cost to Mac hands. Against it: the modern writing cluster (Craft, Notion, Bear) settled ⌘E = code; awl advertised ⌘E = Inline code this morning; and awl's search is a summoned incremental panel, not a find-pasteboard client — the idiom wouldn't compose system-wide anyway. **The spirit is recoverable chord-free: when ⌘F opens with an active selection, prefill the query from it** (Xcode's behavior; `start_search` currently always starts empty). **Recommendation: keep ⌘E = Inline code; add prefill-from-selection as a fast-follow (§7 Q5).**

**W3 · ⇧⌘L → Task list.** Apple Notes' checklist chord — the one *Apple-native* anchor for any block toggle (the rest of the block family genuinely has no convention, as the identity round concluded). awl's audience (writers, notes) overlaps Notes' exactly. **Conflict:** none (`l` free under ⌘ and ⇧⌘). **Tension:** the identity round settled block toggles as palette-only *this morning* — motion this soon needs the idiom to be strong, and one-app-strong is the honest rating. **Recommendation: lean bind, but it's a genuine user call (§7 Q3); leaving it palette-only is defensible.**

**W4 · Heading cycling ⌘⌥1–6 — bank.** Bear/Craft/Notion/Ulysses set heading *levels*; awl's `ToggleHeading` is one-level (`# ` on/off) — the actions ⌘⌥N would need don't exist, and inventing them is a feature round, not a rebind. **Recommendation: bank with a future heading-levels round; don't pre-spend the chords.** (Same verdict for a quote-toggle chord: no cross-app convention; palette is right.)

### Tier 4 — Cocoa text-system completeness (quiet, slot-2-class)

**C1 · C-h → delete backward.** A Cocoa *standard* binding (every NSTextView) awl misses; the action exists; the chord is free (awl has no help system for emacs's C-h to claim). Zero cost, tiny value. **Recommendation: bind quietly if the bare-control set isn't considered frozen (§7 Q8).**


**C2 · C-l → recenter.** The one chord where Cocoa (`centerSelectionInVisibleRect:`) and emacs (`recenter`) literally agree. Needs a new one-shot action (distinct from the typewriter-scroll *mode*). **Recommendation: bank — worth it eventually precisely because both traditions share it.**

**C3 · C-t (transpose) / C-o (open line) — bank.** Cocoa + emacs both, but each needs a new editing action; lowest value. Log, don't build.

**C4 · Home/End/PgUp/PgDn caret behavior — leave.** The deepest Cocoa idiom scrolls without moving the insertion point; awl moves the caret (the universal *editor* behavior, and the more useful one). Accepted divergence, now logged instead of latent.

**C5 · ⌃⌘Space emoji — verify live, likely already satisfied.** The system Character Viewer is input-method-level; insertion rides winit's NSTextInputClient → `Ime::Commit` → `handle_ime` (wired ✓). AppKit auto-appends "Emoji & Symbols" to a menu literally titled "Edit" (awl has one). **No action; flag for the next live session's checklist.** Same checklist item: ⌃⌘F Enter Full Screen (AppKit auto-item) and ⌘M (already carried by muda's predefined Minimize accelerator — satisfied).

---

## 4. The ⌘I question — italic vs the held HUD, evaluated honestly

⌘I = Italic is HIG-level (TextEdit, Notes, Pages, Mail, Bear, iA — everything). awl deliberately spent it on the hold-to-peek stats HUD, leaving Italic palette-only. awl now ships tap-vs-hold discrimination culture: the HUD's press/release pair (`hud_key` / `on_key_release` / `hud_release_on_mods`) and the hold-⌘ peek's pure `PeekArm` + single-`WaitUntil` timer (`HOLD_PEEK_MS = 600`).

**Option A — tap-⌘I = Italic, hold-⌘I = HUD.** Press arms a threshold (~250–300 ms, its own constant, NOT the peek's 600 ms — a deliberate two-key chord is already a commitment); crossing it summons the HUD exactly as today (held until release); releasing under it fires Italic on the release. Feasible with existing seams: the arm/timer is `PeekArm`'s shape, the release door already exists, OS auto-repeat just re-affirms the hold.
*Costs, stated:* (1) **Italic fires on release** — ~100–200 ms perceived latency versus ⌘B's instant press; the B/I asymmetry is real but small (a formatting toggle, not a typed character). (2) **The HUD loses its instant summon** — delayed by the threshold; tolerable for a peek (the bare-⌘ peek already waits 600 ms), but the current pop-on-press snap is part of its feel — live taste call. (3) **A third dispatch semantics** hard-coded around one chord — the two-slot model and the rebind capture have no vocabulary for tap-vs-hold; mitigated: the HUD is *already* the one keymap special case (hold-only, excluded from the catalog), so this extends an existing exception, not a new class. (4) Headless: `--hud` and `--keys "Cmd-I"` keep replay-press-equals-hold semantics; Italic stays drivable by name — determinism unaffected.

**Option B — move the HUD (⌥⌘I or ⇧⌘I "info"), plain ⌘I = Italic.** Simpler, zero latency, perfectly idiomatic — and it costs moving a chord the identity round advertised *today*, on a signature awl affordance whose mnemonic ("i for info") is the same letter.

**Option C — leave it.** Italic stays palette-only; ⌘I stays the HUD. The status quo the identity round chose, still coherent.

**Recommendation: A.** It is the only option that keeps both identities, the machinery is genuinely in place, and Bold/Italic is the most-reached writer pair in existence — awl advertising ⌘B but not ⌘I reads as a gap to every writer who tries it. Flag `CARET`-style taste constants (threshold ms) for live tuning; the release-latency feel is live-only and must not be claimed verified. Final call is the user's (§7 Q1).

---

## 5. Conflicts & costs — the whole motion table

| Proposal | Binding | Collides with | What breaks | Verdict |
|---|---|---|---|---|
| P0 swallow guard | — | one pinned test | nothing user-visible; `[keys]` Super rebinds still win | **do now** |
| P1 Settings | ⌘, | nothing | nothing | **do now** |
| P2 find next/prev | ⌘G / ⇧⌘G | nothing | nothing; wants last-query memory to be honest | **do now** (+fast-follow) |
| P3 Hide block | ⌘H / ⌥⌘H (via NSMenu) | nothing (plain ⌘H unbound) | nothing — roster-only | **do now** |
| P4 cancel | ⌘. | nothing (⇧⌘. distinct) | nothing | **do** |
| P5 finish file | ⌘W | nothing | muscle-memory surprise: saves + switches, never destroys | **do** (user call) |
| ⌘I tap/hold | ⌘I | the HUD's instant summon | Italic on-release latency; HUD delayed ~250 ms; one more special case | **recommend** (user call) |
| W1 link | ⌘K | nothing | nothing — reserved until Links v2 | **reserve** |
| W2 code | ⌘E (keep) | Cocoa find-pasteboard idiom | recovered via ⌘F prefill-from-selection | **keep** |
| W3 task list | ⇧⌘L | nothing | contradicts this morning's "block toggles palette-only" — one-app idiom (Notes) | **lean bind** (user call) |
| W4 headings | ⌘⌥1–6 | nothing | needs nonexistent level actions | **bank** |
| C1 C-h | C-h | nothing | none | optional |
| C2/C3 C-l/C-t/C-o | — | — | need new actions | **bank** |

Nothing here needs a third slot, steals an existing chord, or touches the retired layers. All new bindings fill empty native slots and stay `[keys]`-rebindable; every one remains reachable by palette name regardless.

---

## 6. Anti-recommendations — idioms awl should refuse

- **A1 · ⌘D — refuse.** No stable meaning exists (Duplicate in Finder, Bookmark in Safari, Don't Save in dialogs, Delete in Mail, define-in-dictionary in some text views). A chord that means five things means nothing; awl has nothing it honestly names. The dictionary-lookup reading is the only writer-adjacent one, and macOS already gives ⌃⌘D system-wide, three-finger-tap included — for free, everywhere. Don't compete.
- **A2 · ⌘U underline — refuse.** Markdown has no underline; the file is plain text and the render never lies about it. Binding ⌘U to anything else would teach a Mac hand a falsehood on a formatting chord. The empty chord IS the correct behavior (once P0 makes it calmly inert).
- **A3 · ⌘L go-to-line — refuse for now.** BBEdit/Xcode developer idiom, not a writer one; awl has no go-to-line at all, deliberately. If line-jumping ever earns a door it should be a `:42` syntax inside ⌘O's territory, not a chord (chords buy territories, not commands).
- **A4 · Any new Option-letter chord — permanently refused.** The identity round's own law: Option is the typographer's layer (é, ñ, —, •). Word ops on ⌥-arrows/⌫ are the platform's own carve-out; nothing else joins them.
- **A5 · ⌘T Fonts panel — the override stands.** Cocoa text apps mean "show Fonts" by ⌘T; awl means Switch theme. A themed editor with no fonts panel is entitled to this: theme is awl's entire "how it looks" surface. Logged so nobody re-litigates it.
- **A6 · ⇧⌘S Save As / Duplicate — refuse v1.** Autosave + local history + Keep version are awl's model; save-as drags in a foreign document lifecycle. Move note… is a different concept (location, not identity) and stays palette-only.
- **A7 · ⇧⌘V paste-and-match-style — satisfied by construction.** awl pastes plain text always; the idiom's purpose is already the only behavior. Never bind it to anything else.
- **A8 · No format-bar chord sprawl.** Highlight, Strikethrough, Blockquote, the list family (⇧⌘L excepted, Q3) stay palette-only. Eleven formatting chords is a toolbar wearing a keyboard costume; the palette + two-or-three universal chords is the calm answer.
- **A9 · ⌘J, ⌘Y, ⌘' — leave free.** No idiom worth the real estate; free keys are a feature (they're the user's `[keys]` space).

---

## 7. Open questions for you to decide

1. **The ⌘I call.** (A) tap-⌘I = Italic / hold = HUD (recommended; costs on-release latency + a ~250–300 ms HUD delay, threshold live-tuned); (B) HUD moves to ⌥⌘I or ⇧⌘I and plain ⌘I = Italic instantly; (C) leave as-is (Italic palette-only). Which?
2. **⌘W → Finish file** — bind now (recommended), or leave ⌘W inert until awl has a real "close" concept?
3. **⇧⌘L → Task list** — take the Apple Notes checklist idiom (lean bind), or hold this morning's "block toggles are palette-only" line?
4. **⌘G depth** — ship in-panel-only aliases first (cheap, weaker), or with the remembered-last-query re-find (true idiom, small new state)? Recommend the latter.
5. **⌘F prefill-from-selection** — adopt the Xcode behavior as the compensation for keeping ⌘E = Inline code? (Recommend yes.)
6. **Menu roster additions** — App menu gains Settings… (routed) and the predefined Hide / Hide Others / Show All block? (Recommend yes; roster-only, no keymap motion.)
7. **Swallow-guard scope** — confirm ALL unhandled ⌘ chords go inert (P0), with `[keys]` Super rebinds still winning. Any exception you want preserved?
8. **The bare-control set** — is it frozen as-shipped, or may C-h = delete-backward join quietly (the one Cocoa-standard control binding awl misses that needs zero new code)?

---

## 8. Self-check against awl's laws

**Two-slot cap:** every proposal fills an *empty* native slot; no command gains a third chord; all stay `[keys]`-rebindable and palette-reachable. **One accent / calm:** zero new chrome, zero nudges — P1–P5 are invisible until the user's own hands ask; the only surface change is three standard menu rows. **Summoned, not furniture:** nothing persistent added; the HUD/peek hold-semantics stay holds. **Chords buy territories:** ⌘G extends the ⌘F find territory, ⌘, opens the Settings territory, ⌘K is reserved for the link territory — no one-off sprawl; the format family stays palette-only save the universal pair (+⌘I pending Q1). **Retired layers stay retired:** no M-letter, no C-x default, no Space leader, no modes; the prefix machinery is untouched. **Determinism:** every proposed binding is `--keys`-drivable and catalog-visible except the tap-vs-hold discrimination, which — like the HUD and peek it extends — renders its settled state in capture and is honestly flagged live-only. **Untested behavior doesn't exist:** each accepted proposal's landing must extend the agreement sweep (`catalog_and_keymap_agree_on_every_default_chord`), the pairwise-conflict sweep, and P0's flipped self-insert law test.

*Nothing in this report was implemented. The tree was read, not touched.*
