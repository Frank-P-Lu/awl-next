//! Syntax styling spans — awl's Alabaster-style highlighting for CODE buffers.
//!
//! THE PHILOSOPHY (tonsky.me/blog/alabaster): we do NOT rainbow-highlight. A code
//! buffer keeps almost everything in the DEFAULT ink — keywords, operators,
//! identifiers, punctuation all ride `base_content`. Only FOUR roles are ever
//! distinguished, and they distinguish by VALUE first (a muted, low-saturation
//! tint derived from the active theme), never by a loud hue and NEVER by amber
//! (DESIGN.md §3: `primary`/amber is the caret and only the caret):
//!
//! - [`SynKind::Comment`]    — recedes to the DIM ink, exactly like markdown markup.
//! - [`SynKind::Str`]        — string + char literals.
//! - [`SynKind::Constant`]   — numbers, booleans, `nil`/`null`/`None`-style literals.
//! - [`SynKind::Definition`] — the NAME being defined (after `fn`/`def`/`class`/
//!   `struct`/`type`/…, best-effort per language).
//!
//! This module is PURE: [`spans`] is a deterministic function of the text (no
//! clock, no layout), so a headless capture renders the settled styled state and
//! the sidecar can report the spans verbatim (the `syn_spans` block).
//!
//! GATING is by file EXTENSION ([`Lang::from_path`]): only the recognized code
//! extensions below highlight. `.env`, `.md`/`.markdown`, `.txt`, and any unknown
//! / scratch buffer return `None` and render BYTE-IDENTICAL to a plain buffer.
//!
//! ## Adding / completing a language
//!
//! Every language lives in its own file `src/syntax/<lang>.rs` exposing exactly:
//!
//! ```ignore
//! pub fn spans(text: &str) -> Vec<(std::ops::Range<usize>, super::SynKind)>;
//! ```
//!
//! All 20 are PRE-WIRED into [`spans`] below, so completing a language edits ONLY
//! its own `<lang>.rs` (and that file's tests) — never this file, `theme.rs`, or
//! `render.rs`. [`rust`] and [`python`] are the fully-implemented REFERENCE lexers;
//! the rest are stubs returning an empty list (so a stub language renders plain).

use std::ops::Range;

pub mod bash;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod css;
pub mod go;
pub mod html;
pub mod java;
pub mod javascript;
pub mod json;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod sql;
pub mod swift;
pub mod toml;
pub mod typescript;
pub mod yaml;

/// One highlighted ROLE. These are the ONLY four roles awl colors (Alabaster
/// philosophy); everything else in a code buffer stays the default ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynKind {
    /// Line + block comments. Recedes to the DIM ink (`base_content_dim`).
    Comment,
    /// String + char literals (incl. raw / triple where the language has them).
    Str,
    /// Numbers, booleans, and `nil`/`null`/`None`-style literals.
    Constant,
    /// The NAME being defined — the identifier right after a `fn`/`def`/`class`/
    /// `struct`/`type`/… introducer (best-effort per language).
    Definition,
}

impl SynKind {
    /// Stable tag string for the capture sidecar's `syn_spans` block.
    pub fn tag(self) -> &'static str {
        match self {
            SynKind::Comment => "comment",
            SynKind::Str => "string",
            SynKind::Constant => "constant",
            SynKind::Definition => "definition",
        }
    }
}

/// A recognized CODE language. Detected purely by file extension
/// ([`Lang::from_path`] / [`Lang::from_extension`]); a buffer whose extension is
/// not one of these (incl. `.env`, `.md`, `.txt`) has no `Lang` and is NOT
/// highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    C,
    Cpp,
    Java,
    CSharp,
    Ruby,
    Php,
    Swift,
    Kotlin,
    Bash,
    Html,
    Css,
    Json,
    Yaml,
    Toml,
    Sql,
}

impl Lang {
    /// Map a file extension (WITHOUT the dot, any case) to a language, or `None`
    /// for an unrecognized / explicitly-excluded extension. This is the GATE: the
    /// excluded prose/work-file cases (`env`, `md`, `markdown`, `txt`) deliberately
    /// fall through to `None` so they render byte-identically to a plain buffer.
    pub fn from_extension(ext: &str) -> Option<Lang> {
        let e = ext.to_ascii_lowercase();
        Some(match e.as_str() {
            "rs" => Lang::Rust,
            "py" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "tsx" => Lang::TypeScript,
            "go" => Lang::Go,
            "c" | "h" => Lang::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" => Lang::Cpp,
            "java" => Lang::Java,
            "cs" => Lang::CSharp,
            "rb" => Lang::Ruby,
            "php" => Lang::Php,
            "swift" => Lang::Swift,
            "kt" | "kts" => Lang::Kotlin,
            "sh" | "bash" | "zsh" => Lang::Bash,
            "html" | "htm" => Lang::Html,
            "css" => Lang::Css,
            "json" => Lang::Json,
            "yaml" | "yml" => Lang::Yaml,
            "toml" => Lang::Toml,
            "sql" => Lang::Sql,
            _ => return None,
        })
    }

