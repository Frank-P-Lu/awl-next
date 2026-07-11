# awl

A plain-text editor for prose, and light code. Type; the page renders
as you write, and drops back to plain markdown the instant your caret
lands on a line, so the markup is one keystroke away.

## Three doors, always open
- {{key:command_palette}}   the command palette — every command, by name
- {{key:switch_theme}}   switch theme — fifteen worlds, each its own type
- {{key:settings}}   settings — the config, as a text file you edit here

({{key:command_palette}} → "Guide" opens the full user guide as a page, any time.)

## Write
Type to insert. {{key:undo}} undo, {{key:cut}} / {{key:copy}} / {{key:paste}} cut, copy, paste.
{{key:bold}} bold, {{key:italic}} italic, {{key:inline_code}} inline code — the rest of the
formatting toggles (lists, highlight, blockquote…) live in the palette.

## Two more pages worth a look
- /tour.md — a one-page tour of the markdown; try the
  caret on each line and watch it reveal
- /japanese.md — the bundled Japanese type, rendering
  with no network request at all

## This is the browser build

| | Here | Desktop (macOS / Linux) |
|---|---|---|
| Storage | `localStorage`, capped around 5 MB — roughly eight to ten novels of plain text | Real files on disk |
| Preferences, `[keys]` | A `config.toml` over `localStorage`, persists across reloads | `~/.config/awl/config.toml` |
| Copy | Mirrors out to the OS clipboard | To the OS clipboard |
| Paste | From awl's own kill ring only | From the OS clipboard |
| Getting a file out | "Download file" ({{key:command_palette}}) | Already on disk |

A couple of native chords belong to the browser itself (new tab, new
window, and similar) — {{key:new_note}} and {{key:switch_theme}}
resolve to a working alternate here automatically. Every command is
also reachable by name through {{key:command_palette}}.

The desktop build has no storage cap and full OS clipboard paste — see
the project's releases page for macOS and Linux downloads.

The quick brown fox jumps over the lazy dog.
