//! Typed style declarations compiled to static CSS classes.
//!
//! Every `style { ... }` block becomes one class whose name is derived from its content.
//! Responsive (`when viewport < 600px`) and state (`when hover`) conditions become media
//! queries and pseudo-classes, so the browser evaluates them natively.

use std::collections::HashMap;
use std::fmt::Write;

use crate::ast::{Ident, StyleBlock};
use crate::check::suggest;
use crate::diag::{Diag, Result, Span};
use crate::lexer::{Tok, Token};

/// Element arguments that take style values rather than expressions, e.g. `column(gap: 20px)`.
pub fn is_style_arg(name: &str) -> bool {
    matches!(name, "gap" | "align" | "justify" | "padding" | "wrap" | "columns" | "width" | "height" | "grow")
}

pub const BASE_CSS: &str = "*,*::before,*::after{box-sizing:border-box}\
body{margin:0;font-family:system-ui,-apple-system,'Segoe UI',sans-serif;line-height:1.5}\
.v-col{display:flex;flex-direction:column}\
.v-row{display:flex;flex-direction:row}\
.v-stack{display:grid}.v-stack>*{grid-area:1/1}\
.v-grid{display:grid}\
.v-scroll{overflow:auto}\
.v-list{margin:0;padding-left:1.25em}\
.v-check{display:inline-flex;gap:.5em;align-items:center;cursor:pointer}\
.v-input{font:inherit}\
button{font:inherit;cursor:pointer}\
:focus-visible{outline:2px solid #4c9ffe;outline-offset:2px}\
@media (prefers-reduced-motion:reduce){*,*::before,*::after{transition:none!important;animation:none!important;scroll-behavior:auto!important}}\n";

#[derive(Default)]
pub struct Sheet {
    pub css: String,
    classes: HashMap<String, String>,
    /// Class of each top-level named style, by declaration order.
    pub named: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Len(f64, String),
    Num(f64),
    Color(String),
    Kw(String),
    Str(String),
    Func(String, Vec<Val>),
    Comma,
    Slash,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum VTy {
    Length,
    Color,
    Number,
    Keyword,
}

struct Prop {
    css: &'static [&'static str],
    kind: PropKind,
}

enum PropKind {
    /// 1 to `max` lengths (with optional extra keywords like `auto`).
    Lengths(usize, &'static [&'static str]),
    Color,
    Number(f64, f64),
    Keyword(&'static [(&'static str, &'static str)]),
    FontFamily,
    Border,
    Shadow,
    Transform,
    Transition,
    Tracks,
    Integer(i64, i64),
    Weight,
    LineHeight,
    Background,
}

const LEN_AUTO: &[&str] = &["auto"];
const LEN_SIZE: &[&str] = &["auto", "min-content", "max-content", "fit-content"];
const ALIGN: &[(&str, &str)] = &[("start", "flex-start"), ("center", "center"), ("end", "flex-end"), ("stretch", "stretch"), ("baseline", "baseline")];
const JUSTIFY: &[(&str, &str)] = &[
    ("start", "flex-start"), ("center", "center"), ("end", "flex-end"), ("between", "space-between"), ("around", "space-around"),
    ("evenly", "space-evenly"), ("stretch", "stretch"),
];
const OVERFLOW: &[(&str, &str)] = &[("visible", "visible"), ("hidden", "hidden"), ("auto", "auto"), ("scroll", "scroll"), ("clip", "clip")];

