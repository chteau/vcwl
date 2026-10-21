//! `vcwl fmt [--check] [paths...]`: normalizes indentation (4 spaces per nesting level, one
//! extra level for continuation lines) and whitespace. Tokens are never changed: the output is
//! re-lexed and must produce the same token stream, including newline positions.
use crate::diag::SourceMap;
use crate::lexer::{lex, StrPiece, Tok, Token};
use crate::project::{vcwl_files, Project};
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|a| a == "--check");
    let paths: Vec<PathBuf> = args.iter().filter(|a| *a != "--check").map(PathBuf::from).collect();
    let roots = if paths.is_empty() { vec![Project::find(Path::new("."))?.root] } else { paths };
    let mut files = Vec::new();
    for r in &roots {
        vcwl_files(r, &mut files)?;
    }
    let (mut changed, mut errors) = (Vec::new(), 0);
    for f in &files {
        let src = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        match format(&f.display().to_string(), &src) {
            Ok(out) if out != src => {
                if !check {
                    std::fs::write(f, out).map_err(|e| format!("{}: {e}", f.display()))?;
                }
                changed.push(f);
            }
            Ok(_) => {}
            Err(e) => {
                eprint!("{e}");
                errors += 1;
            }
        }
    }
    for f in &changed {
        println!("{}{}", if check { "would reformat " } else { "formatted " }, f.display());
    }
    println!("{} file(s) checked, {} {}", files.len(), changed.len(), if check { "need formatting" } else { "reformatted" });
    if errors > 0 || (check && !changed.is_empty()) { Err(String::new()) } else { Ok(()) }
}

/// Operators after which an expression continues on the next line.
const CONTINUES: &[&str] = &[
    "=", "+=", "-=", "*=", "/=", "+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">=", "&&", "||", "=>", "->",
];

/// Formats `src`; `path` is only used in diagnostics.
pub fn format(path: &str, src: &str) -> Result<String, String> {
    let toks = lex(src, 0).map_err(|d| {
        let mut sm = SourceMap::default();
        sm.add(path, src.to_string());
        d.render(&sm)
    })?;
    let line_starts: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |pos: usize| line_starts.partition_point(|&s| s <= pos) - 1;
    let n_lines = line_starts.len();

    // Per line: nesting depth before its first token (or before the next token, for lines
    // without one), whether that token closes a group, and the punctuation ending the line.
    let mut depth_at = vec![None::<(usize, bool, bool)>; n_lines];
    let mut last_punct: Vec<Option<&str>> = vec![None; n_lines];
    let mut depth = 0usize;
    let mut prev_line = None;
    for t in &toks {
        let line = line_of(t.span.lo as usize);
        let closes = matches!(t.tok, Tok::Punct(")" | "]" | "}"));
        if depth_at[line].is_none() && !matches!(t.tok, Tok::Eof) {
            let leading_dot = matches!(t.tok, Tok::Punct("."));
            depth_at[line] = Some((depth, closes, leading_dot || continues(&last_punct, prev_line)));
        }
        match t.tok {
            Tok::Punct("(" | "[" | "{") => depth += 1,
            Tok::Punct(")" | "]" | "}") => depth = depth.saturating_sub(1),
            _ => {}
        }
        last_punct[line] = match t.tok {
            Tok::Punct(p) => Some(p),
            _ => None,
        };
        prev_line = Some(line);
    }

    // Lines starting inside a block comment or a multi-line string are kept verbatim.
    let mut verbatim = block_comments(src, &toks);
    verbatim.extend(toks.iter().filter(|t| matches!(t.tok, Tok::Str(_))).map(|t| (t.span.lo as usize, t.span.hi as usize)));
    let mut out = String::with_capacity(src.len());
    let mut blank = false;
    // Comment-only lines take the depth before the next token, even when it closes a group.
    let mut next_depth = 0;
    for slot in depth_at.iter_mut().rev() {
        match *slot {
            Some((d, ..)) => next_depth = d,
            None => *slot = Some((next_depth, false, false)),
        }
    }
    for (l, &start) in line_starts.iter().enumerate() {
        let end = line_starts.get(l + 1).map_or(src.len(), |e| e - 1);
        let raw = src[start..end].trim_end();
        if verbatim.iter().any(|&(lo, hi)| lo < start && start < hi) {
            out.push_str(raw);
            out.push('\n');
            blank = false;
            continue;
        }
        let text = raw.trim_start();
        if text.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if blank {
            out.push('\n');
            blank = false;
        }
        let (d, closes, cont) = depth_at[l].unwrap();
        let level = d - usize::from(closes) + usize::from(cont && !closes);
        out.push_str(&"    ".repeat(level));
        out.push_str(text);
        out.push('\n');
    }

    let again = lex(&out, 0).map_err(|d| d.msg)?;
    if let Some(i) = (0..toks.len().max(again.len())).find(|&i| !matches!((toks.get(i), again.get(i)), (Some(a), Some(b)) if same(a, b))) {
        let line = toks.get(i).map_or(n_lines, |t| line_of(t.span.lo as usize) + 1);
        return Err(format!("{path}:{line}: internal formatter error: output would change the program; file left unchanged\n"));
    }
    Ok(out)
}

/// Token equality ignoring source positions.
fn same(a: &Token, b: &Token) -> bool {
    a.nl == b.nl
        && match (&a.tok, &b.tok) {
            (Tok::Str(x), Tok::Str(y)) => {
                x.len() == y.len()
                    && x.iter().zip(y).all(|p| match p {
                        (StrPiece::Lit(l), StrPiece::Lit(r)) => l == r,
                        (StrPiece::Expr(l, _), StrPiece::Expr(r, _)) => l.len() == r.len() && l.iter().zip(r).all(|(l, r)| same(l, r)),
                        _ => false,
                    })
            }
            (x, y) => x == y,
        }
}

fn continues(last_punct: &[Option<&str>], prev_line: Option<usize>) -> bool {
    prev_line.and_then(|l| last_punct[l]).is_some_and(|p| CONTINUES.contains(&p))
}

/// Byte ranges of `/* */` comments. Only the gaps between tokens can hold comments.
fn block_comments(src: &str, toks: &[Token]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut gap_start = 0;
    for t in toks {
        let gap = &src[gap_start..t.span.lo as usize];
        let mut i = 0;
        while i < gap.len() {
            if gap[i..].starts_with("//") {
                i += gap[i..].find('\n').unwrap_or(gap.len() - i);
            } else if gap[i..].starts_with("/*") {
                let len = gap[i..].find("*/").map_or(gap.len() - i, |e| e + 2);
                out.push((gap_start + i, gap_start + i + len));
                i += len;
            } else {
                i += gap[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
        gap_start = t.span.hi as usize;
    }
    out
}
