# A tour of the markdown

This page is plain markdown, rendered live. Land your caret on any
line below and its raw syntax reveals; move on and it settles back
into the render.

## Headings size themselves
A heading grows by level alone — no bold, no color, just size. Put the
caret on the line above to see the plain `#` again.

## Emphasis hides its own marks
Rest the caret inside **this bold run**, or *this italic
one*, or on `this inline code` — the stars, underscores,
and backticks show only while you're there.

## A highlight
Some words want a wash, not a shout: ==like this one==.
Land the caret on it to see the plain `==marks==` underneath.

## A short list
- [x] read the tour
- [ ] pick a theme ({{key:switch_theme}})
- [ ] start writing something of your own

## A small table
| World     | Face      | Mood  |
|-----------|-----------|-------|
| Tawny     | Bitter    | warm  |
| Mopoke    | Klee One  | quiet |
| Currawong | Fira Sans | crisp |

## A quote, and a fence
> Good prose is like a windowpane: you look through it at
> the idea, not at the glass.

```rust
fn caret() -> &'static str {
    "the one warm thing in a calm room"
}
```

---

That covers the vocabulary. Everything past this line is plain text.
