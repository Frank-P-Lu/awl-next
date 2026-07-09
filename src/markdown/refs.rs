//! Image + link REFERENCES: [`ImageRef`] parsing (the Obsidian `|WIDTH` hint
//! convention) and [`link_at`] (the open-link-at-point lookup), both reused
//! by [`super::spans::spans`]. Split out of the former `markdown.rs`
//! monolith (2026-07 code-organization pass); every item's path is
//! unchanged (`markdown::ImageRef`, `markdown::link_at`, …) -- only the file
//! it lives in moved.

/// One parsed IMAGE reference, recovered from an `![alt](path)` SOURCE substring
/// (the byte range of a [`MdKind::ConcealMarkup`]`(`[`ConcealKind::Image`]`)`
/// span). PURE data — the renderer feeds `text[range]` to [`parse_image_source`]
/// each reshape to get the destination PATH (to read the image's header
/// dimensions + draw it), the ALT text, and an optional Obsidian-style width
/// HINT, without a second pulldown parse. This is the "side table keyed by
/// span" the design chose over widening the `Copy` [`MdKind`] with `String`
/// fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// The alt text with any trailing `|NNN`/`|WxH` size hint stripped off.
    pub alt: String,
    /// The image destination — the `(path)` link target (title/angle-brackets
    /// stripped). May be relative (resolved against the doc's dir by the caller).
    pub path: String,
    /// The width HINT parsed OUT of the alt (`![alt|300](p)` → `Some(300)`;
    /// `![alt|300x200](p)` → `Some(300)`, the WIDTH — the height is derived from
    /// the intrinsic aspect, so a `WxH`'s `H` is ignored in v1). `None` when the
    /// alt carries no `|NNN`/`|WxH` suffix.
    pub width_hint: Option<u32>,
}

/// Parse an `![alt](path)` image SOURCE substring into its [`ImageRef`] parts.
/// Lenient + total (returns `None` only if the substring isn't a well-formed
/// `![...](...)`), operating on the exact byte range pulldown ruled an image, so
/// there is no ambiguity about where the ref begins/ends. Handles a `(path
/// "title")` (path = first whitespace token) and a `(<path>)` angle form, and
/// splits the Obsidian size hint out of the alt via [`split_alt_hint`].
pub fn parse_image_source(src: &str) -> Option<ImageRef> {
    let rest = src.trim().strip_prefix("![")?;
    let close = rest.find(']')?;
    let raw_alt = &rest[..close];
    let inner = rest[close + 1..].trim_start().strip_prefix('(')?;
    let end = inner.find(')')?;
    let dest = inner[..end].trim();
    let path = if let Some(a) = dest.strip_prefix('<') {
        a.split('>').next().unwrap_or("").to_string()
    } else {
        dest.split_whitespace().next().unwrap_or("").to_string()
    };
    if path.is_empty() {
        return None;
    }
    let (alt, width_hint) = split_alt_hint(raw_alt);
    Some(ImageRef { alt, path, width_hint })
}

/// Extract EVERY inline image reference `![alt](path)` from `text`, UNGATED by the
/// inline-images toggle. [`spans`] only emits an image span when [`inline_images_on`]
/// is true (native + enabled), so it can't be the scanner's source; this walks the
/// SAME pulldown parse for `Tag::Image` and feeds each image's byte range to the SAME
/// [`parse_image_source`] the renderer trusts — the real parser, never a regex. A
/// reference-style / remote image `parse_image_source` can't resolve to a local
/// `(path)` is skipped (it names no local asset). Frontmatter is stripped first
/// (mirroring [`spans`]), so a metadata value never mis-parses as an image.
///
/// Used by [`crate::assets::scan`] (the Asset Cleaner) to collect the images a
/// document references, so an unreferenced `assets/` file can be found. PURE — no
/// clock, no filesystem — over the document text.
pub fn image_refs(text: &str) -> Vec<ImageRef> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let text = match crate::frontmatter::detect(text) {
        Some(fm) => &text[fm.range.end..],
        None => text,
    };
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    let mut out = Vec::new();
    for (ev, range) in Parser::new_ext(text, opts).into_offset_iter() {
        if let Event::Start(Tag::Image { .. }) = ev {
            if let Some(img) = text.get(range).and_then(parse_image_source) {
                out.push(img);
            }
        }
    }
    out
}

