//! Parser for the CSS subset the VCWL compiler emits: single-class rules with optional
//! pseudo-classes, wrapped in `@media` / `@container` width and preference conditions.

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// A number with its unit (`""` for plain numbers, `"%"` for percentages).
    Num(f32, String),
    Hash(String),
    Str(String),
    Func(String, Vec<Tok>),
    Comma,
    Slash,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pseudo {
    Hover,
    FocusVisible,
    FocusWithin,
    Active,
    Disabled,
    Checked,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cond {
    MaxWidth(f32),
    MinWidth(f32),
    ContainerMax(f32),
    ContainerMin(f32),
    Dark,
    Light,
    ReducedMotion,
    Print,
}

#[derive(Debug)]
pub struct Rule {
    pub class: String,
    pub pseudo: Vec<Pseudo>,
    pub conds: Vec<Cond>,
    pub decls: Vec<(String, Vec<Tok>)>,
}

/// Splits a declaration value into tokens.
pub fn tokens(s: &str) -> Vec<Tok> {
    let mut p = Lex {
        s: s.as_bytes(),
        i: 0,
    };
    p.list(None)
}

struct Lex<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lex<'_> {
    fn list(&mut self, close: Option<u8>) -> Vec<Tok> {
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if Some(c) == close {
                self.i += 1;
                break;
            }
            match c {
                b' ' | b'\t' | b'\n' => self.i += 1,
                b',' => {
                    self.i += 1;
                    out.push(Tok::Comma);
                }
                b'/' => {
                    self.i += 1;
                    out.push(Tok::Slash);
                }
                b'#' => {
                    self.i += 1;
                    out.push(Tok::Hash(self.word()));
                }
                b'"' | b'\'' => {
                    self.i += 1;
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i] != c {
                        self.i += 1;
                    }
                    out.push(Tok::Str(
                        String::from_utf8_lossy(&self.s[start..self.i]).into_owned(),
                    ));
                    self.i += 1;
                }
                b'0'..=b'9' | b'.' | b'-' | b'+' if self.starts_number() => {
                    let start = self.i;
                    self.i += 1;
                    while self.i < self.s.len()
                        && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.')
                    {
                        self.i += 1;
                    }
                    let n = number(&self.s[start..self.i]);
                    let unit = if self.s.get(self.i) == Some(&b'%') {
                        self.i += 1;
                        "%".to_string()
                    } else {
                        self.word()
                    };
                    out.push(Tok::Num(n, unit));
                }
                _ => {
                    let w = self.word();
                    if w.is_empty() {
                        self.i += 1;
                    } else if self.s.get(self.i) == Some(&b'(') {
                        self.i += 1;
                        let args = self.list(Some(b')'));
                        out.push(Tok::Func(w, args));
                    } else {
                        out.push(Tok::Ident(w));
                    }
                }
            }
        }
        out
    }

    fn starts_number(&self) -> bool {
        let c = self.s[self.i];
        let next = self.s.get(self.i + 1).copied().unwrap_or(0);
        c.is_ascii_digit()
            || (c == b'.' && next.is_ascii_digit())
            || ((c == b'-' || c == b'+') && (next.is_ascii_digit() || next == b'.'))
    }

    fn word(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric()
                || self.s[self.i] == b'-'
                || self.s[self.i] == b'_')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }
}

/// Parses a decimal number such as `-12.5` (the only form the compiler emits). Avoids the
/// standard float parser, which is large in WebAssembly.
fn number(s: &[u8]) -> f32 {
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (mut v, mut scale, mut frac) = (0f64, 1f64, false);
    for &c in digits {
        match c {
            b'.' => frac = true,
            b'0'..=b'9' if frac => {
                scale /= 10.0;
                v += (c - b'0') as f64 * scale;
            }
            b'0'..=b'9' => v = v * 10.0 + (c - b'0') as f64,
            _ => break,
        }
    }
    (if neg { -v } else { v }) as f32
}

/// Formats a number with at most two decimals, without the standard float formatter.
pub fn num(v: f32) -> String {
    let neg = v < 0.0;
    let hundredths = (v.abs() * 100.0).round() as u64;
    let (int, frac) = (hundredths / 100, hundredths % 100);
    let sign = if neg && hundredths > 0 { "-" } else { "" };
    match frac {
        0 => format!("{sign}{int}"),
        f if f % 10 == 0 => format!("{sign}{int}.{}", f / 10),
        f => format!("{sign}{int}.{f:02}"),
    }
}

/// Parses a style sheet. Rules this renderer cannot match are skipped.
pub fn parse_sheet(src: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    block(src, &[], &mut rules);
    rules
}

