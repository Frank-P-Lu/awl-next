# GUIDE

This page documents how awl's pieces fit together. The command palette
({{key:command_palette}}) already lists every command by name; this page
covers the model underneath.

---

## Where your words live

**A scratch buffer, always open.** Launch awl with no file and you land
on a writing surface — no save dialog, no "untitled-1.md". It stashes
itself to disk on the same rhythm as everything else: idle, window
blur, buffer switch, quit (`$XDG_DATA_HOME/awl/scratch.md`, or
`~/.local/share/awl/scratch.md` if `XDG_DATA_HOME` isn't set — on the
web build, `localStorage`). Relaunch bare and the scratch buffer is
where you left it, including parts you never explicitly saved.

**Quick notes ({{key:new_note}}) work the same way, with a home.**
{{key:new_note}} jumps to `notes_root` (`~/notes` by default,
configurable) and opens a fresh note buffer. Nothing writes to disk
until you type something.

**Autosave runs on four triggers:** idle (about a second after you
stop typing), window blur, buffer switch, quit. Writes are atomic (a
temp file, then a rename) and never clobber an external edit — if the
file changed on disk since awl last touched it, the write is held and
a notice appears at the bottom of the screen. Editing again re-arms
it. A manual {{key:save}} always force-writes.

**A small dot (•) in the window title marks an unsaved buffer**,
clearing the instant it's written. The stats HUD ({{key:stats_hud}})
shows a **SAVED** row too — "just now", "3m ago", "unsaved changes."

**Local history keeps more than the current text.** Every save of a
file not under git records a snapshot, pruned by an aged retention
ladder: everything from the last ~15 minutes, then one per writing
session, then one per day, then one per week — never a flat FIFO
cutoff. {{key:version_history}} opens the timeline for the current
file; Enter on any entry restores it as one ordinary undoable edit.
"Keep version" ({{key:command_palette}}) pins a snapshot the retention
ladder will never prune. A file under git skips awl's own history
entirely — `git log` is that file's timeline.

**A corrupted store never eats your data.** Session state, usage
stats, recent-projects, the history log, the scratch stash — if any of
these is unreadable (garbled, not just missing) when awl tries to load
it, the bad file is copied aside first (`<name>.corrupt-<timestamp>`)
before awl falls back to a clean default. Nothing is silently
discarded. (`config.toml` is the one exception — a syntax error there
keeps your last-known-good settings and shows a notice, since your
editor buffer and undo history already hold your intended text.)

## The notes model

A note starts as an ordinary scratch buffer. Save it — or keep typing
and let autosave catch up — and awl slugifies the **first line** into
a filename and writes it under `notes_root`. Change the first line and
the file on disk renames to match, until you save it under a different
name on purpose.

Three verbs live in the palette once a note exists:

- **Rename note…** — pick a new name, breaking the
  first-line-tracks-filename link.
- **Duplicate note** — an immediate copy, no dialog.
- **Move note…** — file it elsewhere under (or out of) your notes
  tree.

None carry a default chord — {{key:command_palette}}, type "rename", Enter.

## Keys

Every command has up to two bindings — **slot 1 is native** (⌘ on
macOS, Ctrl on Linux) and is the one awl teaches; **slot 2 is Emacs**,
a second layer that never goes away. Both fire. The palette
({{key:command_palette}}) shows both next to each command's name.

**Linux gets the same commands under Ctrl.** Where a native Ctrl chord
collides with a bare-control Emacs default (Ctrl-S save vs. Emacs
`C-s` search), the native one wins and the Emacs default steps aside —
still one `[keys]` line away. Set `keymap = "emacs"` in the config to
bring back the whole displaced cluster at once, instead of naming
chords one at a time. The **Omarchy/Hyprland recipe** (that compositor
forwards Super+C/X/V as Ctrl+C/X/V for the system clipboard):
`keymap = "emacs"` plus `[keys] copy = "C-c"`, `cut = "C-x"`,
`paste = "C-v"` keeps those three chords native under the emacs
preset.

