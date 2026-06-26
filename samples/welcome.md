# awl editor

A calm, native editor for writing — and light editing — with mg / Emacs keys.
It opens in Tawny; press C-x t to wander the other worlds.

## Move
- C-f / C-b  char,   C-n / C-p  line
- M-f / M-b  word,   C-a / C-e  line ends
- M-< / M->  buffer ends,   C-v / M-v  page

## Edit
Type to insert. C-d delete, C-k kill line, C-y yank.
C-Space sets the mark; C-w cut, M-w copy the region. C-/ undo.

## Find & go
- C-s / C-r   incremental search
- C-x C-f     go to a file in this project (fuzzy)
- C-x j       browse the folder, one level at a time
- C-x p       switch project
- C-x b       flip to the last file

## Look
- C-x t       switch theme — eight worlds, each with its own type
- C-x c       caret: solid block, or a glyph that morphs as it moves
- Cmd +/-/0   zoom

## Files
C-x C-s save · C-x C-c quit · C-g cancel

The quick brown fox jumps over the lazy dog.