fn prop(name: &str) -> Option<Prop> {
    use PropKind::*;
    let p = |css, kind| Some(Prop { css, kind });
    match name {
        "color" => p(&["color"], Color),
        "background" => p(&["background"], Background),
        "width" => p(&["width"], Lengths(1, LEN_SIZE)),
        "height" => p(&["height"], Lengths(1, LEN_SIZE)),
        "min-width" => p(&["min-width"], Lengths(1, LEN_SIZE)),
        "max-width" => p(&["max-width"], Lengths(1, &["none"])),
        "min-height" => p(&["min-height"], Lengths(1, LEN_SIZE)),
        "max-height" => p(&["max-height"], Lengths(1, &["none"])),
        "padding" => p(&["padding"], Lengths(4, &[])),
        "padding-x" => p(&["padding-inline"], Lengths(2, &[])),
        "padding-y" => p(&["padding-block"], Lengths(2, &[])),
        "padding-top" => p(&["padding-top"], Lengths(1, &[])),
        "padding-bottom" => p(&["padding-bottom"], Lengths(1, &[])),
        "padding-left" => p(&["padding-left"], Lengths(1, &[])),
        "padding-right" => p(&["padding-right"], Lengths(1, &[])),
        "margin" => p(&["margin"], Lengths(4, LEN_AUTO)),
        "margin-x" => p(&["margin-inline"], Lengths(2, LEN_AUTO)),
        "margin-y" => p(&["margin-block"], Lengths(2, LEN_AUTO)),
        "margin-top" => p(&["margin-top"], Lengths(1, LEN_AUTO)),
        "margin-bottom" => p(&["margin-bottom"], Lengths(1, LEN_AUTO)),
        "margin-left" => p(&["margin-left"], Lengths(1, LEN_AUTO)),
        "margin-right" => p(&["margin-right"], Lengths(1, LEN_AUTO)),
        "gap" => p(&["gap"], Lengths(2, &[])),
        "row-gap" => p(&["row-gap"], Lengths(1, &[])),
        "column-gap" => p(&["column-gap"], Lengths(1, &[])),
        "border" => p(&["border"], Border),
        "border-top" => p(&["border-top"], Border),
        "border-bottom" => p(&["border-bottom"], Border),
        "border-left" => p(&["border-left"], Border),
        "border-right" => p(&["border-right"], Border),
        "radius" => p(&["border-radius"], Lengths(4, &[])),
        "font" => p(&["font-family"], FontFamily),
        "size" => p(&["font-size"], Lengths(1, &[])),
        "weight" => p(&["font-weight"], Weight),
        "line-height" => p(&["line-height"], LineHeight),
        "letter-spacing" => p(&["letter-spacing"], Lengths(1, &["normal"])),
        "italic" => p(&["font-style"], Keyword(&[("yes", "italic"), ("no", "normal")])),
        "text-align" => p(&["text-align"], Keyword(&[("left", "left"), ("center", "center"), ("right", "right"), ("justify", "justify"), ("start", "start"), ("end", "end")])),
        "decoration" => p(&["text-decoration"], Keyword(&[("none", "none"), ("underline", "underline"), ("line-through", "line-through")])),
        "case" => p(&["text-transform"], Keyword(&[("upper", "uppercase"), ("lower", "lowercase"), ("capitalize", "capitalize"), ("none", "none")])),
        "white-space" => p(&["white-space"], Keyword(&[("normal", "normal"), ("nowrap", "nowrap"), ("pre", "pre"), ("pre-wrap", "pre-wrap")])),
        "direction" => p(&["flex-direction"], Keyword(&[("row", "row"), ("column", "column"), ("row-reverse", "row-reverse"), ("column-reverse", "column-reverse")])),
        "align" => p(&["align-items"], Keyword(ALIGN)),
        "align-self" => p(&["align-self"], Keyword(ALIGN)),
        "justify" => p(&["justify-content"], Keyword(JUSTIFY)),
        "wrap" => p(&["flex-wrap"], Keyword(&[("yes", "wrap"), ("no", "nowrap"), ("wrap", "wrap"), ("nowrap", "nowrap")])),
        "grow" => p(&["flex-grow"], Number(0.0, 1e6)),
        "shrink" => p(&["flex-shrink"], Number(0.0, 1e6)),
        "basis" => p(&["flex-basis"], Lengths(1, LEN_AUTO)),
        "display" => p(&["display"], Keyword(&[
            ("flex", "flex"), ("grid", "grid"), ("block", "block"), ("inline", "inline"), ("inline-block", "inline-block"),
            ("inline-flex", "inline-flex"), ("none", "none"), ("contents", "contents"),
        ])),
        "columns" => p(&["grid-template-columns"], Tracks),
        "rows" => p(&["grid-template-rows"], Tracks),
        "span" => p(&["grid-column"], Integer(1, 1000)),
        "position" => p(&["position"], Keyword(&[("static", "static"), ("relative", "relative"), ("absolute", "absolute"), ("fixed", "fixed"), ("sticky", "sticky")])),
        "top" => p(&["top"], Lengths(1, LEN_AUTO)),
        "right" => p(&["right"], Lengths(1, LEN_AUTO)),
        "bottom" => p(&["bottom"], Lengths(1, LEN_AUTO)),
        "left" => p(&["left"], Lengths(1, LEN_AUTO)),
        "inset" => p(&["inset"], Lengths(4, LEN_AUTO)),
        "z" => p(&["z-index"], Integer(-100000, 100000)),
        "overflow" => p(&["overflow"], Keyword(OVERFLOW)),
        "overflow-x" => p(&["overflow-x"], Keyword(OVERFLOW)),
        "overflow-y" => p(&["overflow-y"], Keyword(OVERFLOW)),
        "opacity" => p(&["opacity"], Number(0.0, 1.0)),
        "transform" => p(&["transform"], Transform),
        "transition" => p(&["transition"], Transition),
        "cursor" => p(&["cursor"], Keyword(&[("pointer", "pointer"), ("default", "default"), ("text", "text"), ("not-allowed", "not-allowed"), ("grab", "grab"), ("move", "move")])),
        "shadow" => p(&["box-shadow"], Shadow),
        "fit" => p(&["object-fit"], Keyword(&[("cover", "cover"), ("contain", "contain"), ("fill", "fill"), ("none", "none")])),
        "container" => p(&["container-type"], Keyword(&[("inline", "inline-size"), ("size", "size"), ("none", "normal")])),
        "user-select" => p(&["user-select"], Keyword(&[("none", "none"), ("text", "text"), ("all", "all"), ("auto", "auto")])),
        _ => None,
    }
}