**Rebind anything.** `[keys]` in the config maps a command's slugified
name to a chord, or up to two. Example — restoring the Option-letter
word motions the platform rule retired by default (macOS reserves
Option-letters for accented characters):

```toml
[keys]
forward_word  = ["M-Right", "M-f"]
backward_word = ["M-Left", "M-b"]
```

Or capture the key directly: {{key:command_palette}} → "Keybindings…"
opens a picker over every command; Enter starts a capture, and the
next key or chord becomes the new binding, written into your config.

**The hold-⌘ peek.** Hold the arming modifier alone for a beat (⌘ on
Mac, Ctrl on Linux) and a card of frequently-used shortcuts appears.
Release the hold and the card is gone — no click, no dismiss.

Generated from the live command catalog — never hand-edited (see the
law test `guide::tests::generated_keys_reference_matches_catalog`;
regenerate with `cargo test --bin awl guide::tests::print_generated_keys_reference
-- --ignored --nocapture` and paste the printed table between the
markers below, byte for byte).

<!-- GENERATED:keys-reference:BEGIN -->
| Command | macOS | Linux |
|---|---|---|
| Go to file… | ⌘O | Ctrl+O |
| Switch project… | ⌘⇧P | Ctrl+Shift+P |
| Recent projects… |  |  |
| Browse files… |  |  |
| Go to heading… |  |  |
| Spell suggestions… | ⌘; | Ctrl+; |
| Version history… | ⌘⇧H | Ctrl+Shift+H |
| Clean unused assets… |  |  |
| Keep version |  |  |
| Last file | ⌃Tab | Ctrl+Tab |
| New note | ⌘N | Ctrl+N |
| Move note… |  |  |
| Rename note… |  |  |
| Duplicate note |  |  |
| Finish file | ⌘W | Ctrl+W |
| Follow link | C-c C-o |  |
| Switch theme… | ⌘T | Ctrl+T |
| Caret style… |  |  |
| Dictionary… |  |  |
| Toggle spellcheck |  |  |
| Toggle hidden files | ⌘⇧. | Ctrl+Shift+. |
| Toggle caret style |  |  |
| Toggle page mode |  |  |
| Toggle writing nits |  |  |
| Widen page |  |  |
| Narrow page |  |  |
| Reset page width |  |  |
| Toggle debug |  |  |
| Toggle outline | ⌘⇧O | Ctrl+Shift+O |
| Toggle typewriter scroll |  |  |
| Toggle menu bar |  |  |
| About |  |  |
| Credits |  |  |
| Guide |  |  |
| Lifetime stats |  |  |
| Line endings… |  |  |
| Align table |  |  |
| Report a Problem |  |  |
| Download file |  |  |
| Check for Updates |  |  |
| Blockquote |  |  |
| Bullet list |  |  |
| Numbered list |  |  |
| Task list | ⌘⇧L | Ctrl+Shift+L |
| Heading |  |  |
| Code block |  |  |
| Bold | ⌘B | Ctrl+B |
| Italic | ⌘I | Ctrl+I |
| Inline code | ⌘E | Ctrl+E |
| Highlight |  |  |
| Strikethrough |  |  |
| Export as Word… |  |  |
| Export as HTML… |  |  |
| Export as PDF… |  |  |
| Insert link… | ⌘K |  |
| Save | ⌘S | Ctrl+S |
| Quit | ⌘Q | Ctrl+Q |
| Search forward | ⌘F · C-s | Ctrl+F |
| Search backward | ⌘⇧F · C-r | Ctrl+Shift+F |
| Find and replace… | ⌘R | Ctrl+R |
| Undo | ⌘Z · C-/ | Ctrl+Z · C-/ |
| Redo | ⌘⇧Z | Ctrl+Shift+Z |
| Copy | ⌘C | Ctrl+C |
| Cut | ⌘X · C-w | Ctrl+X |
| Paste | ⌘V · C-y | Ctrl+V · C-y |
| Select all | ⌘A | Ctrl+A |
| Zoom in | ⌘= | Ctrl+= |
| Zoom out | ⌘- | Ctrl+- |
| Reset zoom | ⌘0 | Ctrl+0 |
| Forward word | ⌥Right | Alt+Right |
| Backward word | ⌥Left | Alt+Left |
| Line start | ⌘Left · C-a | Home |
| Line end | ⌘Right · C-e | End |
| Document start | ⌘Up | Ctrl+Home |
| Document end | ⌘Down | Ctrl+End |
| Forward char | C-f |  |
| Backward char | C-b |  |
| Next line | C-n |  |
| Previous line | C-p |  |
| Settings… | ⌘, | Ctrl+, |
| Keybindings… |  |  |
<!-- GENERATED:keys-reference:END -->

