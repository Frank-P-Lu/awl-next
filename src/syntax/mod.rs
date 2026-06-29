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
        // Rust + Python are implemented; a trivial snippet yields at least one span.
        assert!(!spans(Lang::Rust, "// hi\n").is_empty());
        assert!(!spans(Lang::Python, "# hi\n").is_empty());
        // A stub language returns no spans (renders plain) but does not panic.
        assert!(spans(Lang::Go, "package main\n").is_empty());
    }
}
