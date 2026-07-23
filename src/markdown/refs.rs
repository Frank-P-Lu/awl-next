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
///
/// STRICT + the SHARED owner: a `WxH` hint's `H` must be all digits too, so a
/// malformed `alt|300xfoo` / `alt|300x` yields NO hint (the whole run kept as
/// alt, verbatim). The [export model][crate::export::model] routes its image-alt
/// split through this exact fn (re-exported as `crate::markdown::split_alt_hint`),
/// so the editor's applied width hint and the export's can never disagree on a
/// malformed suffix — the divergence a lax second copy once caused (a bad hint
/// rendered natural-width in the editor but sized in the export). Law test:
/// `render_export_alt_hint_agree`.
pub fn split_alt_hint(alt: &str) -> (String, Option<u32>) {
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

/// The DESTINATION byte range within a `[text](url)` / `![alt](url)` markdown
/// reference SOURCE substring — the interior of the `(...)` that follows the
/// label's closing `]`, EXCLUDING the syntax characters and the label/alt text
/// before it. Locates the FIRST `](` (the same split
/// [`super::spans::push_link_markers`]'s conceal split already scans for — a
/// link's own `ConcealKind::Link` tail span starts exactly there) and, for an
/// image, that search skips straight past the `![alt` prefix automatically
/// (the alt text ends at the first `]`, same as [`parse_image_source`]'s own
/// `rest.find(']')`), so ONE tiny idiom serves both reference shapes. Returns
/// `None` when `src` has no `](` or no closing `)` — a reference-style
/// (`[text][ref]`) or malformed link, nothing to exclude, mirroring
/// `push_link_markers`'s own fallback.
pub fn label_destination_range(src: &str) -> Option<std::ops::Range<usize>> {
    let rel = src.find("](")?;
    let inner_start = rel + 2;
    let inner = src.get(inner_start..)?;
    let end_rel = inner.find(')')?;
    Some(inner_start..inner_start + end_rel)
}

/// EVERY inline link/image DESTINATION byte range in the document, in ABSOLUTE
/// document byte coordinates (queue item 60 — "markdown destinations are
/// ADDRESSES, not prose"). Reads the SAME `ConcealMarkup(Link)` /
/// `ConcealMarkup(Image)` spans [`super::spans::spans`] already parsed this
/// reshape (the identical `md_spans` field item 25's concealed-image work
/// reads via `line_is_inline_image`) and slices out just the `(...)` interior
/// with [`label_destination_range`] — never a second pulldown parse, never a
/// second path/extension heuristic. A link's `Link` span is already just the
/// `](url…)` tail (`push_link_markers`), so it needs no alt-skip; an image's
/// `Image` span is the WHOLE `![alt](path…)` reference, and
/// `label_destination_range`'s `](`-search skips past the alt text
/// automatically — so alt text (and a link's visible label, which never
/// shares either span) keeps its own existing spell/nit behavior untouched.
/// Empty for a non-markdown buffer (`md_spans` is empty there) or a document
/// with no link/image, so every other buffer's candidates are byte-identical.
pub fn destination_ranges(
    text: &str,
    md_spans: &[(std::ops::Range<usize>, super::MdKind)],
) -> Vec<std::ops::Range<usize>> {
    use super::{ConcealKind, MdKind};
    let mut out = Vec::new();
    for (r, k) in md_spans {
        if !matches!(
            k,
            MdKind::ConcealMarkup(ConcealKind::Link) | MdKind::ConcealMarkup(ConcealKind::Image)
        ) {
            continue;
        }
        let Some(src) = text.get(r.clone()) else {
            continue;
        };
        if let Some(rel) = label_destination_range(src) {
            out.push(r.start + rel.start..r.start + rel.end);
        }
    }
    out
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
    link_at_full(text, byte).map(|l| l.url)
}

/// LINKS V2: the full markdown link CONTAINING document byte offset `byte` —
/// its whole `[text](url)` DOCUMENT byte range, the visible link TEXT (the raw
/// substring between `[` and the first `]`, so any nested markup the user typed
/// there — `**bold**`, inline code — round-trips verbatim), and the destination
/// URL. The richer sibling of [`link_at`] (which this now delegates its URL
/// extraction to structurally, by construction — the two can never disagree
/// about which link a byte lands in): `link_at` narrows this to just the URL,
/// [`crate::actions`]'s `Action::InsertLink` dispatch reads the whole thing to
/// build its EDIT mode (rewrap `[new-text-preserved](new-url)` over the exact
/// same range). A caret outside every link is the calm `None`.
pub struct LinkAt {
    pub start: usize,
    pub end: usize,
    pub link_text: String,
    pub url: String,
}

pub fn link_at_full(text: &str, byte: usize) -> Option<LinkAt> {
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
                let src = &body[range.clone()];
                let link_text = src
                    .strip_prefix('[')
                    .and_then(|rest| rest.find(']').map(|i| rest[..i].to_string()))
                    .unwrap_or_default();
                return Some(LinkAt {
                    start: range.start + body_offset,
                    end: range.end + body_offset,
                    link_text,
                    url: dest_url.to_string(),
                });
            }
        }
    }
    None
}