PDF export is available in the native app only; the browser continues to
offer Word and HTML export.

## Looks

**Sixteen worlds, one chord away.** {{key:switch_theme}} opens the
theme picker — each world pairs its own display face with its own ink
ladder. Wagtail is the exception: awl's one monochrome world, drawn in
black, white, and nothing between.

**Two page widths, one for prose, one for code.** The writing column
measures 70 characters by default for prose and 100 for code
(rustfmt's own convention) — independent settings; widening one never
touches the other. Drag the column's edge, or use "Widen page" /
"Narrow page" / "Reset page width" in the palette.

**WYSIWYG, reveal-on-caret.** Markdown markup — a heading's `#`,
`**bold**`, `` `code` ``, `==highlight==`, a fenced code block's fence
lines — renders concealed except on the line your caret is on, where
it shows in full for editing. The file on disk is always plain
markdown; only the render is rich. `wysiwyg = false` disables the
conceal entirely.

**Reduce Motion is a real accessibility preference, not a cosmetic
toggle.** Absent config means `auto`: awl reads the OS-level "Reduce
Motion" setting where one is reachable (macOS, the web build) and
follows it. Set `reduce_motion = true` by hand on Linux, where there's
no reliable cross-desktop signal yet.

## The config file

Settings live in a plain text file, edited inside awl:
{{key:command_palette}} → "Settings" opens `config.toml` into the
buffer (writing the commented starter template first, if none exists).
Edit it like any other document, then save — the keymap, folders, and
every sticky preference re-apply live, no restart. A config with a
syntax error keeps prior values in place and shows a notice.

An absent config is just today's defaults. Once you touch it, it
remembers: theme, zoom, page widths, caret style, dictionary, and a
dozen other toggles persist across launches the moment you change them
live, and every key is hand-editable too.

## Awl in the browser

The web build is the same editor compiled to `wasm32-unknown-unknown`,
running in a `<canvas>` with no native filesystem underneath it.

| | Desktop (macOS / Linux) | Browser |
|---|---|---|
| Storage | Real files on disk | `localStorage`, capped around 5 MB — roughly eight to ten novels of plain text; scoped to this browser profile, gone if site data is cleared |
| Preferences, `[keys]` | `~/.config/awl/config.toml` | A `config.toml` over `localStorage`, same format, persists across reloads |
| Copy | To the OS clipboard | Mirrors out to the OS clipboard (best-effort, async) |
| Paste | From the OS clipboard | From awl's own kill ring only — an external copy doesn't appear until you've copied something from awl at least once |
| Getting a file out | Already on disk | "Download file" ({{key:command_palette}}) — saves the active buffer as a plain-text download |

**Hidden on web:** Recent projects…, Version history…, Clean unused
assets…, Keep version, Finish file, Lifetime stats, Quit, Check for
Updates — daemon, session-restore, and local-version-history machinery
with nothing to attach to in a browser tab.

**A couple of native chords belong to the browser itself** (new tab,
new window, and similar). {{key:new_note}} and {{key:switch_theme}}
resolve to a working alternate chord on web automatically; every
command is also reachable by name through {{key:command_palette}}.

The desktop build has no storage cap, real OS clipboard paste, and the
commands above — see the project's releases page for macOS and Linux
downloads.