    /// Detect the language of a file path by its extension. `None` for a path with
    /// no extension or an unrecognized one (the gate above). A `.env` file has the
    /// FILE NAME `.env` (no real extension), so it yields `None` here too.
    pub fn from_path(path: &std::path::Path) -> Option<Lang> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Lang::from_extension)
    }
}

/// THE DISPATCH: parse `text` into syntax styling spans for `lang`, in DOCUMENT
/// byte coordinates. Each arm calls the matching `<lang>.rs::spans`, so language
/// work touches only that one file. Spans may be returned in any order and may
/// overlap; the renderer applies them in order (last-wins on overlap), so a lexer
/// that pushes a coarse span then a finer one inside it gets the finer styling.
pub fn spans(lang: Lang, text: &str) -> Vec<(Range<usize>, SynKind)> {
    match lang {
        Lang::Rust => rust::spans(text),
        Lang::Python => python::spans(text),
        Lang::JavaScript => javascript::spans(text),
        Lang::TypeScript => typescript::spans(text),
        Lang::Go => go::spans(text),
        Lang::C => c::spans(text),
        Lang::Cpp => cpp::spans(text),
        Lang::Java => java::spans(text),
        Lang::CSharp => csharp::spans(text),
        Lang::Ruby => ruby::spans(text),
        Lang::Php => php::spans(text),
        Lang::Swift => swift::spans(text),
        Lang::Kotlin => kotlin::spans(text),
        Lang::Bash => bash::spans(text),
        Lang::Html => html::spans(text),
        Lang::Css => css::spans(text),
        Lang::Json => json::spans(text),
        Lang::Yaml => yaml::spans(text),
        Lang::Toml => toml::spans(text),
        Lang::Sql => sql::spans(text),
    }
}

// ---------------------------------------------------------------------------
// Shared lexer primitives
//
// The hand-written scanners below were copy-pasted across nearly every
// `<lang>.rs`; they are gathered here so a lexer just `use`s or calls them
// instead of carrying its own identical copy. They add NO behavior — each is the
// exact body the lexers already shipped, lifted verbatim and (where languages
// differed only in a constant) parameterized. A lexer with a genuine quirk
// (extra identifier chars, a hex-float exponent, a digit separator, …) keeps its
// own local version and does not reach for these.
// ---------------------------------------------------------------------------

/// True if `c` can START an identifier in the common case: ASCII letter or `_`.
/// The lexers whose identifier rule is exactly this `use super::is_ident_start`;
/// the few with extra chars (`$` in JS/TS/Java, `-` in CSS, non-ASCII in PHP)
/// keep a local override instead.
pub(super) fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

/// True if `c` can CONTINUE an identifier in the common case: ASCII alphanumeric
/// or `_`. Companion to [`is_ident_start`]; same sharing rule.
pub(super) fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

/// Scan a quoted literal starting at the opening quote `open`; return the index
/// just past the closing `quote` byte (or, when `stop_at_newline`, the newline
/// that terminates an unclosed single-line literal — or EOF). A `\\` escapes the
/// next byte so an escaped quote does not close the literal. This is the shared
/// body behind ~15 per-language `scan_string` scanners; the caller supplies the
/// `quote` byte (a fixed `"` for most, `b[open]` for the languages that also
/// accept `'`) and whether a newline ends an unterminated literal.
pub(super) fn scan_quoted(b: &[u8], open: usize, quote: u8, stop_at_newline: bool) -> usize {
    let n = b.len();
    let mut i = open + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'\n' if stop_at_newline => return i,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    n
}

/// Per-language knobs for the shared [`scan_number`] — the small set of constants
/// the otherwise-identical numeric scanners varied by.
pub(super) struct NumOpts {
    /// Letters that, right after a leading `0`, open a radix-prefixed integer
    /// (e.g. `b"xXoObB"` for hex/octal/binary, `b"xXbB"` where there is no `0o`).
    pub radix: &'static [u8],
    /// Extra bytes allowed inside a radix body beyond ASCII alphanumerics + `_`
    /// (Go alone allows a `.` for its hex floats; most pass `b""`).
    pub radix_extra: &'static [u8],
    /// Whether a `..` after the integer part stops the scan (the range op): when
    /// set, a `.` followed by another `.` ends the number; otherwise only a `.`
    /// before an identifier start does.
    pub dot_dot_stops: bool,
}