const PROPS: &[&str] = &[
    "color", "background", "width", "height", "min-width", "max-width", "min-height", "max-height", "padding", "padding-x", "padding-y",
    "margin", "margin-x", "margin-y", "gap", "row-gap", "column-gap", "border", "radius", "font", "size", "weight", "line-height",
    "letter-spacing", "italic", "text-align", "decoration", "case", "white-space", "direction", "align", "align-self", "justify",
    "wrap", "grow", "shrink", "basis", "display", "columns", "rows", "span", "position", "top", "right", "bottom", "left", "inset",
    "z", "overflow", "overflow-x", "overflow-y", "opacity", "transform", "transition", "cursor", "shadow", "fit", "container",
    "user-select",
];

const NAMED_COLORS: &[&str] = &["transparent", "currentcolor", "white", "black", "inherit"];
const LENGTH_UNITS: &[&str] = &["px", "rem", "em", "%", "vh", "vw", "vmin", "vmax", "ch", "dvh", "svh", "fr"];

fn num(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

impl Val {
    fn css(&self) -> String {
        match self {
            Val::Len(n, u) => format!("{}{u}", num(*n)),
            Val::Num(n) => num(*n),
            Val::Color(c) | Val::Kw(c) => c.clone(),
            Val::Str(s) => format!("\"{}\"", s.replace(['"', '\\', '<', '\n'], "")),
            Val::Func(f, args) => {
                let mut s = format!("{f}(");
                let mut first = true;
                for a in args {
                    if *a == Val::Comma {
                        s.push(',');
                        first = true;
                        continue;
                    }
                    if !first {
                        s.push(' ');
                    }
                    s.push_str(&a.css());
                    first = false;
                }
                s.push(')');
                s
            }
            Val::Comma => ",".into(),
            Val::Slash => "/".into(),
        }
    }
    fn ty(&self) -> Option<VTy> {
        match self {
            Val::Len(_, u) if u != "fr" => Some(VTy::Length),
            Val::Num(n) if *n == 0.0 => Some(VTy::Length),
            Val::Num(_) => Some(VTy::Number),
            Val::Color(_) => Some(VTy::Color),
            Val::Func(f, _) if matches!(f.as_str(), "rgb" | "rgba" | "hsl" | "hsla") => Some(VTy::Color),
            Val::Kw(_) => Some(VTy::Keyword),
            _ => None,
        }
    }
    fn describe(&self) -> String {
        match self {
            Val::Len(n, u) => format!("length `{}{u}`", num(*n)),
            Val::Num(n) => format!("number `{}`", num(*n)),
            Val::Color(c) => format!("color `{c}`"),
            Val::Kw(k) => format!("keyword `{k}`"),
            Val::Str(_) => "a string".into(),
            Val::Func(f, _) => format!("`{f}(...)`"),
            Val::Comma => "`,`".into(),
            Val::Slash => "`/`".into(),
        }
    }
}

/// Parses raw value tokens into style values, joining hyphenated identifiers.
fn parse_vals(toks: &[Token]) -> Result<Vec<(Val, Span)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let span = t.span;
        let v = match &t.tok {
            Tok::Dim(n, u) => {
                if !LENGTH_UNITS.contains(&u.as_str()) && !matches!(u.as_str(), "ms" | "s" | "deg" | "turn") {
                    return Err(Diag::ty(span, format!("unknown unit `{u}`")).help(format!("supported units: {}, ms, s, deg", LENGTH_UNITS.join(", "))));
                }
                Val::Len(*n, u.clone())
            }
            Tok::Int(n) => Val::Num(*n as f64),
            Tok::Float(n) => Val::Num(*n),
            Tok::Punct("-") if matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Dim(..) | Tok::Int(_) | Tok::Float(_))) => {
                i += 1;
                match &toks[i].tok {
                    Tok::Dim(n, u) => Val::Len(-n, u.clone()),
                    Tok::Int(n) => Val::Num(-(*n as f64)),
                    Tok::Float(n) => Val::Num(-n),
                    _ => unreachable!(),
                }
            }
            Tok::Color(c) => Val::Color(format!("#{c}")),
            Tok::Str(p) => match p.as_slice() {
                [crate::lexer::StrPiece::Lit(s)] => Val::Str(s.clone()),
                _ => return Err(Diag::ty(span, "style strings cannot contain interpolation")),
            },
            Tok::Punct(",") => Val::Comma,
            Tok::Punct("/") => Val::Slash,
            Tok::Ident(first) => {
                let mut name = first.clone();
                let mut end = span.hi;
                while i + 2 < toks.len() + 1
                    && matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Punct("-")))
                    && toks[i + 1].span.lo == end
                    && let Some(Tok::Ident(next)) = toks.get(i + 2).map(|t| &t.tok)
                    && toks[i + 2].span.lo == toks[i + 1].span.hi
                {
                    name = format!("{name}-{next}");
                    end = toks[i + 2].span.hi;
                    i += 2;
                }
                if matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Punct("("))) {
                    let start = i + 2;
                    let mut depth = 1;
                    let mut j = start;
                    while j < toks.len() {
                        match toks[j].tok {
                            Tok::Punct("(") => depth += 1,
                            Tok::Punct(")") => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if j >= toks.len() {
                        return Err(Diag::ty(span, format!("unclosed `{name}(`")));
                    }
                    let args = parse_vals(&toks[start..j])?.into_iter().map(|(v, _)| v).collect();
                    i = j;
                    Val::Func(name, args)
                } else if NAMED_COLORS.contains(&name.to_ascii_lowercase().as_str()) {
                    Val::Color(name.to_ascii_lowercase())
                } else {
                    Val::Kw(name)
                }
            }
            _ => return Err(Diag::ty(span, "unexpected token in style value")),
        };
        out.push((v, span));
        i += 1;
    }
    Ok(out)
}

