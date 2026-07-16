//! Neutral inline tree → styled text/image pieces. Unsupported scalars are
//! replaced before shaping, with their original value retained as ActualText.

use super::super::model::Inline;
use super::fonts::{FontRole, fallback_char, has_glyph};

#[derive(Clone, Debug)]
pub(super) struct Style {
    pub role: FontRole,
    pub size: f32,
    pub leading: f32,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    pub code: bool,
    pub link: Option<String>,
    pub actual: Option<String>,
}

impl Style {
    pub fn body() -> Self {
        Self {
            role: FontRole::Serif,
            size: 11.0,
            leading: 16.0,
            italic: false,
            strike: false,
            highlight: false,
            code: false,
            link: None,
            actual: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Piece {
    Text {
        text: String,
        style: Style,
    },
    Image {
        src: String,
        alt: String,
        width_hint: Option<u32>,
    },
}

pub(super) fn flatten(inlines: &[Inline], base: Style) -> Vec<Piece> {
    let mut out = Vec::new();
    visit(inlines, &base, &mut out);
    out
}

fn visit(inlines: &[Inline], style: &Style, out: &mut Vec<Piece>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_checked(out, text, style),
            Inline::Strong(children) => {
                let mut next = style.clone();
                next.role = match next.role {
                    FontRole::Serif | FontRole::SerifBold => FontRole::SerifBold,
                    FontRole::Mono | FontRole::MonoBold => FontRole::MonoBold,
                };
                visit(children, &next, out);
            }
            Inline::Emphasis(children) => {
                let mut next = style.clone();
                next.italic = true;
                visit(children, &next, out);
            }
            Inline::Strikethrough(children) => {
                let mut next = style.clone();
                next.strike = true;
                visit(children, &next, out);
            }
            Inline::Highlight(children) => {
                let mut next = style.clone();
                next.highlight = true;
                visit(children, &next, out);
            }
            Inline::Code(code) => {
                let mut next = style.clone();
                next.role = if style.role == FontRole::SerifBold {
                    FontRole::MonoBold
                } else {
                    FontRole::Mono
                };
                next.code = true;
                push_checked(out, code, &next);
            }
            Inline::Link { url, children } => {
                let mut next = style.clone();
                next.link = Some(url.clone());
                visit(children, &next, out);
            }
            Inline::Image {
                src,
                alt,
                width_hint,
            } => out.push(Piece::Image {
                src: src.clone(),
                alt: alt.clone(),
                width_hint: *width_hint,
            }),
            Inline::SoftBreak => push_checked(out, " ", style),
            Inline::HardBreak => push_checked(out, "\n", style),
        }
    }
}

fn push_checked(out: &mut Vec<Piece>, text: &str, style: &Style) {
    let mut supported = String::new();
    let flush = |out: &mut Vec<Piece>, supported: &mut String| {
        if !supported.is_empty() {
            out.push(Piece::Text {
                text: std::mem::take(supported),
                style: style.clone(),
            });
        }
    };
    for ch in text.chars() {
        if ch == '\n' || has_glyph(style.role, ch) {
            supported.push(ch);
        } else {
            flush(out, &mut supported);
            let mut replacement = style.clone();
            replacement.actual = Some(ch.to_string());
            out.push(Piece::Text {
                text: fallback_char(style.role).to_string(),
                style: replacement,
            });
        }
    }
    flush(out, &mut supported);
}
