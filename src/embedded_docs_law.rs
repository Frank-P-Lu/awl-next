//! src/embedded_docs_law.rs — the DOCS-DECOUPLING laws.
//!
//! Three standing tests keep documentation coupling honest, so that a doc move
//! is a deliberate choice and a stale citation is a LOUD build failure instead
//! of silent rot:
//!
//! 1. [`embed_owner_is_the_only_include_str_site`] — every `include_str!` of a
//!    repo doc / sample / bundled-license file lives in ONE owner
//!    (`crate::embedded_docs`). A reintroduced scattered embed fails to compile-
//!    green.
//! 2. [`docs_links_resolve`] — every CAPITALIZED root-doc citation (in the root
//!    `*.md` web AND in `src/**` doc-comments), every `DESIGN §N`-style section
//!    citation, and every markdown link between root docs, RESOLVES to a file
//!    that exists at its cited path. A renamed/moved doc that leaves a dangling
//!    reference fails this test.
//! 3. [`test_fixtures_exist`] — the committed test-owned fixtures that the
//!    corpus/index/ranking unit tests name (so they never reference the real
//!    `README.md`) actually exist on disk.
//!
//! The whole module is `#[cfg(test)]` — it ships no runtime code.
#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

/// Repo root (the crate manifest dir — the worktree/checkout root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `*.md` file directly in the repo root (the contract-doc web).
fn root_docs(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(root)
        .expect("read repo root")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "md").unwrap_or(false))
        .collect();
    out.sort();
    out
}

/// Every `*.rs` under `src/` (recursively).
fn src_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_rs(&root.join("src"), &mut out);
    out.sort();
    out
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// PROSE-PLACEHOLDER ALLOWLIST — cited `.md`/`§` tokens that are illustrative,
/// not real files, and are deliberately tolerated. Keep this SHORT and each
/// entry justified; an empty list is the healthy state. (Today: empty — the one
/// former placeholder, `SOMEDOC.md`, was reworded out of `embedded_docs.rs`.)
const ALLOW: &[&str] = &[
    // A metavariable in CAPTURE.md's harness prose (`samples/NAME.md` = "any
    // sample"), not a real file.
    "samples/NAME.md",
    // CREDITS.md cites the UPSTREAM iA-Fonts repo's `LICENSE.md` (a different
    // project on GitHub), not a file in this repo.
    "LICENSE.md",
    // Bare shorthand in RELEASING.md / WORLDS.md prose for the per-directory
    // asset license inventories. The REAL files live at `assets/fonts/LICENSES.md`
    // and `assets/dict/LICENSES.md` — both verified at their full path by the
    // markdown-link scan here and by `scripts/site-links.sh`.
    "LICENSES.md",
];

/// The ALL-CAPS root-doc naming grammar: a basename of `[A-Z][A-Z0-9-]+`
/// (two-or-more chars, uppercase/digit/hyphen). Matches `DESIGN`, `CREDITS`,
/// `THIRD-PARTY-LICENSES`, `LICENSES`, `README`, `WORLD-ROLES` … and NOT the
/// lowercase virtual fixture paths (`/notes/a.md`, `/proj/doc.md`) that fill
/// the test suite, nor single-letter examples (`A.md`, `N.md`).
fn is_doc_basename(base: &str) -> bool {
    let mut chars = base.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    let rest: Vec<char> = chars.collect();
    if rest.is_empty() {
        return false; // need 2+ chars total (excludes A.md / N.md)
    }
    rest.iter()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
}

/// Resolve a cited token to a repo path, or `None` if it is not a repo-relative
/// file citation (absolute virtual fixture, URL, …). Strips leading `./` and
/// `../` segments (a `src/…` comment cites one dir up) and rejoins from root.
fn resolve(root: &Path, token: &str) -> Option<PathBuf> {
    if token.starts_with('/') {
        return None; // absolute → a virtual test-fixture path, not a citation
    }
    let mut rel = token;
    loop {
        if let Some(r) = rel.strip_prefix("./") {
            rel = r;
        } else if let Some(r) = rel.strip_prefix("../") {
            rel = r;
        } else {
            break;
        }
    }
    if rel.is_empty() {
        return None;
    }
    Some(root.join(rel))
}

/// Scan `text` for CAPITALIZED-doc `.md` citations, pushing each raw token.
fn scan_doc_md_tokens(text: &str, out: &mut Vec<String>) {
    let bytes = text.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(".md") {
        let dot = search_from + rel; // index of '.' in ".md"
        let after = dot + 3;
        search_from = after;
        // Reject ".md" that is a prefix of a longer extension (".mdx", ".markdown").
        if let Some(&nb) = bytes.get(after) {
            if (nb as char).is_ascii_alphanumeric() {
                continue;
            }
        }
        // Walk left over path-token characters. STOP before crossing a `/` that
        // terminates a PREVIOUS `.md` (so an adjacent citation pair like
        // `DESIGN.md/PHILOSOPHY.md` scans as two tokens, not one bogus path).
        let mut start = dot;
        while start > 0 {
            let c = bytes[start - 1] as char;
            if c == '/' {
                let sl = start - 1; // index of the '/'
                if sl >= 3 && &text[sl - 3..sl] == ".md" {
                    break;
                }
            }
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/') {
                start -= 1;
            } else {
                break;
            }
        }
        let token = &text[start..after];
        let base = token.rsplit('/').next().unwrap_or(token);
        let base = base.strip_suffix(".md").unwrap_or(base);
        if is_doc_basename(base) {
            out.push(token.to_string());
        }
    }
}

