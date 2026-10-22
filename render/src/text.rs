//! Text measurement (delegated to the host) with a cache, and white-space handling.

use std::collections::HashMap;
use std::rc::Rc;

use crate::style::{Case, Style, WhiteSpace};

/// Measures the advance width of `text` in a CSS `font` with extra letter spacing.
pub trait Measure {
    fn width(&mut self, font: &str, spacing: f32, text: &str) -> f32;
}

/// Measured widths by font (with letter spacing) and text.
#[derive(Default)]
pub struct Fonts {
    cache: HashMap<(Rc<str>, u32), HashMap<String, f32>>,
    entries: usize,
}

impl Fonts {
    pub fn width(&mut self, m: &mut dyn Measure, s: &Style, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let key = (s.font.clone(), s.letter_spacing.to_bits());
        if let Some(&w) = self.cache.get(&key).and_then(|c| c.get(text)) {
            return w;
        }
        let w = m.width(&s.font, s.letter_spacing, text);
        if self.entries > 50_000 {
            self.cache.clear();
            self.entries = 0;
        }
        self.entries += 1;
        self.cache
            .entry(key)
            .or_default()
            .insert(text.to_string(), w);
        w
    }
}

/// Applies `text-transform`.
pub fn transform(s: &Style, text: &str) -> String {
    match s.case {
        Case::None => text.to_string(),
        Case::Upper => text.to_uppercase(),
        Case::Lower => text.to_lowercase(),
        Case::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut start = true;
            for c in text.chars() {
                if start && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                } else {
                    out.push(c);
                }
                start = c.is_whitespace();
            }
            out
        }
    }
}

/// A unit of inline text: a word, a collapsible space or a forced line break.
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
    Word(&'a str),
    Space(&'a str),
    Break,
}

/// Splits text into words and spaces following `white-space`.
pub fn tokens(ws: WhiteSpace, text: &str) -> Vec<Token<'_>> {
    let preserve = matches!(ws, WhiteSpace::Pre | WhiteSpace::PreWrap);
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if preserve && c == '\n' {
            out.push(Token::Break);
            rest = &rest[1..];
        } else if c.is_whitespace() {
            let end = rest
                .find(|ch: char| !ch.is_whitespace() || (preserve && ch == '\n'))
                .unwrap_or(rest.len());
            if preserve {
                out.push(Token::Space(&rest[..end]));
            } else if !matches!(out.last(), Some(Token::Space(_))) {
                out.push(Token::Space(" "));
            }
            rest = &rest[end..];
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(Token::Word(&rest[..end]));
            rest = &rest[end..];
        }
    }
    out
}

pub fn wraps(ws: WhiteSpace) -> bool {
    matches!(ws, WhiteSpace::Normal | WhiteSpace::PreWrap)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Every character is `size * 0.5` wide, plus letter spacing.
    pub struct Mono;

    impl Measure for Mono {
        fn width(&mut self, font: &str, spacing: f32, text: &str) -> f32 {
            let size: f32 = font
                .split_whitespace()
                .find_map(|p| p.strip_suffix("px")?.parse().ok())
                .unwrap_or(16.0);
            text.chars().count() as f32 * (size * 0.5 + spacing)
        }
    }

    #[test]
    fn splits() {
        use Token::*;
        assert_eq!(
            tokens(WhiteSpace::Normal, "  a  b\nc "),
            vec![
                Space(" "),
                Word("a"),
                Space(" "),
                Word("b"),
                Space(" "),
                Word("c"),
                Space(" ")
            ]
        );
        assert_eq!(
            tokens(WhiteSpace::Pre, "a  b\nc"),
            vec![Word("a"), Space("  "), Word("b"), Break, Word("c")]
        );
    }

    #[test]
    fn caches() {
        let mut f = Fonts::default();
        let s = Style::default();
        assert_eq!(f.width(&mut Mono, &s, "abcd"), 32.0);
        assert_eq!(f.entries, 1);
        assert_eq!(
            transform(
                &Style {
                    case: Case::Capitalize,
                    ..Style::default()
                },
                "hello big world"
            ),
            "Hello Big World"
        );
    }
}