impl Sheet {
    /// Compiles a style block into a class (deduplicated by content) and returns its name.
    pub fn block(&mut self, b: &StyleBlock) -> Result<String> {
        let mut body = String::new();
        let mut extra = Vec::new();
        self.block_into(b, &mut body, &mut extra, &[], "")?;
        Ok(self.class(body, extra))
    }

    fn class(&mut self, body: String, extra: Vec<(Vec<String>, String, String)>) -> String {
        let key = format!("{body}|{extra:?}");
        if let Some(c) = self.classes.get(&key) {
            return c.clone();
        }
        let mut h: u32 = 0x811c9dc5;
        for b in key.bytes() {
            h = (h ^ b as u32).wrapping_mul(0x01000193);
        }
        let mut name = format!("s{:x}", h & 0xffffff);
        while self.classes.values().any(|c| *c == name) {
            name.push('x');
        }
        if !body.is_empty() {
            let _ = write!(self.css, ".{name}{{{body}}}");
        }
        for (media, pseudo, decls) in &extra {
            let rule = format!(".{name}{pseudo}{{{decls}}}");
            if media.is_empty() {
                self.css.push_str(&rule);
            } else {
                let mut wrapped = rule;
                for m in media.iter().rev() {
                    wrapped = format!("{m}{{{wrapped}}}");
                }
                self.css.push_str(&wrapped);
            }
        }
        self.css.push('\n');
        self.classes.insert(key, name.clone());
        name
    }