/// Scan a numeric literal beginning at the digit `i`; return the index just past
/// it. The shared body behind the "decimal + radix-prefix" family of scanners:
/// a `0x`/`0o`/`0b` (per `opts.radix`) integer consumes alphanumerics, `_`, and
/// `opts.radix_extra`; otherwise the decimal loop consumes alphanumerics + `_`
/// and a fractional `.` unless that `.` opens a range (`opts.dot_dot_stops`) or a
/// member access (a `.` before `is_ident_start`). The `is_ident_start` predicate
/// is passed in so a language with extra identifier chars (`$`, non-ASCII) keeps
/// its exact member-access boundary. Languages with a hex-float exponent or a
/// digit separator keep their own local scanner.
pub(super) fn scan_number(
    b: &[u8],
    i: usize,
    opts: NumOpts,
    is_ident_start: fn(u8) -> bool,
) -> usize {
    let n = b.len();
    let mut j = i + 1;
    if b[i] == b'0' && j < n && opts.radix.contains(&b[j]) {
        j += 1;
        while j < n
            && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || opts.radix_extra.contains(&b[j]))
        {
            j += 1;
        }
        return j;
    }
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' {
            if j + 1 < n && ((opts.dot_dot_stops && b[j + 1] == b'.') || is_ident_start(b[j + 1])) {
                break;
            }
            j += 1;
        } else {
            break;
        }
    }
    j
}

/// Shared assertion helpers for the per-language lexer tests. Every `<lang>.rs`
/// test module reaches for the same two: [`has`] (an exact `start..end` span of a
/// role exists) and [`at`] (the substrings a role covers, for readable failures).
/// Colocated here so each lexer just `use crate::syntax::testutil::{has, at};`
/// instead of copy-pasting the pair. Test-only — no runtime impact.
#[cfg(test)]
pub(crate) mod testutil {
    use super::{Range, SynKind};

    /// True if `s` contains an exact `lo..hi` span of role `k`.
    pub(crate) fn has(s: &[(Range<usize>, SynKind)], lo: usize, hi: usize, k: SynKind) -> bool {
        s.iter().any(|(r, kk)| r.start == lo && r.end == hi && *kk == k)
    }