/// Split an image alt on a trailing `|NNN` / `|WxH` size hint (the Obsidian
/// `![alt|300](p)` convention — the size lives in the ALT so pulldown still
/// parses the image cleanly). Returns the alt with the hint removed + the WIDTH
/// (the `NNN`, or the `W` of `WxH`; `H` is ignored — height rides the intrinsic
/// aspect in v1). No `|`, or a non-numeric suffix (so an alt that legitimately
/// contains `|`, like `"a | b"`, is preserved verbatim), yields the alt
/// unchanged + `None`.
fn split_alt_hint(alt: &str) -> (String, Option<u32>) {
    let Some((head, tail)) = alt.rsplit_once('|') else {
        return (alt.to_string(), None);
    };
    let t = tail.trim();
    let (w, h) = match t.split_once(['x', 'X']) {
        Some((w, h)) => (w, Some(h)),
        None => (t, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(w) && h.map(digits).unwrap_or(true) {
        if let Ok(n) = w.parse::<u32>() {
            return (head.trim_end().to_string(), Some(n));
        }
    }
    (alt.to_string(), None)
}

/// Set the Obsidian `|NNN` width hint on an image ALT to `width` — the inverse of
/// [`split_alt_hint`]. If the alt already carries a `|NNN`/`|WxH` hint it is
/// REPLACED (the alt text before it is preserved verbatim); otherwise `|width` is
/// appended after the alt text. An alt that legitimately contains a `|` but no
/// numeric suffix (`"a | b"`) is treated as hint-less, so the new hint appends
/// cleanly (`"a | b|300"`). Pure — the drag-resize write-back builds the new alt
/// with this and applies it as ONE undoable edit.
fn set_alt_width_hint(raw_alt: &str, width: u32) -> String {
    let (base, _) = split_alt_hint(raw_alt);
    if base.is_empty() {
        format!("|{}", width)
    } else {
        format!("{}|{}", base, width)
    }
}

/// DRAG-RESIZE WRITE-BACK: given an image SOURCE substring `![alt](path)` and a new
/// pixel `width`, compute the BYTE RANGE within `src` of the ALT text and the
/// replacement alt (the Obsidian `![alt|NNN](path)` form — the hint set/replaced by
/// [`set_alt_width_hint`]). Returns `None` if `src` isn't a well-formed
/// `![...](...)`. Pure: the app converts the `src`-relative byte offsets to absolute
/// buffer positions and applies ONE [`crate::buffer::Buffer::replace_char_range`] —
/// exactly the single-undoable-edit shape `write_back_lang_tag_once` uses, so a
/// whole drag writes back ONCE on release and Cmd-Z restores the pre-drag size.
pub fn image_width_hint_edit(src: &str, width: u32) -> Option<(usize, usize, String)> {
    let open = src.find("![")?;
    let alt_start = open + 2;
    let close_rel = src.get(alt_start..)?.find(']')?;
    let alt_end = alt_start + close_rel;
    // Must be a real image: a `(path)` link target follows the `]`.
    let after = src.get(alt_end + 1..)?.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    let raw_alt = &src[alt_start..alt_end];
    Some((alt_start, alt_end, set_alt_width_hint(raw_alt, width)))
}

/// The destination URL of the markdown link CONTAINING document byte offset
/// `byte`, or `None` when the caret is not inside any link — the pure extraction
/// behind [`crate::keymap::Action::FollowLink`] (open-link-at-point). Reuses
/// pulldown (the SAME parse [`spans`] drives), tracking each `Tag::Link`'s own
/// `dest_url` against its byte range: the first link whose `[text](url)` range
/// contains `byte` wins. A leading [`crate::frontmatter`] block is skipped exactly
/// like [`spans`] (a link can't live in frontmatter), so `byte` is measured in the
/// same DOCUMENT coordinates the caret uses. Pure + total — never opens anything
/// itself (the live App performs the OS browser handoff on the returned URL); a
/// caret outside every link is the calm `None` no-op.
pub fn link_at(text: &str, byte: usize) -> Option<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let (body, body_offset) = match crate::frontmatter::detect(text) {
        Some(fm) => (&text[fm.range.end..], fm.range.end),
        None => (text, 0),
    };
    if byte < body_offset {
        return None;
    }
    let target = byte - body_offset;
    let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    for (ev, range) in Parser::new_ext(body, opts).into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. }) = ev {
            if range.contains(&target) {
                return Some(dest_url.to_string());
            }
        }
    }
    None
}
