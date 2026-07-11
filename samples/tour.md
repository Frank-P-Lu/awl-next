# A tour of the markdown

This whole page is plain markdown, rendered live. Land your
caret on any line below and watch its raw syntax reveal;
move on and it settles back into the render. That reveal
*is* the demo — no other trick to it.

## Headings size themselves
A heading grows by level alone — no bold, no color, just
weight of place on the page. Put the caret on the line
above and you'll see the plain `#` again.

## Emphasis hides its own marks
Rest the caret inside **this bold run**, or *this italic
one*, or on `this inline code` — the stars, underscores,
and backticks only show while you're there.

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

That's the whole vocabulary. Everything past this line is
just words — go ahead and use them.