fn block(src: &str, conds: &[Cond], out: &mut Vec<Rule>) {
    let mut rest = src;
    loop {
        rest = rest.trim_start();
        let Some(open) = rest.find('{') else { return };
        let prelude = rest[..open].trim();
        let Some(close) = matching(rest, open) else {
            return;
        };
        let body = &rest[open + 1..close];
        rest = &rest[close + 1..];
        if let Some(at) = prelude.strip_prefix('@') {
            if let Some(c) = condition(at) {
                let mut inner = conds.to_vec();
                inner.push(c);
                block(body, &inner, out);
            }
        } else if let Some(sel) = prelude.strip_prefix('.') {
            let mut parts = sel.split(':');
            let class = parts.next().unwrap_or("").to_string();
            let mut pseudo = Vec::new();
            let mut known = true;
            for p in parts {
                match p {
                    "hover" => pseudo.push(Pseudo::Hover),
                    "focus-visible" => pseudo.push(Pseudo::FocusVisible),
                    "focus-within" => pseudo.push(Pseudo::FocusWithin),
                    "active" => pseudo.push(Pseudo::Active),
                    "disabled" => pseudo.push(Pseudo::Disabled),
                    "checked" => pseudo.push(Pseudo::Checked),
                    _ => known = false,
                }
            }
            if known {
                out.push(Rule {
                    class,
                    pseudo,
                    conds: conds.to_vec(),
                    decls: decls(body),
                });
            }
        }
    }
}

fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < open) {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open_q)) if q == open_q => quote = None,
            (_, Some(_)) => {}
            ('{', None) => depth += 1,
            ('}', None) => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn condition(at: &str) -> Option<Cond> {
    let (kind, q) = at.split_once(' ')?;
    let q = q.trim();
    if kind == "media" && q == "print" {
        return Some(Cond::Print);
    }
    let inner = q.strip_prefix('(')?.strip_suffix(')')?;
    let (name, value) = inner.split_once(':')?;
    let container = kind == "container";
    match name.trim() {
        "max-width" | "min-width" => {
            let toks = tokens(value);
            let [Tok::Num(n, unit)] = toks.as_slice() else {
                return None;
            };
            let px = match unit.as_str() {
                "px" => *n,
                "rem" | "em" => n * 16.0,
                _ => return None,
            };
            Some(match (name.trim() == "max-width", container) {
                (true, false) => Cond::MaxWidth(px),
                (false, false) => Cond::MinWidth(px),
                (true, true) => Cond::ContainerMax(px),
                (false, true) => Cond::ContainerMin(px),
            })
        }
        "prefers-color-scheme" if value.trim() == "dark" => Some(Cond::Dark),
        "prefers-color-scheme" => Some(Cond::Light),
        "prefers-reduced-motion" => Some(Cond::ReducedMotion),
        _ => None,
    }
}

fn decls(body: &str) -> Vec<(String, Vec<Tok>)> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    let bytes = body.as_bytes();
    for i in 0..=bytes.len() {
        let c = bytes.get(i).copied().unwrap_or(b';');
        match c {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => {
                if let Some((k, v)) = body[start..i].split_once(':') {
                    out.push((k.trim().to_string(), tokens(v.trim())));
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(
            tokens("8px 12px"),
            vec![Tok::Num(8.0, "px".into()), Tok::Num(12.0, "px".into())]
        );
        assert_eq!(tokens("-0.5em"), vec![Tok::Num(-0.5, "em".into())]);
        assert_eq!(tokens("#b6ff6c"), vec![Tok::Hash("b6ff6c".into())]);
        assert_eq!(
            tokens("repeat(3,minmax(0,1fr))"),
            vec![Tok::Func(
                "repeat".into(),
                vec![
                    Tok::Num(3.0, "".into()),
                    Tok::Comma,
                    Tok::Func(
                        "minmax".into(),
                        vec![
                            Tok::Num(0.0, "".into()),
                            Tok::Comma,
                            Tok::Num(1.0, "fr".into())
                        ]
                    ),
                ]
            )]
        );
        assert_eq!(
            tokens("\"Inter\",sans-serif"),
            vec![
                Tok::Str("Inter".into()),
                Tok::Comma,
                Tok::Ident("sans-serif".into())
            ]
        );
        assert_eq!(tokens("50%"), vec![Tok::Num(50.0, "%".into())]);
        assert_eq!(tokens("599.98px"), vec![Tok::Num(599.98, "px".into())]);
        assert_eq!(
            [num(1.5), num(-0.25), num(16.0), num(2.0 / 3.0), num(-0.001)],
            ["1.5", "-0.25", "16", "0.67", "0"]
        );
    }

    #[test]
    fn sheet() {
        let r = parse_sheet(
            ".s1{padding:8px;color:#fff}.s1:hover{color:#000}\n@media (max-width:599.98px){.s1:focus-visible{gap:1px}}@media print{.s2{color:red}}.s3::before{x:y}",
        );
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].class, "s1");
        assert_eq!(r[0].decls.len(), 2);
        assert_eq!(r[1].pseudo, vec![Pseudo::Hover]);
        assert_eq!(r[2].conds, vec![Cond::MaxWidth(599.98)]);
        assert_eq!(r[2].pseudo, vec![Pseudo::FocusVisible]);
        assert_eq!(r[3].conds, vec![Cond::Print]);
    }

    #[test]
    fn nested_conditions() {
        let r = parse_sheet(
            "@media (prefers-color-scheme:dark){@container (min-width:30rem){.a{color:#fff}}}",
        );
        assert_eq!(r[0].conds, vec![Cond::Dark, Cond::ContainerMin(480.0)]);
    }
}
