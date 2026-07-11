# GUIDE

This is the guide, in awl's own voice — the model underneath the editor,
not an exhaustive option dump. The command palette (⌘P) already documents
every command by name; this page documents how the pieces fit together.

---

## Where your words live

**A scratch buffer, always open.** Launch awl with no file and you land on
a bare writing surface — no save dialog, no "untitled-1.md". Type. It
stashes itself to disk quietly (`$XDG_DATA_HOME/awl/scratch.md`, or
`~/.local/share/awl/scratch.md` if `XDG_DATA_HOME` isn't set — on the web
build it lives in the browser's `localStorage` instead) on the same
rhythm as everything else: idle, window blur, buffer switch, quit. Close
awl mid-thought and relaunch it bare — the scratch buffer is exactly
where you left it, even the parts you never explicitly saved.

**Quick notes (⌘N) work the same way, with a home.** ⌘N jumps you into
`notes_root` (`~/notes` by default, yours to change in the config) and
opens a fresh note buffer. Nothing is written to disk until you actually
type something — an empty note stays weightless.

**Autosave, quietly, on four triggers:** idle (about a second after you
stop typing), the window losing focus, switching to a different buffer,
and quitting. Writes are atomic (a temp file, then a rename — never a
half-written file on disk) and they never clobber an external edit: if
the file changed on disk since awl last touched it, the write is held
and a calm notice appears at the bottom of the screen instead of
silently overwriting someone else's change. Editing again re-arms it. A
manual ⌘S always force-writes, no questions asked.

**You can always tell if something's unsaved.** A small dot (•) appears
in the window title the moment a buffer goes dirty, and disappears the
instant it's written. Hold the stats HUD (Option-⌘I) for a plainer
readout — a **SAVED** row reading "just now", "3m ago", or "unsaved
changes."

**Local history remembers more than just the current text.** Every save
of a file NOT under git records a snapshot — pruned by an aged retention
ladder (everything from the last ~15 minutes, then one per writing
session, then one per day, then one per week) so recent work stays
granular and old work stays light, never a flat FIFO cutoff that discards
the file's oldest version. ⌘⇧H opens the timeline for the current file;
Enter on any entry restores it as one ordinary undoable edit. If you know
right now is a moment worth keeping forever, "Keep version" (⌘P) pins a
snapshot that the retention ladder will never prune, no matter how old it
gets. A file that IS under git skips awl's own history entirely — `git
log` is already that file's timeline, and awl doesn't duplicate it.

**A corrupted store never eats your data.** Session state, usage stats,
recent-projects, the history log, the scratch stash — if any of these is
found unreadable (not just missing — actually garbled) the moment awl
tries to load it, the bad file is copied aside first (`<name>.corrupt-<
timestamp>`) before awl falls back to a clean default. Nothing is ever
silently discarded; the evidence survives beside the fresh file. (Your
own `config.toml` is the one exception — a typo there just keeps your
last-known-good settings and shows a notice, since your editor buffer
and undo history already hold your intended text.)

## The notes model

A note starts life as an ordinary scratch buffer. The moment you save
it — or just keep typing and let autosave catch up — awl slugifies the
**first line** into a filename and writes it under `notes_root`. Keep
writing and change your mind about the opening line? The filename keeps
tracking it: rename the first line, and the file on disk quietly renames
to match, right up until you've saved it under some other name on
purpose.

Once a note exists, three verbs are always in the palette:

- **Rename note…** — pick a new name yourself, breaking the
  first-line-tracks-filename link for that note.
- **Duplicate note** — a copy, immediately, no dialog.
- **Move note…** — file it somewhere else under your notes tree (or out
  of it).

None of these carry a default chord — they're rare enough, and clear
enough by name, to live in the palette only. ⌘P, type "rename", Enter.

## Keys

Every command has up to two bindings — **slot 1 is native** (⌘ on
macOS, Ctrl on Linux) and is the one awl teaches; **slot 2 is Emacs**, a
quiet second layer that never goes away. Both fire, always. The
palette (⌘P) shows both next to every command's name, so it teaches the
chords as you search.

**Linux gets the same commands under Ctrl**, and where a native Ctrl
chord would collide with a bare-control Emacs default (Ctrl-S save vs.
the Emacs `C-s` search, say), the native one wins and the Emacs default
quietly steps aside — still one `[keys]` line away if you want it back.
If you're an Emacs hand and want the *whole* displaced cluster back at
once rather than naming chords one at a time, set `keymap = "emacs"` in
the config — a whole-catalog preset over the same mechanism. The
**Omarchy/Hyprland recipe**, since that compositor forwards Super+C/X/V
as Ctrl+C/X/V for the system clipboard: `keymap = "emacs"` plus
`[keys] copy = "C-c"`, `cut = "C-x"`, `paste = "C-v"` keeps those three
chords native no matter what the emacs preset would otherwise reclaim.

**Rebind anything.** `[keys]` in the config maps a command's slugified
name to a chord (or up to two). For example, to bring back the
Option-letter word motions the platform rule retired by default (macOS
reserves Option-letters for accented characters):

```toml
[keys]
forward_word  = ["M-Right", "M-f"]
backward_word = ["M-Left", "M-b"]
```

Or press the actual key: ⌘P → "Keybindings…" opens a picker over every
command, Enter starts a capture, and the next key or chord you press
becomes the new binding — written back into your config for you.

**The hold-⌘ peek.** Hold the arming modifier alone for a beat (⌘ on
Mac, Ctrl on Linux — whichever convention your chords live under) and a
calm card of shortcuts appears: the ones you reach for often but keep
taking the slow palette door to. Release the hold, the card is gone —
no click, no dismiss, it just answers the "what were the shortcuts
again?" moment you were already in.

Generated from the live command catalog — never hand-edited (see the law
test `guide::tests::generated_keys_reference_matches_catalog`; regenerate
with `cargo test --bin awl guide::tests::print_generated_keys_reference
-- --ignored --nocapture` and paste the printed table between the markers
below, byte for byte).

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
| Insert link… | ⌘K | Ctrl+K |
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

## Looks

**Fifteen worlds, one chord away.** ⌘T (Ctrl-T on Linux) opens the
theme picker — every world pairs its own display face with its own ink
ladder, never a bolted-on color scheme. Wagtail is the odd one out on
purpose: awl's first true monochrome world, drawn only in black, white,
and nothing between — a deliberate exception to the "one warm accent"
rule everywhere else.

**Two page widths, one for prose, one for code.** The writing column
measures 70 characters by default for prose and 100 for code (rustfmt's
own convention) — independent settings, so widening one never touches
the other. Drag the column's edge with the mouse, or reach for "Widen
page" / "Narrow page" / "Reset page width" in the palette.

**WYSIWYG, reveal-on-caret.** Markdown markup — a heading's `#`,
`**bold**`, `` `code` ``, `==highlight==`, a fenced code block's fence
lines — renders concealed everywhere EXCEPT the line your caret is
actually on, where it shows in full so you can edit it. Move the caret
away and it conceals again. The file on disk is always plain markdown;
only the on-screen render is rich. Turn it off entirely with
`wysiwyg = false` if you'd rather see the markup all the time.

**Reduce Motion, honestly wired.** Every bit of juice — the caret's
spring and glide, its little squash on a fast edit, the copy pulse — is
a genuine accessibility preference, not a cosmetic toggle. Absent
config means `auto`: awl reads the OS-level "Reduce Motion" setting
where one is reachable (macOS, the web build) and follows it. Set
`reduce_motion = true` by hand on Linux, where there's no reliable
cross-desktop signal to read yet.

## The config file

Settings live in a plain text file you edit inside awl: ⌘P → "Settings"
opens `config.toml` right into the buffer (writing the commented
starter template first, if none exists yet). Edit it like any other
document, then save — the keymap, folders, and every sticky preference
re-apply live, no restart. A config with a genuine syntax error keeps
your prior values in place and shows a notice, rather than resetting
anything.

Nothing here is required — an absent config is just today's defaults,
purely additive. But once you touch it, it remembers: theme, zoom, page
widths, caret style, dictionary, and a dozen other toggles all persist
across launches the moment you change them live, and you can always
just hand-edit the same keys yourself.
