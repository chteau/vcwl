//! Security review of registry packages before they are installed (`vcwl install`, `vcwl add`)
//! and on demand (`vcwl audit`). It reads a package's files and lists what the package could do
//! to the app that installs it. Findings are notices (shown) or dangers (installation stops until
//! the app trusts that exact version in its `[trust]` table).
//!
//! What a package can reach, and so what is looked for: VCWL runs no package code when
//! installing (there are no install scripts). Its code runs where the app's does: in the
//! browser, under the app's capabilities (a package cannot widen the network allow list or the
//! CSP); during prerendering, compiled into the app's WebAssembly and run by Node.js with the
//! host functions VCWL gives it; and on the server, inside server functions, which can read
//! environment variables and the app's whole persistent store but have no file system or
//! outgoing network access. Its `public/` files are copied into the app's site. This review is a
//! static reading of the source for those powers, not a proof that a package is harmless.

use std::path::Path;

use crate::lexer::{StrPiece, Tok, Token};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Notice,
    Danger,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub what: String,
}

/// Extensions of `public/` files that run code or change how the site is served when they sit
/// on the app's origin.
const ACTIVE: &[&str] = &[
    "html", "htm", "xhtml", "js", "mjs", "cjs", "svg", "xml", "wasm", "php", "shtml",
];
/// Files hosts treat as configuration.
const HOST_FILES: &[&str] = &[
    "_headers",
    "_redirects",
    "CNAME",
    "netlify.toml",
    "vercel.json",
    "sw.js",
    "service-worker.js",
    "manifest.json",
    "manifest.webmanifest",
    "robots.txt",
];
const BIG: u64 = 5 << 20;

/// Every token of a file, with those inside string interpolations.
fn flatten(toks: &[Token], out: &mut Vec<Token>) {
    for t in toks {
        if let Tok::Str(pieces) = &t.tok {
            for p in pieces {
                if let StrPiece::Expr(inner, _) = p {
                    flatten(inner, out);
                }
            }
        }
        out.push(t.clone());
    }
}

fn literal(t: Option<&Token>) -> Option<String> {
    match &t?.tok {
        Tok::Str(p) => Some(
            p.iter()
                .map(|x| {
                    if let StrPiece::Lit(s) = x {
                        s.as_str()
                    } else {
                        "{…}"
                    }
                })
                .collect(),
        ),
        _ => None,
    }
}

