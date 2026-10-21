//! Source files, spans and diagnostics.

use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(lo: u32, hi: u32) -> Span {
        Span { lo, hi }
    }
    pub fn to(self, other: Span) -> Span {
        Span { lo: self.lo.min(other.lo), hi: self.hi.max(other.hi) }
    }
}

pub struct SourceFile {
    pub path: String,
    pub text: String,
    pub base: u32,
}

/// All loaded files share one global offset space so a `Span` alone identifies a location.
#[derive(Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, path: &str, text: String) -> u32 {
        let base = self.files.last().map(|f| f.base + f.text.len() as u32 + 1).unwrap_or(0);
        self.files.push(SourceFile { path: path.to_string(), text, base });
        base
    }

    pub fn file_of(&self, pos: u32) -> &SourceFile {
        self.files.iter().rev().find(|f| f.base <= pos).expect("span outside source map")
    }

    /// 1-based line and column (in characters) of a global offset.
    pub fn line_col(&self, pos: u32) -> (usize, usize) {
        let f = self.file_of(pos);
        let off = ((pos - f.base) as usize).min(f.text.len());
        let before = &f.text[..off];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        (line, col)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Syntax,
    Type,
    Compile,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub kind: Kind,
    pub msg: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diag {
    pub fn new(kind: Kind, span: Span, msg: impl Into<String>) -> Diag {
        Diag { kind, msg: msg.into(), span, help: None }
    }
    pub fn syntax(span: Span, msg: impl Into<String>) -> Diag {
        Diag::new(Kind::Syntax, span, msg)
    }
    pub fn ty(span: Span, msg: impl Into<String>) -> Diag {
        Diag::new(Kind::Type, span, msg)
    }
    pub fn compile(span: Span, msg: impl Into<String>) -> Diag {
        Diag::new(Kind::Compile, span, msg)
    }
    pub fn help(mut self, h: impl Into<String>) -> Diag {
        self.help = Some(h.into());
        self
    }

    pub fn render(&self, sm: &SourceMap) -> String {
        let kind = match self.kind {
            Kind::Syntax => "syntax error",
            Kind::Type => "type error",
            Kind::Compile => "compile error",
        };
        let mut out = String::new();
        if sm.files.is_empty() || self.span == Span::default() {
            let _ = writeln!(out, "{kind}: {}", self.msg);
            return out;
        }
        let f = sm.file_of(self.span.lo);
        let (line, col) = sm.line_col(self.span.lo);
        let _ = writeln!(out, "{kind}: {}", self.msg);
        let _ = writeln!(out, "  --> {}:{}:{}", f.path, line, col);
        if let Some(src) = f.text.lines().nth(line - 1) {
            let width = (self.span.hi.saturating_sub(self.span.lo) as usize)
                .clamp(1, src.chars().count().saturating_sub(col - 1).max(1));
            let _ = writeln!(out, "{:>4} | {}", line, src);
            let _ = writeln!(out, "     | {}{}", " ".repeat(col - 1), "^".repeat(width));
        }
        if let Some(h) = &self.help {
            let _ = writeln!(out, "  help: {h}");
        }
        out
    }
}

pub type Result<T> = std::result::Result<T, Diag>;