    /// The substring each span of role `k` covers, for readable assertions.
    pub(crate) fn at<'a>(text: &'a str, s: &[(Range<usize>, SynKind)], k: SynKind) -> Vec<&'a str> {
        s.iter().filter(|(_, kk)| *kk == k).map(|(r, _)| &text[r.clone()]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_detection_covers_all_languages() {
        assert_eq!(Lang::from_extension("rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension("py"), Some(Lang::Python));
        for e in ["js", "mjs", "cjs", "jsx"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::JavaScript), "{e}");
        }
        for e in ["ts", "tsx"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::TypeScript), "{e}");
        }
        assert_eq!(Lang::from_extension("go"), Some(Lang::Go));
        for e in ["c", "h"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::C), "{e}");
        }
        for e in ["cpp", "cc", "cxx", "hpp", "hh"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::Cpp), "{e}");
        }
        assert_eq!(Lang::from_extension("java"), Some(Lang::Java));
        assert_eq!(Lang::from_extension("cs"), Some(Lang::CSharp));
        assert_eq!(Lang::from_extension("rb"), Some(Lang::Ruby));
        assert_eq!(Lang::from_extension("php"), Some(Lang::Php));
        assert_eq!(Lang::from_extension("swift"), Some(Lang::Swift));
        for e in ["kt", "kts"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::Kotlin), "{e}");
        }
        for e in ["sh", "bash", "zsh"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::Bash), "{e}");
        }
        for e in ["html", "htm"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::Html), "{e}");
        }
        assert_eq!(Lang::from_extension("css"), Some(Lang::Css));
        assert_eq!(Lang::from_extension("json"), Some(Lang::Json));
        for e in ["yaml", "yml"] {
            assert_eq!(Lang::from_extension(e), Some(Lang::Yaml), "{e}");
        }
        assert_eq!(Lang::from_extension("toml"), Some(Lang::Toml));
        assert_eq!(Lang::from_extension("sql"), Some(Lang::Sql));
    }

    #[test]
    fn extension_detection_is_case_insensitive() {
        assert_eq!(Lang::from_extension("RS"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension("Py"), Some(Lang::Python));
    }

    #[test]
    fn excluded_and_unknown_extensions_are_none() {
        // The gate: prose / work-file cases must NOT highlight.
        for e in ["env", "md", "markdown", "txt", "", "log", "bin", "lock"] {
            assert_eq!(Lang::from_extension(e), None, "{e:?} must not highlight");
        }
    }

    #[test]
    fn from_path_uses_extension() {
        use std::path::Path;
        assert_eq!(Lang::from_path(Path::new("/a/b/main.rs")), Some(Lang::Rust));
        assert_eq!(Lang::from_path(Path::new("notes.md")), None);
        assert_eq!(Lang::from_path(Path::new("README")), None);
        // A `.env` file has no real extension (the name IS `.env`) -> no highlight.
        assert_eq!(Lang::from_path(Path::new(".env")), None);
    }

    #[test]
    fn tags_are_stable() {
        assert_eq!(SynKind::Comment.tag(), "comment");
        assert_eq!(SynKind::Str.tag(), "string");
        assert_eq!(SynKind::Constant.tag(), "constant");
        assert_eq!(SynKind::Definition.tag(), "definition");
    }

    #[test]
    fn dispatch_routes_to_implemented_lexers() {
        // Every language now has a real lexer; a trivial snippet yields a span.
        assert!(!spans(Lang::Rust, "// hi\n").is_empty());
        assert!(!spans(Lang::Python, "# hi\n").is_empty());
        assert!(!spans(Lang::Go, "// hi\n").is_empty());
        // A SQL comment recedes too — the dispatch routes to the SQL lexer.
        assert!(!spans(Lang::Sql, "-- hi\n").is_empty());
    }

    #[test]
    fn shared_is_ident_is_the_ascii_common_case() {
        assert!(is_ident_start(b'_') && is_ident_start(b'a') && is_ident_start(b'Z'));
        assert!(!is_ident_start(b'0') && !is_ident_start(b'$') && !is_ident_start(b'-'));
        assert!(is_ident_continue(b'_') && is_ident_continue(b'9') && is_ident_continue(b'x'));
        assert!(!is_ident_continue(b'$') && !is_ident_continue(b' '));
    }

    #[test]
    fn shared_scan_quoted_handles_escapes_quote_and_newline() {
        // Closes on the matching quote; the index returned is just past it.
        let t = br#""ab"x"#;
        assert_eq!(scan_quoted(t, 0, b'"', false), 4);
        // A `\\` escapes the next byte, so an escaped quote does not close.
        let e = br#""a\"b""#;
        assert_eq!(scan_quoted(e, 0, b'"', false), e.len());
        // With `stop_at_newline`, an unterminated literal ends AT the newline...
        let nl = b"\"ab\ncd";
        assert_eq!(scan_quoted(nl, 0, b'"', true), 3);
        // ...without it, the newline rides inside and the scan runs to EOF.
        assert_eq!(scan_quoted(nl, 0, b'"', false), nl.len());
        // The quote byte is the caller's choice (single quotes too).
        let sq = b"'q' ";
        assert_eq!(scan_quoted(sq, 0, b'\'', false), 3);
    }

    #[test]
    fn shared_scan_number_radix_fraction_and_boundaries() {
        let o = || NumOpts { radix: b"xXoObB", radix_extra: b"", dot_dot_stops: true };
        // A hex body consumes alnum + `_`; the suffix rides along.
        let hex = b"0xFF_u8;";
        assert_eq!(scan_number(hex, 0, o(), is_ident_start), 7);
        // A float keeps its fractional point.
        let f = b"3.14 ";
        assert_eq!(scan_number(f, 0, o(), is_ident_start), 4);
        // `dot_dot_stops`: a `..` range op ends the integer before the dots.
        let r = b"0..5";
        assert_eq!(scan_number(r, 0, o(), is_ident_start), 1);
        // A member access (`.` before an ident start) also ends the integer.
        let m = b"1.foo";
        assert_eq!(scan_number(m, 0, o(), is_ident_start), 1);
        // Without `dot_dot_stops`, only the member-access guard applies, so a
        // `.`-before-digit is consumed as a fraction.
        let no = NumOpts { radix: b"xXbB", radix_extra: b"", dot_dot_stops: false };
        assert_eq!(scan_number(b"1.5", 0, no, is_ident_start), 3);
    }
}