    fn block_into(&mut self, b: &StyleBlock, body: &mut String, extra: &mut Vec<(Vec<String>, String, String)>, media: &[String], pseudo: &str) -> Result<()> {
        let mut seen: Vec<&str> = Vec::new();
        for d in &b.decls {
            if seen.contains(&d.prop.name.as_str()) {
                return Err(Diag::ty(d.prop.span, format!("`{}` is set twice in this block", d.prop.name)));
            }
            seen.push(&d.prop.name);
            body.push_str(&decl(&d.prop, &d.value, d.span)?);
        }
        for (toks, span, inner) in &b.conds {
            let (m, p) = condition(toks, *span)?;
            let mut media2 = media.to_vec();
            media2.extend(m);
            let pseudo2 = format!("{pseudo}{p}");
            let mut ibody = String::new();
            self.block_into(inner, &mut ibody, extra, &media2, &pseudo2)?;
            if !ibody.is_empty() {
                extra.push((media2, pseudo2, ibody));
            }
        }
        Ok(())
    }

    /// Compiles layout arguments such as `column(gap: 20px, align: center)` into a class.
    pub fn layout_args(&mut self, elem: &str, args: &[(Ident, Vec<Token>, Span)]) -> Result<String> {
        let mut body = String::new();
        for (id, toks, span) in args {
            if id.name == "columns" && elem != "grid" {
                return Err(Diag::ty(id.span, "`columns` is only supported on `grid`"));
            }
            body.push_str(&decl(id, toks, *span)?);
        }
        Ok(self.class(body, vec![]))
    }

    pub fn output(&self) -> String {
        format!("{BASE_CSS}{}", self.css)
    }
}

