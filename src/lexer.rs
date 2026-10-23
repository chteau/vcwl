//! Tokenizer. Newlines are not tokens; each token records whether a newline preceded it,
//! which the parser uses as a statement terminator.

use crate::diag::{Diag, Result, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    /// A number with a unit suffix, e.g. `12px`, `50%`, `200ms`.
    Dim(f64, String),
    /// `#rrggbb` style color literal (digits only, without `#`).
    Color(String),
    Str(Vec<StrPiece>),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPiece {
    Lit(String),
    /// Tokens of an interpolated `{expr}`.
    Expr(Vec<Token>, Span),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// A newline appears between the previous token and this one.
    pub nl: bool,
}

const PUNCTS: &[&str] = &[
    "..=", "...", "->", "=>", "|>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=",
    "..", "(", ")", "{", "}", "[", "]", ",", ".", ":", ";", "=", "<", ">", "+", "-", "*", "/", "%",
    "!", "?", "|", "&", "@",
];

pub fn lex(src: &str, base: u32) -> Result<Vec<Token>> {
    let mut lx = Lexer { src, pos: 0, base };
    let mut out = Vec::new();
    let mut nl = true;
    loop {
        nl |= lx.skip_trivia()?;
        let start = lx.pos;
        let Some(c) = lx.peek() else {
            out.push(Token {
                tok: Tok::Eof,
                span: lx.span(start),
                nl: true,
            });
            return Ok(out);
        };
        let tok = if c.is_ascii_digit() {
            let after_dot = matches!(
                out.last(),
                Some(Token {
                    tok: Tok::Punct("."),
                    ..
                })
            );
            lx.number(after_dot)?
        } else if c.is_alphabetic() || c == '_' {
            while lx.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                lx.bump();
            }
            Tok::Ident(src[start..lx.pos].to_string())
        } else if c == '"' {
            lx.string()?
        } else if c == '#' {
            lx.bump();
            let s = lx.pos;
            while lx.peek().is_some_and(|c| c.is_ascii_alphanumeric()) {
                lx.bump();
            }
            let hex = &src[s..lx.pos];
            if ![3, 4, 6, 8].contains(&hex.len()) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(Diag::syntax(
                    lx.span(start),
                    format!("invalid color literal `#{hex}`"),
                )
                .help("colors are written as #rgb, #rgba, #rrggbb or #rrggbbaa"));
            }
            Tok::Color(hex.to_ascii_lowercase())
        } else if let Some(p) = PUNCTS.iter().find(|p| src[lx.pos..].starts_with(**p)) {
            lx.pos += p.len();
            Tok::Punct(p)
        } else {
            return Err(Diag::syntax(
                lx.span_len(start, c.len_utf8()),
                format!("unexpected character `{c}`"),
            ));
        };
        out.push(Token {
            tok,
            span: lx.span(start),
            nl,
        });
        nl = false;
    }
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    base: u32,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }
    fn peek2(&self) -> Option<char> {
        self.src[self.pos..].chars().nth(1)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start as u32, self.base + self.pos as u32)
    }
    fn span_len(&self, start: usize, len: usize) -> Span {
        Span::new(self.base + start as u32, self.base + (start + len) as u32)
    }

    /// Skips whitespace and comments; returns whether a newline was crossed.
    fn skip_trivia(&mut self) -> Result<bool> {
        let mut nl = false;
        loop {
            match self.peek() {
                Some('\n') => {
                    nl = true;
                    self.bump();
                }
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some('/') if self.peek2() == Some('*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        if self.src[self.pos..].starts_with("*/") {
                            self.pos += 2;
                            break;
                        }
                        match self.bump() {
                            Some('\n') => nl = true,
                            Some(_) => {}
                            None => {
                                return Err(Diag::syntax(
                                    self.span_len(start, 2),
                                    "unterminated block comment",
                                ));
                            }
                        }
                    }
                }
                _ => return Ok(nl),
            }
        }
    }

    fn number(&mut self, after_dot: bool) -> Result<Tok> {
        let start = self.pos;
        let mut float = false;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
            self.bump();
        }
        if !after_dot
            && self.peek() == Some('.')
            && self.peek2().is_some_and(|c| c.is_ascii_digit())
        {
            float = true;
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                self.bump();
            }
        }
        let text: String = self.src[start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let unit_start = self.pos;
        if self.peek() == Some('%') {
            self.bump();
        } else {
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.bump();
            }
        }
        let unit = &self.src[unit_start..self.pos];
        if !unit.is_empty() {
            return Ok(Tok::Dim(text.parse().unwrap(), unit.to_string()));
        }
        if float {
            Ok(Tok::Float(text.parse().unwrap()))
        } else {
            text.parse::<i64>().map(Tok::Int).map_err(|_| {
                Diag::syntax(self.span(start), "integer literal does not fit in 64 bits")
            })
        }
    }

    fn string(&mut self) -> Result<Tok> {
        let start = self.pos;
        self.bump();
        let mut pieces = Vec::new();
        let mut lit = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err(Diag::syntax(
                    self.span(start),
                    "unterminated string literal",
                ));
            };
            match c {
                '"' => break,
                '\\' => {
                    let esc = self.bump();
                    lit.push(match esc {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('0') => '\0',
                        Some(c @ ('\\' | '"' | '{' | '}')) => c,
                        _ => {
                            return Err(Diag::syntax(
                                self.span_len(self.pos - 2, 2),
                                "unknown escape sequence",
                            )
                            .help("supported escapes: \\n \\t \\r \\0 \\\\ \\\" \\{ \\}"));
                        }
                    });
                }
                '{' => {
                    let inner_start = self.pos;
                    let mut depth = 1;
                    while depth > 0 {
                        match self.bump() {
                            Some('{') => depth += 1,
                            Some('}') => depth -= 1,
                            Some('"') => {
                                return Err(Diag::syntax(
                                    self.span(inner_start),
                                    "string literals are not allowed inside interpolation",
                                )
                                .help("bind the value with `let` first"));
                            }
                            Some(_) => {}
                            None => {
                                return Err(Diag::syntax(
                                    self.span(start),
                                    "unterminated interpolation in string",
                                ));
                            }
                        }
                    }
                    let inner = &self.src[inner_start..self.pos - 1];
                    let span = Span::new(
                        self.base + inner_start as u32,
                        self.base + self.pos as u32 - 1,
                    );
                    if inner.trim().is_empty() {
                        return Err(Diag::syntax(span, "empty interpolation `{}`")
                            .help("write `\\{` for a literal brace"));
                    }
                    let mut toks = lex(inner, self.base + inner_start as u32)?;
                    toks.last_mut().unwrap().span = Span::new(span.hi, span.hi);
                    if !lit.is_empty() {
                        pieces.push(StrPiece::Lit(std::mem::take(&mut lit)));
                    }
                    pieces.push(StrPiece::Expr(toks, span));
                }
                '}' => {
                    return Err(Diag::syntax(
                        self.span_len(self.pos - 1, 1),
                        "unmatched `}` in string",
                    )
                    .help("write `\\}` for a literal brace"));
                }
                c => lit.push(c),
            }
        }
        if !lit.is_empty() || pieces.is_empty() {
            pieces.push(StrPiece::Lit(lit));
        }
        Ok(Tok::Str(pieces))
    }
}
