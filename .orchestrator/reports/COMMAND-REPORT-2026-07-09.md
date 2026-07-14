# Morning Design Report — awl command naming, organization, and mini-panel reach

*Prepared overnight from four inventories (commands, settings, overlays/mini-panel, and the design-principles brief). Everything stays inside awl's contract: smaller-or-clearer, keyboard-first, two-slots-max, summoned-not-furniture, one-ink×one-size, amber untouched. This is a naming-and-organization pass — no command is added or cut.*

---

## 1. TL;DR

1. **The catalog is functionally sound; only its *surface* is inconsistent.** Of 58 commands, **23 are already fine**. The other **35** split cleanly into **12 pure casing fixes**, **12 pure `…`-suffix additions** (display-only), and **11 substantive renames** that fix a real confusion or vocabulary split.

2. **Adopt one naming law with two halves.** *App-state* toggles read **`Toggle <thing>`** (or **`Cycle <thing>`** when >2 states); *document-edit* toggles stay **bare nouns** (`Bold`, `Heading`) — the settled format-group convention. *Pickers* take a trailing **`…`**; everything is **sentence case**. This one law resolves all four axes the audit flagged (toggle-naming, casing, file/buffer split, style/mode drift).

3. **Kill the two real collisions.** `Outline` (the heading jump-picker) collides with `Toggle Outline` (the margin ToC) — rename the picker **`Go to heading…`**. `History` (the version picker) shadows the `Local history` setting — rename it **`Version history…`**. Both renames also make the names self-describing.

4. **Standardize on the `mg` word: buffer.** `Last file` and `Finish File` dispatch `LastBuffer`/`FinishBuffer` and the docs already say "buffer" — make the labels agree: **`Last buffer`**, **`Finish buffer`**.

5. **Grow the mini panel into three places it is misfiled.** Page width and zoom are blind *typed-digit* fields today — they should be **named-stop mini panels** that reflow the live document as you arrow. Line-ending conversion is a blind command — fold it into a preview-less **`Line endings…`** mini panel (the shape `Dictionary` already uses). `Dictionary` is the precedent; `Theme` is the rich sibling and needs no duplicate.

---

## 2. Full command inventory (as-is)

Grouped by functional area (the live catalog in `commands.rs` is a flat list). Chords, kinds, and behavior are verbatim from source.

### Navigation & files
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Go to file | | C-x C-f | picker | Fuzzy go-to over the active project's file index |
| Browse files | | C-x j | picker | One-level browse navigator for the active root |
| Switch project | | C-x p | picker | Switch-project overlay over the workspace children |
| Recent projects | | | picker | Flat MRU of recently switched-to project roots |
| Outline | | | picker | Fuzzy picker over the doc's headings; jumps to the chosen one |
| Last file | | C-x b | action | Toggle to the previously-opened buffer (2-deep history) |
| Follow link | | C-c C-o | action | Open the markdown link under the caret in the OS browser |

### Notes
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| New note | | C-x n | action | Jump to the notes project and open a fresh empty note |
| Move note | | C-x m | picker | Move-destination picker (folders under the notes root) |

### Buffer & session
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Finish File | | C-x # | action | Save buffer, notify daemon `--wait` clients, switch to previous |
| Save | Cmd-S | C-x C-s | action | Write the active buffer to disk (atomic) |
| Quit | | C-x C-c | action | Quit through the clean-shutdown path |

### History
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| History | Cmd-S-h | | picker | Local-history timeline; Enter restores a version |
| Keep This Version | | | action | Pin the current buffer state as a prune-exempt snapshot |

### Search & replace
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Search forward | Cmd-F | C-s | picker | Incremental search forward / advance to next match |
| Search backward | Cmd-S-f | C-r | picker | Incremental search backward / step to previous match |
| Find and replace | Cmd-R | | picker | Summon the search panel with the replace row revealed |
| Spell suggestions | Cmd-; | | picker | Spell-suggestion picker for the misspelled word at the caret |