fn condition(toks: &[Token], span: Span) -> Result<(Vec<String>, String)> {
    let vals = parse_vals(toks)?;
    let kw = |v: &Val| if let Val::Kw(k) = v { Some(k.clone()) } else { None };
    if vals.len() == 1
        && let Some(k) = kw(&vals[0].0)
    {
        return Ok(match k.as_str() {
            "hover" => (vec![], ":hover".into()),
            "focus" => (vec![], ":focus-visible".into()),
            "focus-within" => (vec![], ":focus-within".into()),
            "active" => (vec![], ":active".into()),
            "disabled" => (vec![], ":disabled".into()),
            "checked" => (vec![], ":checked".into()),
            "reduced-motion" => (vec!["@media (prefers-reduced-motion:reduce)".into()], String::new()),
            "dark" => (vec!["@media (prefers-color-scheme:dark)".into()], String::new()),
            "light" => (vec!["@media (prefers-color-scheme:light)".into()], String::new()),
            "print" => (vec!["@media print".into()], String::new()),
            _ => return Err(cond_err(span)),
        });
    }
    // `viewport < 600px` / `container >= 30rem`
    let ops: Vec<&Token> = toks.iter().filter(|t| matches!(t.tok, Tok::Punct("<" | "<=" | ">" | ">="))).collect();
    let target = toks.first().and_then(|t| if let Tok::Ident(n) = &t.tok { Some(n.as_str()) } else { None });
    if let (Some(target @ ("viewport" | "container")), [op]) = (target, ops.as_slice())
        && let [_, (Val::Len(n, u), _)] = vals.as_slice()
        && u != "%"
        && u != "fr"
    {
        let at = if target == "viewport" { "@media" } else { "@container" };
        let q = match op.tok {
            Tok::Punct("<") => format!("(max-width:{}{u})", num(n - if u == "px" { 0.02 } else { 0.001 })),
            Tok::Punct("<=") => format!("(max-width:{}{u})", num(*n)),
            Tok::Punct(">") => format!("(min-width:{}{u})", num(n + if u == "px" { 0.02 } else { 0.001 })),
            _ => format!("(min-width:{}{u})", num(*n)),
        };
        return Ok((vec![format!("{at} {q}")], String::new()));
    }
    Err(cond_err(span))
}

fn cond_err(span: Span) -> Diag {
    Diag::ty(span, "unsupported style condition")
        .help("use `when viewport < 600px`, `when container >= 30rem`, `when hover`, `when focus`, `when active`, `when disabled`, `when dark` or `when reduced-motion`")
}

fn decl(p: &Ident, toks: &[Token], span: Span) -> Result<String> {
    let Some(spec) = prop(&p.name) else {
        let mut d = Diag::ty(p.span, format!("unknown style property `{}`", p.name));
        if let Some(s) = suggest(&p.name, PROPS.iter().copied()) {
            d = d.help(format!("did you mean `{s}`?"));
        }
        return Err(d);
    };
    let vals = parse_vals(toks)?;
    let value = check_value(&p.name, &spec.kind, &vals, span)?;
    Ok(spec.css.iter().map(|c| format!("{c}:{value};")).collect())
}

fn expect_err(prop: &str, want: &str, got: &(Val, Span)) -> Diag {
    let mut d = Diag::ty(got.1, format!("`{prop}` expects {want}, found {}", got.0.describe()));
    if let Val::Str(_) = got.0 {
        d = d.help("style values are typed; write `12px` or `#ff0000` without quotes");
    }
    d
}

