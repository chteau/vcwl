//! Lints: warnings about code that compiles but probably should not stay as it is. They work
//! per file, on the parsed file and its tokens, so a name counts as used when it appears
//! anywhere else in the file (`vcwl lint`, and the language server's warnings).

use crate::ast::{self, Item, View};
use crate::lexer::{self, StrPiece, Tok, Token};

pub struct Lint {
    /// Byte offsets in the file.
    pub lo: usize,
    pub hi: usize,
    pub code: &'static str,
    pub msg: String,
}

fn count(toks: &[Token], name: &str, n: &mut usize) {
    for t in toks {
        match &t.tok {
            Tok::Ident(s) if s == name => *n += 1,
            Tok::Str(pieces) => {
                for p in pieces {
                    if let StrPiece::Expr(inner, _) = p {
                        count(inner, name, n);
                    }
                }
            }
            _ => {}
        }
    }
}

fn depth(views: &[View]) -> usize {
    views
        .iter()
        .map(|v| match v {
            View::Element(e) => 1 + depth(&e.children),
            View::If { then, els, .. } => depth(then).max(els.as_deref().map_or(0, depth)),
            View::For { body, .. } => depth(body),
            View::Match { arms, .. } => arms.iter().map(|(_, b)| depth(b)).max().unwrap_or(0),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// Placeholder text left in string literals.
const PLACEHOLDERS: &[&str] = &["lorem ipsum", "todo:", "fixme", "asdf"];

/// Lints of the file `text` (nothing when it does not parse: the compiler reports that).
pub fn lint(path: &str, text: &str) -> Vec<Lint> {
    let Ok(toks) = lexer::lex(text, 0) else {
        return Vec::new();
    };
    let Ok(file) = crate::parser::parse_file(path, &toks, &mut 0) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let uses = |name: &str| {
        let mut n = 0;
        count(&toks, name, &mut n);
        n
    };
    let mut unused = |id: &ast::Ident, what: &str, code| {
        if !id.name.starts_with('_') && uses(&id.name) <= 1 {
            out.push(Lint {
                lo: id.span.lo as usize,
                hi: id.span.hi as usize,
                code,
                msg: format!("{what} `{}` is never used", id.name),
            });
        }
    };
    for item in &file.items {
        match item {
            Item::Fn(f)
                if !f.exported
                    && !f.server
                    && f.name.name != "main"
                    && !f.name.name.starts_with("test_") =>
            {
                unused(&f.name, "function", "unused-fn")
            }
            Item::Component(c) => {
                if !c.exported && c.name.name != "App" {
                    unused(&c.name, "component", "unused-component");
                }
                for v in &c.body {
                    match v {
                        View::State { name, .. } => unused(name, "state", "unused-state"),
                        View::Let { name, .. } => unused(name, "value", "unused-let"),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    for item in &file.items {
        if let Item::Component(c) = item {
            let d = depth(&c.body);
            if d > 8 {
                out.push(Lint {
                    lo: c.name.span.lo as usize,
                    hi: c.name.span.hi as usize,
                    code: "deep-view",
                    msg: format!("`{}` nests elements {d} levels deep; split it into components (or seek help)", c.name.name),
                });
            }
        }
    }
    for t in &toks {
        if let Tok::Str(pieces) = &t.tok {
            let lit: String = pieces
                .iter()
                .filter_map(|p| {
                    if let StrPiece::Lit(s) = p {
                        Some(s.to_lowercase())
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(p) = PLACEHOLDERS.iter().find(|p| lit.contains(**p)) {
                out.push(Lint {
                    lo: t.span.lo as usize,
                    hi: t.span.hi as usize,
                    code: "placeholder",
                    msg: format!("placeholder text (`{p}`) is about to ship to production"),
                });
            }
        }
    }
    out.sort_by_key(|l| l.lo);
    out
}

/// `vcwl lint [files]`: compile errors and lints of the given files, or of every `.vcwl` file of
/// the project. Fails when there is any.
pub fn run(args: &[String]) -> Result<(), String> {
    let files: Vec<std::path::PathBuf> = if args.is_empty() {
        let p = crate::project::Project::find(std::path::Path::new("."))?;
        let mut v = Vec::new();
        crate::project::vcwl_files(&p.root, &mut v)?;
        v
    } else {
        args.iter().map(std::path::PathBuf::from).collect()
    };
    let mut problems = 0;
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let shown = f.display().to_string();
        for l in lint(&shown, &text) {
            let line = text[..l.lo].matches('\n').count() + 1;
            let col = text[..l.lo]
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .chars()
                .count()
                + 1;
            println!("{shown}:{line}:{col}: warning[{}]: {}", l.code, l.msg);
            problems += 1;
        }
    }
    if problems > 0 {
        return Err(format!(
            "{problems} warning{}",
            if problems == 1 { "" } else { "s" }
        ));
    }
    println!(
        "no warnings in {} file{}",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_unused_names_and_placeholders() {
        let src = "fn helper() -> Int {\n    1\n}\n\nfn used() -> Int {\n    2\n}\n\nexport component Card {\n    state open: Bool = false\n    let n = used()\n\n    text(\"Lorem ipsum {n}\")\n}\n";
        let l: Vec<String> = lint("src/a.vcwl", src)
            .iter()
            .map(|l| format!("{}: {}", l.code, l.msg))
            .collect();
        assert_eq!(
            l,
            [
                "unused-fn: function `helper` is never used",
                "unused-state: state `open` is never used",
                "placeholder: placeholder text (`lorem ipsum`) is about to ship to production",
            ]
        );
    }
}