### Theme & caret
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Switch theme | | C-x t | picker | Theme picker over the worlds with live preview |
| Caret style | | | picker | Caret-style picker (Block / Morph / I-beam), animated preview |
| Toggle caret mode | | C-x c | toggle | Toggle the caret look Block ↔ I-beam |

### Writing & language
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Dictionary | | | picker | Dictionary picker (English US / UK / Australia) |
| Toggle Spellcheck | | | toggle | Flip global spell-check (sticky, default ON) |
| Writing nits | | | toggle | Flip the mechanical-typo underline (sticky, default ON) |
| Convert Line Endings | | | value | Flip on-disk LF ↔ CRLF (document metadata, not undoable) |

### Page, zoom & view
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Toggle page mode | | C-x w | toggle | Centered measure-capped column vs edge-to-edge |
| Page wider | | C-x } | value | Widen the writing-column measure one step (sticky) |
| Page narrower | | C-x { | value | Narrow the measure one step (sticky) |
| Reset Page Width | | | value | Snap the measure to the buffer default; clear the override |
| Zoom in | Cmd-= | | value | Increase glyph zoom one step |
| Zoom out | Cmd-- | | value | Decrease glyph zoom one step |
| Reset zoom | Cmd-0 | | value | Reset glyph zoom to the default |
| Focus mode | | C-x d | toggle | Cycle focus dimming Off → Paragraph → Sentence |
| Toggle Debug | | C-x r | toggle | Toggle the dim top-left debug/perf panel (default OFF) |
| Toggle Outline | Cmd-S-o | | toggle | Toggle the persistent margin table-of-contents (default OFF) |
| Typewriter Scroll | | | toggle | Pin the caret row centered (sticky, default OFF) |
| Toggle Hidden Files | Cmd-S-. | | toggle | Reveal/hide dotfiles in an open picker |

### Markdown formatting
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Blockquote | | | action | Toggle-edit: prefix each line with `> ` |
| Bullet List | | | action | Toggle-edit: prefix each line with `- ` |
| Numbered List | | | action | Toggle-edit: prefix lines with `1. `, `2. ` … renumbered |
| Task List | | | action | Toggle-edit: prefix each line with `- [ ] ` |
| Heading | | | action | Toggle-edit: prefix line(s) with one `# ` |
| Code Block | | | action | Toggle-edit: wrap in ` ``` ` fences |
| Bold | Cmd-B | | action | Toggle-edit: wrap in `**…**` |
| Italic | | | action | Toggle-edit: wrap in `*…*` |
| Inline Code | Cmd-E | | action | Toggle-edit: wrap in `` `…` `` |
| Highlight | | | action | Toggle-edit: wrap in `==…==` |
| Strikethrough | | | action | Toggle-edit: wrap in `~~…~~` |
| Align Table | | | action | Re-pad the GFM table under the caret (one undoable edit) |

### Core editing
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Undo | Cmd-Z | C-/ | action | Undo the last edit group |
| Redo | Cmd-S-z | | action | Redo the last undone group |
| Copy | Cmd-C | M-w | action | Copy the region to the kill buffer / clipboard |
| Cut | Cmd-X | C-w | action | Cut the region to the kill buffer / clipboard |
| Paste | Cmd-V | C-y | action | Paste (yank) at the caret |
| Select all | Cmd-A | | action | Select the whole buffer |

### App & config
| Command | Native | Emacs | Kind | What it does |
|---|---|---|---|---|
| Settings | | | picker | Faceted settings menu (config-as-text behind a row) |
| Keybindings | | | picker | Game-style rebind menu (captures a key/chord per command) |
| About | | | other | Summoned About card (name, version, world, ornament) |

*(58 commands, all accounted for.)*

---

## 3. What is inconsistent

Four axes plus one cross-surface note, every one drawn from the inventory.

**A. State-toggles are named three different ways.** Six carry the prefix — `Toggle Spellcheck`, `Toggle Hidden Files`, `Toggle caret mode`, `Toggle page mode`, `Toggle Debug`, `Toggle Outline` — but three equivalent sticky/process-global toggles do **not**: `Writing nits` and `Typewriter Scroll` (plain on/off), and `Focus mode` (which actually *cycles* Off/Paragraph/Sentence via `CycleFocusMode`, so its label hides that it isn't even binary). A user scanning the palette can't tell from the name which entries flip a persistent state.

**B. The format group names its toggles the opposite way — and that's correct.** All eleven markdown commands are `Toggle*` actions yet read as bare nouns (`Bold`, `Bullet List`, `Heading`). This is *not* a bug: the design brief endorses those names. It means "Toggle" cannot be a blanket rule — an *app-state* flip and a *document edit* are genuinely different, and the naming must say which is which.

**C. Casing is unsystematic.** Sentence case (`Go to file`, `Reset zoom`, `Select all`) mixes freely with Title Case (`Keep This Version`, `Reset Page Width`, `Toggle Spellcheck`, `Convert Line Endings`, `Bullet List`, `Inline Code`, `Align Table`). Even *inside* the Toggle family it splits — `Toggle caret mode`/`Toggle page mode` lowercase their tails while `Toggle Debug`/`Toggle Outline` capitalize them — and the two reset commands disagree (`Reset zoom` vs `Reset Page Width`).

**D. Vocabulary drifts across paired features.**
- **file vs buffer:** `Last file` → `Action::LastBuffer`; `Finish File` → `Action::FinishBuffer` (whose own docs/tests call it "Finish Buffer"). The label fights the action.
- **style vs mode:** `Caret style` (picker) and `Toggle caret mode` (toggle) name one concept two ways.
- **hard name collisions:** `Outline` (jump picker, `OpenOutline`) vs `Toggle Outline` (margin ToC) — same word, two unrelated features. `History` (version picker) shadows the `Local history` setting.
- **adjective vs verb nudgers:** `Page wider`/`Page narrower` (adjective) against the parallel `Zoom in`/`Zoom out` (verb).

**E. Cross-surface: same feature, two labels.** The Settings menu and the palette disagree on wording for shared features — `Page mode` vs `Toggle page mode`, `Spellcheck` vs `Toggle Spellcheck`, `Theme` vs `Switch theme`, `Typewriter scroll` vs `Typewriter Scroll`. Most are *defensibly* different (a settings row is a noun with a value cell; a palette entry is an action) — but only once that split is made a rule instead of an accident. The one that is not defensible is the `Outline` collision, which is real ambiguity on both surfaces.

*(Two non-naming notes surfaced alongside: `Writing nits` is uniquely backed by the `Ignore` sentinel rather than a real `Action`; and `settings.rs:68` still comments "19-setting corpus" while the live count is 23. Both are code-hygiene items — parked in §8.)*

---

## 4. The proposed convention

Six rules. Each is grounded in a design principle, and each keeps the derived action-name (`display → lowercase → spaces = _`) addressable by `[keys]`, the menu routing table, and the rebind menu.

**Rule 1 — Sentence case, always.** Only the first word (and proper nouns) capitalized: `Bullet list`, `Inline code`, `Reset page width`, `Toggle spellcheck`, `Align table`. *(Ink and size carry emphasis, never capitals — DESIGN §4.)*

**Rule 2 — App-state toggles: `Toggle <thing>` (or `Cycle <thing>`).** A command that flips a **sticky / process-global setting that shows an on/off (or a value) in the Settings menu** is named `Toggle` + the setting's noun. A command that steps through **more than two** states is `Cycle` + the noun (honest about the affordance). This brings `Writing nits`, `Typewriter Scroll`, and `Focus mode` into the family.

**Rule 3 — Document-edit toggles: bare noun of the result.** A command that **inserts/removes markup as one undoable edit** keeps its bare-noun name — `Bold`, `Heading`, `Bullet list`, `Code block`. These are *edits* ("make this a heading"), phrased like `Copy`/`Cut`; you would never say "Toggle copy." This is the settled format-group convention. Rules 2 and 3 are the two halves of "toggle," disambiguated by *what* is toggled — app state vs document text.

**Rule 4 — Pickers end in `…`.** A command that **summons a list/menu you pick a row from** takes a trailing ellipsis: `Go to file…`, `Switch theme…`, `Settings…`, `Dictionary…` — the familiar platform idiom that an ellipsis means "opens a further choice," applied consistently here for the first time. *Search-mode entries stay bare* (`Search forward`, `Search backward`) — they type in place incrementally, they don't present a chooser. **Implementation gate:** the action-name derivation must strip a trailing `…` so `switch_theme` (not `switch_theme…`) stays the `[keys]`/menu-routing key — a one-line canonicalization plus a law-test assertion (see §8, Q1).

**Rule 5 — Verb-first for actions; noun for the setting it names.** Palette *commands* lead with a verb where one is natural (`Widen page`, `Reset page width`, `Switch theme…`, `Keep version`), matching `Zoom in`/`Reset zoom`. The **Settings menu row** for the same feature stays the **noun of the setting** plus its live value cell (`Page width`, `Theme`, `Spellcheck`). This makes the §3E disagreements *principled by surface* — a palette verb and a settings noun for one feature are correct, not sloppy — everywhere except a true collision.

**Rule 6 — One word per concept; no name shadows another.** Pick `style` over `mode` for the caret (`Caret style…`, `Toggle caret style`). Standardize on the `mg` word **buffer** where the object is genuinely a buffer (`Last buffer`, `Finish buffer`). And no two commands — nor a command and a setting — may share a bare name: `Outline` the picker becomes `Go to heading…`; `History` the picker becomes `Version history…`.

**On/off state is surfaced by value, not in the name.** As today: the *name* never says "on" — the Settings row shows the calm word `on`/`off` (or the value/selection) in its secondary column, read from the same owner the renderer reads. The palette shows only bindings. *(Amber stays the caret's; nothing here breathes — DESIGN §3.)*

---

## 5. The rename table

**35 of 58 commands change; 23 are already fine and are omitted.** The 23 untouched: New note · Follow link · Toggle page mode · About · Blockquote · Heading · Bold · Italic · Highlight · Strikethrough · Save · Quit · Search forward · Search backward · Undo · Redo · Copy · Cut · Paste · Select all · Zoom in · Zoom out · Reset zoom.

The 35 changes split into three footprints. Only the **11 substantive** renames alter meaning; the other 24 are mechanical.

### 5a. Substantive renames (11) — fix a real confusion or vocabulary split
| Current | Proposed | Why |
|---|---|---|
| Outline | **Go to heading…** | Removes the hard collision with `Toggle Outline`; names what it does, paralleling `Go to file…`. |
| History | **Version history…** | Stops shadowing the `Local history` setting; says it's the version timeline. |
| Last file | **Last buffer** | Label now matches `Action::LastBuffer` and the `mg` idiom (a buffer may be an unsaved scratch, not a file). |
| Finish File | **Finish buffer** | Matches `Action::FinishBuffer` and the docs/tests, which already say "Finish Buffer." |
| Toggle caret mode | **Toggle caret style** | One word for the concept — pairs with the `Caret style…` picker (Rule 6). |
| Writing nits | **Toggle writing nits** | A sticky on/off app-state toggle; joins its family (Rule 2). |
| Typewriter Scroll | **Toggle typewriter scroll** | Sticky on/off toggle; joins the family (Rule 2) + casing. |
| Focus mode | **Cycle focus mode** | Honest: it cycles Off/Paragraph/Sentence (`CycleFocusMode`), it doesn't toggle (Rule 2). |
| Page wider | **Widen page** | Verb-first, parallels `Zoom in` (Rule 5). |
| Page narrower | **Narrow page** | Verb-first, parallels `Zoom out` (Rule 5). |
| Convert Line Endings | **Line endings…** | Reframes a blind one-shot as a picker that shows the current LF/CRLF and lets you choose — and sets up the mini panel in §7. |

### 5b. Casing only (12) — Title Case → sentence case, meaning unchanged
| Current | Proposed |
|---|---|
| Keep This Version | **Keep version** *(also drops filler "This")* |
| Toggle Spellcheck | **Toggle spellcheck** |
| Toggle Hidden Files | **Toggle hidden files** |
| Toggle Debug | **Toggle debug** |
| Toggle Outline | **Toggle outline** |
| Reset Page Width | **Reset page width** |
| Align Table | **Align table** |
| Bullet List | **Bullet list** |
| Numbered List | **Numbered list** |
| Task List | **Task list** |
| Code Block | **Code block** |
| Inline Code | **Inline code** |

### 5c. Ellipsis only (12) — display suffix marking a picker; derived key unchanged
`Go to file…` · `Browse files…` · `Switch project…` · `Recent projects…` · `Move note…` · `Spell suggestions…` · `Switch theme…` · `Caret style…` · `Dictionary…` · `Find and replace…` · `Settings…` · `Keybindings…`

*(These twelve keep their exact action-name — `go_to_file`, `switch_theme`, … — under Rule 4's canonicalization; the `…` is display-only. `Find and replace…` is the one debatable member — see Q1.)*

---

## 6. Organization / taxonomy

### Palette lens facets
awl's Cmd-P palette is **already faceted** (`commands.rs` → `crate::facets`). Today it lenses on **All · File · Edit · View · Recent**, where File/Edit/View mirror the macOS menu bar's grouping (via `menu_section`) and **Recent** is a dynamic most-recently-run list. Everything outside File/Edit/View — every format, find, and app command — currently falls through to **All** only.

The proposal splits those out into three more static facets so nothing hides in the flat list, and leaves the dynamic **Recent** facet exactly as it is. This extends the single `menu_section` owner with three more section lists — the same table-driven shape it already uses — and no new chrome: it's a relabeling of an existing summoned, faceted picker (DESIGN §5).

| Facet | Commands |
|---|---|
| **All** | *(default — the full list)* |
| **File** | Go to file… · Browse files… · Switch project… · Recent projects… · New note · Move note… · Last buffer · Save · Finish buffer · Follow link · Quit |
| **Edit** | Undo · Redo · Cut · Copy · Paste · Select all · Align table |
| **Format** | Bold · Italic · Inline code · Highlight · Strikethrough · Heading · Blockquote · Bullet list · Numbered list · Task list · Code block |
| **Find** | Search forward · Search backward · Find and replace… · Go to heading… · Spell suggestions… |
| **View** | Toggle page mode · Widen page · Narrow page · Reset page width · Zoom in · Zoom out · Reset zoom · Toggle outline · Cycle focus mode · Toggle typewriter scroll · Caret style… · Toggle caret style · Switch theme… · Toggle debug |
| **App** | Settings… · Keybindings… · Version history… · Keep version · Toggle spellcheck · Toggle writing nits · Dictionary… · Line endings… · Toggle hidden files · About |

Plus the existing **Recent** (dynamic MRU) facet, unchanged. Every one of the 58 commands lands in exactly one static facet.

### Relationship to the Settings lenses
The palette groups **actions**; the Settings menu groups **configurable state** — its own lenses (**Editor · Appearance · Writing · Files · Keybindings · Advanced**) stay as they are. These are deliberately different cuts of one catalog, each tuned to its surface's job; they are *not* forced to share facet labels. The only discipline (Rule 5) is that a feature living on **both** surfaces never uses contradicting words:

- palette `Toggle page mode` / settings `Page mode`
- palette `Switch theme…` / settings `Theme`
- palette `Toggle spellcheck` / settings `Spellcheck`

Each pair is *the same feature, labeled by its surface's logic*, and no name shadows another (the `Outline` settings row becomes unambiguous the moment the picker is `Go to heading…`). The macOS menu bar's routing table (`menu::SECTIONS`) updates in lockstep for the renamed labels — the `every_routed_command_exists_in_the_catalog` law test enforces it.

---

## 7. Mini-panel expansion

awl's mini panel — the caret-style picker (`render/chrome/preview.rs`) — is **a short crisp list of an enumerable set + a floating live-preview card over an *undimmed* document, revert-on-cancel, commit-on-Enter.** It earns its place exactly where a setting is **small, enumerable, and consequential-in-view.** By that test, three settings are currently misfiled and one is already the pattern.

**1. Page width — the best fit, currently the worst-served.** `Page width (prose)` and `Page width (code)` are `SettingKind::Value` rows you edit by **typing digits** into an inline cell. That's backwards: nobody knows they want "72" — they know which reflow *looks* right, and the machinery to show it already exists (`App::sync_page_measure` re-wraps the instant the measure changes). Make it a mini panel over **named comfortable stops**, doc kept crisp so the column reflows as you arrow, the pre-open measure held for Esc-revert, Enter persisting to the class-matching key. `PageClass::of_syntax` already tells you which stop-set to show.
   - *Prose:* Narrow 60 · Comfort 66 · Wide 72 · Roomy 80
   - *Code:* 80 · 100 · 120

**2. Zoom — same story, same fix.** `Zoom` is also a typed `Value` row. `Cmd-±/0` step it, but there's no *named, bounded, previewable* door. Enumerate stops and live-preview them (the whole doc reshapes through the existing zoom/DPI `restyle_all_lines` path), revert on cancel. Total, instant visual consequence; a bounded ladder is friendlier than free digits.
   - *Stops:* 90% · 100% · 110% · 125% · 150%

**3. Line endings LF ↔ CRLF — a clean upgrade that doubles as the §5a rename.** `Convert Line Endings` is a blind palette flip today — you can't see which you're on at the moment of choosing. It's a textbook two-element set. Give it a **preview-less** mini panel (EOL is invisible in-document, so like `Dictionary` it *highlights, doesn't preview*), reusing the Dictionary shape verbatim. The HUD already reports `eol`, so the state is first-class — this just gives it a front door, and it *is* the `Line endings…` command from §5a.
   - *Rows:* **LF** — "Unix · macOS" · **CRLF** — "Windows"

**4. Dictionary — already the pattern; cite it as the precedent.** `Dictionary` (US / UK / AU + descriptions, highlight-not-preview) is *already* the caret-panel layout minus preview. No change — it's the proof that recommendations 1–3 have a settled shape to lean on.

**Considered and declined:**
- **Theme** already nails crisp-live-preview-revert across ~14 faceted worlds; a mini-panel duplicate would be redundant. *Optional, low priority:* a "recent worlds" quick-cycle over the two or three you actually live in — the stats layer already tallies per-world writing time (`stats::per_world_ms`) and surfaces the single top one as the HUD's "YOUR WORLD" row, so the ranking data exists — only if the full picker feels heavy in daily use.
- **The binary `Toggle`s** (page mode, WYSIWYG, spellcheck, writing nits, inline images, ligatures, outline) are a single on/off — a mini panel is overkill for two states; a toggle chord is the right weight. *(One nuance for §8: WYSIWYG and page mode have *large* visual payoff, so if anything they'd want a live-preview **toggle** — flip-and-watch, revert-on-release — a different affordance than the enumerable picker.)*
- **CJK priority** is a `SettingKind::List` — a *reorder* of four tags, not a pick-one; that's a list-editor, and it deliberately defers to config-as-text (`SettingKind::List` doc).
- **Notes root / Workspace / Project root** are unbounded `Path` rows already routing into the folder navigator — correct as-is.

Every one of these stays a **summoned overlay the mouse only points at** — no button, no bar, dismisses on Enter/Esc, crisp document so the preview can't lie (DESIGN §3/§5). The pattern *reduces* surface (a numeric field becomes a four-row pick), which is the discipline.

---

## 8. Open questions for you to decide

1. **`…` derivation gate.** Adding the ellipsis (§5c) is safe **only if** the action-name derivation strips a trailing `…` so `switch_theme` stays the `[keys]`/menu key. Confirm you want the ellipsis convention shipped with that one-line canonicalization + a law-test — and whether `Find and replace…` counts as a picker (it opens the replace *panel*, an input surface, not a chooser; I included it, but you may prefer it bare like the search family).

2. **buffer vs file.** I standardized on **buffer** (`Last buffer`, `Finish buffer`) — matches the Actions, the docs, and your `mg` idiom. Confirm you don't prefer "file" for approachability. (You're the audience of one and you know `mg`, so I lean buffer.)

3. **Keep vs Pin.** `Keep This Version` → I proposed **`Keep version`** (smallest honest fix). The semantics are "pinned, prune-exempt" — do you prefer **`Pin version`** (more precise) over the plainer "Keep"?

4. **The `Writing nits` sentinel.** It's uniquely backed by the `Ignore` sentinel (`WRITING_NITS_ACTION`), not a real `Action`. To make `Toggle writing nits` a proper rebindable sibling of the other toggles, it wants a real `ToggleWritingNits` action (the `ToggleTypewriter` action is the pattern to copy). Recommend yes — small, removes a special case. Confirm.

5. **Cycle vs keep-short for focus.** `Focus mode` → **`Cycle focus mode`** tells the truth (3 states) but is longer. Accept the honest name, or keep the shorter `Focus mode` and accept that the label hides the cycle?

6. **Settings-only render toggles.** WYSIWYG, inline images, and code ligatures have *no* palette/chord door (Settings is their sole surface). Subtraction says don't add commands for symmetry — but WYSIWYG and page mode have large visual payoff. Do you want a quick **live-preview toggle** affordance for those two (flip-and-watch, revert-on-release), or leave them settings-only?

7. **Retired `[keys]` keys.** Twelve derived keys change: the 11 substantive renames (`last_file`→`last_buffer`, `focus_mode`→`cycle_focus_mode`, `typewriter_scroll`→`toggle_typewriter_scroll`, `writing_nits`→`toggle_writing_nits`, `convert_line_endings`→`line_endings`, `outline`→`go_to_heading`, `history`→`version_history`, `toggle_caret_mode`→`toggle_caret_style`, `page_wider`→`widen_page`, `page_narrower`→`narrow_page`, `finish_file`→`finish_buffer`) **plus** `keep_this_version`→`keep_version` (the one casing-group entry that also drops a filler word). The 12 casing-only and 12 ellipsis-only renames leave their keys unchanged (lowercasing erases the case difference; Rule 4 strips the `…`). Any old line in *your* config goes **silently inert** (no crash — the lenient loader treats it as unknown), which you fix once by hand. Confirm that's acceptable rather than shipping alias-back-compat e thefor the old names.

8. **Code hygiene (tangential, worth a line).** `settings.rs:68` comments "The 19-setting corpus" and the `settings_table_names_are_unique` test (~line 354) accounts "= 21 rows"; the live corpus is **23** rows, and the test asserts uniqueness — not count — so the stale numbers never fail. Separately, `settings.rs:16` still says the Enter-to-edit / toggle / sub-picker interactions "are wired next phase," though `Value` (page width, zoom) and `Path` (notes root, workspace, project root) editing are live and working — only the CJK-priority `List` row genuinely punts to config-as-text (correctly documented at `SettingKind::List`). Worth folding these comment fixes into the same pass so the docs stop lying.

---

*One-line self-check: the proposal is smaller-or-clearer (35 renames, 0 added/cut, 3 typed fields reduced to picks), keyboard-first (no button, every entry still a chord or summoned row), two-slots-max (no chord touched — Cmd-I stays the HUD, Italic stays palette-only), summoned-not-furniture (no new chrome; the Recent MRU facet is preserved), one-ink×one-size with amber untouched — and every renamed name stays in sync across catalog, `[keys]`, the menu routing table, and the law tests, gated on the Rule 4 canonicalization.*