fn check_value(name: &str, kind: &PropKind, vals: &[(Val, Span)], span: Span) -> Result<String> {
    use PropKind::*;
    if vals.is_empty() {
        return Err(Diag::ty(span, format!("`{name}` needs a value")));
    }
    let join = |vs: &[(Val, Span)]| vs.iter().map(|(v, _)| v.css()).collect::<Vec<_>>().join(" ");
    let single = |want: &str| -> Result<&(Val, Span)> {
        if vals.len() != 1 {
            return Err(Diag::ty(vals[1].1, format!("`{name}` takes a single {want}")));
        }
        Ok(&vals[0])
    };
    match kind {
        Lengths(max, kws) => {
            if vals.len() > *max {
                return Err(Diag::ty(vals[*max].1, format!("`{name}` takes at most {max} length(s)")));
            }
            for v in vals {
                let ok = v.0.ty() == Some(VTy::Length) || matches!(&v.0, Val::Kw(k) if kws.contains(&k.as_str()));
                if !ok {
                    let want = if kws.is_empty() { "a length (e.g. `12px`, `1.5rem`, `50%`)".to_string() } else { format!("a length or {}", kws.join("/")) };
                    return Err(expect_err(name, &want, v));
                }
            }
            Ok(join(vals))
        }
        Color => {
            let v = single("color")?;
            if v.0.ty() != Some(VTy::Color) {
                return Err(expect_err(name, "a color (e.g. `#1e90ff`, `transparent`, `rgb(0, 0, 0)`)", v));
            }
            Ok(v.0.css())
        }
        Background => {
            let v = single("color or gradient")?;
            let ok = v.0.ty() == Some(VTy::Color) || matches!(&v.0, Val::Func(f, _) if f == "linear-gradient" || f == "radial-gradient");
            if !ok {
                return Err(expect_err(name, "a color or `linear-gradient(...)`", v));
            }
            Ok(v.0.css())
        }
        Number(lo, hi) => {
            let v = single("number")?;
            match v.0 {
                Val::Num(n) if n >= *lo && n <= *hi => Ok(num(n)),
                Val::Num(n) => Err(Diag::ty(v.1, format!("`{name}` must be between {} and {}, found {}", num(*lo), num(*hi), num(n)))),
                _ => Err(expect_err(name, "a number", v)),
            }
        }
        Integer(lo, hi) => {
            let v = single("integer")?;
            match v.0 {
                Val::Num(n) if n == n.trunc() && n >= *lo as f64 && n <= *hi as f64 => {
                    Ok(if name == "span" { format!("span {}", num(n)) } else { num(n) })
                }
                _ => Err(expect_err(name, &format!("an integer from {lo} to {hi}"), v)),
            }
        }
        Weight => {
            let v = single("font weight")?;
            match &v.0 {
                Val::Num(n) if (1.0..=1000.0).contains(n) => Ok(num(*n)),
                Val::Kw(k) if k == "normal" || k == "bold" => Ok(k.clone()),
                _ => Err(expect_err(name, "a font weight (100–900, `normal` or `bold`)", v)),
            }
        }
        LineHeight => {
            let v = single("line height")?;
            match &v.0 {
                Val::Num(n) if *n >= 0.0 => Ok(num(*n)),
                Val::Len(..) => Ok(v.0.css()),
                _ => Err(expect_err(name, "a number or length", v)),
            }
        }
        Keyword(opts) => {
            let v = single("keyword")?;
            let Val::Kw(k) = &v.0 else {
                return Err(expect_err(name, &format!("one of: {}", opts.iter().map(|o| o.0).collect::<Vec<_>>().join(", ")), v));
            };
            match opts.iter().find(|o| o.0 == k) {
                Some((_, css)) => Ok(css.to_string()),
                None => {
                    let mut d = Diag::ty(v.1, format!("`{name}` does not accept `{k}`"));
                    d = d.help(format!("expected one of: {}", opts.iter().map(|o| o.0).collect::<Vec<_>>().join(", ")));
                    Err(d)
                }
            }
        }
        FontFamily => {
            let mut out = String::new();
            for (v, s) in vals {
                match v {
                    Val::Kw(k) => {
                        if !out.is_empty() && !out.ends_with(',') {
                            out.push(' ');
                        }
                        out.push_str(k);
                    }
                    Val::Str(_) => out.push_str(&v.css()),
                    Val::Comma => out.push(','),
                    other => return Err(expect_err(name, "font family names", &(other.clone(), *s))),
                }
            }
            Ok(out)
        }
        Border => {
            if let [(Val::Kw(k), _)] = vals
                && k == "none"
            {
                return Ok("none".into());
            }
            let (mut w, mut st, mut c) = (None, None, None);
            for v in vals {
                match &v.0 {
                    x if x.ty() == Some(VTy::Length) && w.is_none() => w = Some(x.css()),
                    Val::Kw(k) if ["solid", "dashed", "dotted", "double"].contains(&k.as_str()) && st.is_none() => st = Some(k.clone()),
                    x if x.ty() == Some(VTy::Color) && c.is_none() => c = Some(x.css()),
                    _ => return Err(expect_err(name, "`<width> <solid|dashed|dotted|double> <color>`", v)),
                }
            }
            Ok([w.or(Some("1px".into())), st.or(Some("solid".into())), c].into_iter().flatten().collect::<Vec<_>>().join(" "))
        }
        Shadow => {
            if let [(Val::Kw(k), _)] = vals
                && k == "none"
            {
                return Ok("none".into());
            }
            let mut parts = Vec::new();
            for group in vals.split(|v| v.0 == Val::Comma) {
                let lens: Vec<_> = group.iter().filter(|v| v.0.ty() == Some(VTy::Length)).collect();
                let colors: Vec<_> = group.iter().filter(|v| v.0.ty() == Some(VTy::Color)).collect();
                let inset = group.iter().any(|v| v.0 == Val::Kw("inset".into()));
                if lens.len() < 2 || lens.len() > 4 || colors.len() > 1 || lens.len() + colors.len() + inset as usize != group.len() {
                    return Err(Diag::ty(span, format!("`{name}` expects `<x> <y> [blur] [spread] [color]`, e.g. `0 2px 8px #0003`")));
                }
                parts.push(join(group));
            }
            Ok(parts.join(","))
        }
        Transform => {
            for v in vals {
                let Val::Func(f, args) = &v.0 else { return Err(expect_err(name, "transform functions like `scale(1.1)`", v)) };
                let ok = match f.as_str() {
                    "scale" | "scaleX" | "scaleY" => args.iter().all(|a| matches!(a, Val::Num(_) | Val::Comma)),
                    "rotate" => matches!(args.as_slice(), [Val::Len(_, u)] if u == "deg" || u == "turn"),
                    "translate" | "translateX" | "translateY" => args.iter().all(|a| *a == Val::Comma || a.ty() == Some(VTy::Length)),
                    _ => false,
                };
                if !ok {
                    return Err(Diag::ty(v.1, format!("invalid transform `{}`", v.0.css()))
                        .help("supported: scale(n), rotate(<deg>), translate(<len>, <len>), translateX(<len>), translateY(<len>)"));
                }
            }
            Ok(join(vals))
        }
        Transition => {
            let mut parts = Vec::new();
            for group in vals.split(|v| v.0 == Val::Comma) {
                let mut out = Vec::new();
                for (i, v) in group.iter().enumerate() {
                    let ok = match (&v.0, i) {
                        (Val::Kw(k), 0) => prop(k).map(|p| p.css[0].to_string()).or(if k == "all" { Some("all".into()) } else { None }).map(|c| out.push(c)).is_some(),
                        (Val::Len(_, u), 1 | 3) if u == "ms" || u == "s" => {
                            out.push(v.0.css());
                            true
                        }
                        (Val::Kw(k), 2) if ["ease", "linear", "ease-in", "ease-out", "ease-in-out"].contains(&k.as_str()) => {
                            out.push(k.clone());
                            true
                        }
                        _ => false,
                    };
                    if !ok {
                        return Err(Diag::ty(v.1, format!("invalid transition part {}", v.0.describe()))
                            .help("write `<property> <duration> [easing] [delay]`, e.g. `opacity 200ms ease-out`"));
                    }
                }
                if out.len() < 2 {
                    return Err(Diag::ty(span, "a transition needs a property and a duration, e.g. `opacity 200ms`"));
                }
                parts.push(out.join(" "));
            }
            Ok(parts.join(","))
        }
        Tracks => {
            if let [(Val::Num(n), s)] = vals {
                if *n < 1.0 || *n > 100.0 || *n != n.trunc() {
                    return Err(Diag::ty(*s, "the number of grid tracks must be an integer from 1 to 100"));
                }
                return Ok(format!("repeat({},minmax(0,1fr))", num(*n)));
            }
            if let [(Val::Func(f, args), s)] = vals
                && f == "fill"
            {
                let [Val::Len(n, u)] = args.as_slice() else {
                    return Err(Diag::ty(*s, "`fill(...)` takes a minimum track width, e.g. `fill(220px)`"));
                };
                return Ok(format!("repeat(auto-fill,minmax(min({}{u},100%),1fr))", num(*n)));
            }
            for v in vals {
                let ok = matches!(&v.0, Val::Len(..)) || matches!(&v.0, Val::Kw(k) if k == "auto");
                if !ok {
                    return Err(expect_err(name, "a track count, `fill(<min>)` or a list of lengths (`1fr 2fr 200px`)", v));
                }
            }
            Ok(join(vals))
        }
    }
}