/// Reviews the package named `name` whose files are in `dir`.
pub fn review(name: &str, dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |level, what: String| out.push(Finding { level, what });
    let (mut server, mut env, mut store, mut fetches, mut clipboard, mut navigates) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
        Vec::new(),
    );
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let rel = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                add(Level::Danger, format!("{rel} is a symbolic link"));
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if rel == ".vcwl-version" {
                continue;
            }
            if meta.len() > BIG {
                add(
                    Level::Notice,
                    format!("{rel} is large ({} MB)", meta.len() >> 20),
                );
            }
            if let Some(public) = rel.strip_prefix("public/") {
                let file = public.rsplit('/').next().unwrap_or(public);
                let ext = file
                    .rsplit_once('.')
                    .map(|(_, x)| x.to_ascii_lowercase())
                    .unwrap_or_default();
                if ACTIVE.contains(&ext.as_str()) {
                    add(
                        Level::Danger,
                        format!(
                            "{rel} would be served from your site's origin and can run code there (`.{ext}`)"
                        ),
                    );
                } else if HOST_FILES.contains(&file)
                    || public.starts_with(".well-known/")
                    || file.starts_with('.')
                {
                    add(
                        Level::Danger,
                        format!("{rel} would change how your site is served"),
                    );
                }
            } else if rel.split('/').any(|p| p.starts_with('.')) {
                add(Level::Notice, format!("{rel} is a hidden file"));
            }
            if !rel.ends_with(".vcwl") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(raw) = crate::lexer::lex(&text, 0) else {
                add(Level::Notice, format!("{rel} does not parse"));
                continue;
            };
            let mut toks = Vec::new();
            flatten(&raw, &mut toks);
            let ident = |i: usize| match toks.get(i).map(|t| &t.tok) {
                Some(Tok::Ident(s)) => Some(s.as_str()),
                _ => None,
            };
            let punct = |i: usize, p: &str| matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Punct(q)) if *q == p);
            for i in 0..toks.len() {
                match ident(i) {
                    Some("server") if ident(i + 1) == Some("fn") => {
                        server.push(ident(i + 2).unwrap_or("?").to_string());
                    }
                    Some("env") if punct(i + 1, "(") => env.push(
                        literal(toks.get(i + 2))
                            .map_or_else(|| "a computed name".into(), |k| format!("\"{k}\"")),
                    ),
                    Some("save" | "load" | "remove") if punct(i + 1, "(") || punct(i + 1, "<") => {
                        let at = if punct(i + 1, "<") {
                            (i..toks.len()).find(|&j| punct(j, "(")).map(|j| j + 1)
                        } else {
                            Some(i + 2)
                        };
                        match at.and_then(|j| literal(toks.get(j))) {
                            Some(key) if key.starts_with(&format!("{name}.")) => {}
                            Some(key) => store.push(format!("\"{key}\"")),
                            None => store.push("a computed key".into()),
                        }
                    }
                    Some("fetch") => {
                        if let Some(url) = literal(toks.get(i + 1)) {
                            fetches.push(url);
                        } else if let Some(f) = ident(i + 1) {
                            fetches.push(format!("server function `{f}`"));
                        }
                    }
                    Some("copy") if punct(i + 1, "(") => clipboard = true,
                    Some("navigate") if punct(i + 1, "(") => {
                        if let Some(u) = literal(toks.get(i + 2)).filter(|u| u.contains("://")) {
                            navigates.push(u);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let list = |v: &mut Vec<String>| {
        v.sort();
        v.dedup();
        v.join(", ")
    };
    if !server.is_empty() {
        add(
            Level::Notice,
            format!(
                "runs code on your server: server functions {}",
                list(&mut server)
            ),
        );
    }
    if !env.is_empty() {
        add(
            Level::Danger,
            format!(
                "reads your server's environment variables (where secrets live): {}",
                list(&mut env)
            ),
        );
    }
    if !store.is_empty() {
        add(
            Level::Danger,
            format!(
                "uses persistent store keys outside its own `{name}.` prefix, so it can read or overwrite your app's data: {}",
                list(&mut store)
            ),
        );
    }
    if !fetches.is_empty() {
        add(
            Level::Notice,
            format!(
                "makes requests (allowed only by your app's `network` capability): {}",
                list(&mut fetches)
            ),
        );
    }
    if clipboard {
        add(Level::Notice, "writes to the clipboard".into());
    }
    if !navigates.is_empty() {
        add(
            Level::Notice,
            format!("navigates to other sites: {}", list(&mut navigates)),
        );
    }
    out.sort_by(|a, b| b.level.cmp(&a.level));
    out
}

/// One package's findings as text, dangers first.
pub fn report(name: &str, version: &str, findings: &[Finding]) -> String {
    if findings.is_empty() {
        return format!("  {name} {version}: nothing to report\n");
    }
    let mut s = format!("  {name} {version}:\n");
    for f in findings {
        let tag = if f.level == Level::Danger {
            "DANGER"
        } else {
            "notice"
        };
        s.push_str(&format!("    {tag}  {}\n", f.what));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_powers_a_package_uses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tmp/review-unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("public")).unwrap();
        std::fs::write(
            dir.join("src/lib.vcwl"),
            "export server fn steal() -> String {\n    save(\"users\", 1)\n    save(\"spy.count\", 1)\n    env(\"API_KEY\").unwrap_or(\"\")\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("public/evil.html"), "<script></script>").unwrap();
        std::fs::write(dir.join("public/logo.png"), "png").unwrap();
        let f = review("spy", &dir);
        let text = report("spy", "1.0.0", &f);
        assert!(text.contains("DANGER  reads your server's environment variables (where secrets live): \"API_KEY\""), "{text}");
        assert!(
            text.contains("outside its own `spy.` prefix")
                && text.contains("\"users\"")
                && !text.contains("spy.count"),
            "{text}"
        );
        assert!(
            text.contains("public/evil.html would be served") && !text.contains("logo.png"),
            "{text}"
        );
        assert!(
            text.contains("notice  runs code on your server: server functions steal"),
            "{text}"
        );
    }
}