/// Scan `text` for `NAME §` section citations (e.g. `DESIGN §3`), pushing
/// `NAME.md` for each uppercase NAME.
fn scan_section_citations(text: &str, out: &mut Vec<String>) {
    let bytes = text.as_bytes();
    for (i, _) in text.match_indices('§') {
        // Walk left over spaces, then capture the preceding word.
        let mut j = i;
        while j > 0 && (bytes[j - 1] as char).is_whitespace() {
            j -= 1;
        }
        let word_end = j;
        while j > 0 {
            let c = bytes[j - 1] as char;
            if c.is_ascii_alphanumeric() || c == '-' {
                j -= 1;
            } else {
                break;
            }
        }
        let word = &text[j..word_end];
        if is_doc_basename(word) {
            out.push(format!("{word}.md"));
        }
    }
}

/// Scan root-doc `text` for markdown links `](target.md)`, pushing each
/// repo-relative `.md` target (URLs / anchors skipped).
fn scan_markdown_links(text: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(p) = rest.find("](") {
        let after = &rest[p + 2..];
        let Some(close) = after.find(')') else { break };
        let mut target = &after[..close];
        rest = &after[close + 1..];
        if let Some(hash) = target.find('#') {
            target = &target[..hash];
        }
        if target.is_empty()
            || target.starts_with("http")
            || target.starts_with("mailto:")
        {
            continue;
        }
        if target.ends_with(".md") {
            out.push(target.to_string());
        }
    }
}

/// THE LINK LAW. Collects every capitalized-doc citation (root docs + src),
/// every section citation, and every root-doc markdown link, then asserts each
/// resolves to a file that EXISTS at its cited path. Fast: pure filesystem +
/// string scanning, no network.
#[test]
fn docs_links_resolve() {
    let root = repo_root();
    let docs = root_docs(&root);
    let srcs = src_rs_files(&root);

    // (token, origin-description) so a failure names WHERE the dangling ref is.
    let mut cited: Vec<(String, String)> = Vec::new();

    for doc in &docs {
        let text = fs::read_to_string(doc).expect("read root doc");
        let name = doc.file_name().unwrap().to_string_lossy().to_string();
        let mut toks = Vec::new();
        scan_doc_md_tokens(&text, &mut toks);
        scan_section_citations(&text, &mut toks);
        scan_markdown_links(&text, &mut toks);
        for t in toks {
            cited.push((t, name.clone()));
        }
    }
    for src in &srcs {
        // The law module cites the very tokens it defends; skip its own text.
        if src.ends_with("embedded_docs_law.rs") {
            continue;
        }
        let text = fs::read_to_string(src).expect("read src file");
        let name = src
            .strip_prefix(&root)
            .unwrap_or(src)
            .to_string_lossy()
            .to_string();
        let mut toks = Vec::new();
        scan_doc_md_tokens(&text, &mut toks);
        scan_section_citations(&text, &mut toks);
        for t in toks {
            cited.push((t, name.clone()));
        }
    }

    let mut dangling: Vec<String> = Vec::new();
    for (token, origin) in &cited {
        if ALLOW.contains(&token.as_str()) {
            continue;
        }
        match resolve(&root, token) {
            None => {} // absolute/URL — not a repo-relative citation
            Some(path) => {
                if !path.exists() {
                    dangling.push(format!("{origin} cites `{token}` → {path:?} (MISSING)"));
                }
            }
        }
    }
    dangling.sort();
    dangling.dedup();
    assert!(
        dangling.is_empty(),
        "dangling documentation citations (fix the path, move the doc back, or \
         allowlist an intentional placeholder in embedded_docs_law::ALLOW):\n{}",
        dangling.join("\n")
    );
}

/// THE EMBED-OWNER LAW. Every `include_str!` of a repo doc / sample / bundled-
/// license file must live in `src/embedded_docs.rs` alone. A reintroduced
/// scattered embed (the coupling this round removed) fails here.
#[test]
fn embed_owner_is_the_only_include_str_site() {
    let root = repo_root();
    let mut offenders: Vec<String> = Vec::new();
    for src in src_rs_files(&root) {
        if src.ends_with("embedded_docs.rs") {
            continue; // the ONE sanctioned owner
        }
        let text = fs::read_to_string(&src).expect("read src file");
        let name = src
            .strip_prefix(&root)
            .unwrap_or(&src)
            .to_string_lossy()
            .to_string();
        let mut from = 0usize;
        while let Some(p) = text[from..].find("include_str!") {
            let idx = from + p;
            from = idx + "include_str!".len();
            // Parse the string-literal argument: skip `(`, whitespace, then `"…"`.
            let tail = text[from..].trim_start();
            let tail = match tail.strip_prefix('(') {
                Some(t) => t.trim_start(),
                None => continue, // a prose `include_str!` mention, not a macro call
            };
            let Some(rest) = tail.strip_prefix('"') else { continue };
            let Some(qend) = rest.find('"') else { continue };
            let arg = &rest[..qend];
            let base = arg.rsplit('/').next().unwrap_or(arg);
            let is_doc = base.ends_with(".md") || base == "OFL.txt";
            if is_doc {
                offenders.push(format!("{name} embeds `{arg}` (must move to embedded_docs.rs)"));
            }
        }
    }
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "doc/sample/license `include_str!` outside the ONE owner \
         (src/embedded_docs.rs):\n{}",
        offenders.join("\n")
    );
}

/// The committed test-owned fixtures that the corpus/index/ranking unit tests
/// name (instead of the real `README.md`) must exist on disk — so the fixture
/// is a genuine artifact, not a phantom string.
#[test]
fn test_fixtures_exist() {
    let root = repo_root();
    for rel in ["tests/fixtures/doc-fixture.md"] {
        let p = root.join(rel);
        assert!(p.exists(), "missing committed test fixture: {rel} ({p:?})");
    }
}
