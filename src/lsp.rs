//! `vcwl lsp`: a language server over stdin/stdout (JSON-RPC with `Content-Length` framing).
//! It publishes compile errors and lints as diagnostics when a document opens or changes,
//! formats documents with `vcwl fmt`, and completes keywords, elements and style properties.
//! Open documents are compiled from their unsaved text; their project's entry is compiled when
//! it imports them (so uses of their components are checked too), otherwise the file alone.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use crate::compile::OVERLAY;
use crate::server::{Json, quote};

const KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "var",
    "if",
    "else",
    "match",
    "for",
    "in",
    "while",
    "return",
    "break",
    "continue",
    "struct",
    "enum",
    "import",
    "export",
    "component",
    "state",
    "event",
    "emit",
    "on",
    "style",
    "when",
    "app",
    "slot",
    "server",
    "fetch",
    "true",
    "false",
    "routes",
    "font",
    "key",
];
const ELEMENTS: &[&str] = &[
    "text",
    "column",
    "row",
    "stack",
    "grid",
    "box",
    "scroll",
    "section",
    "header",
    "footer",
    "nav",
    "main",
    "article",
    "aside",
    "list",
    "ordered_list",
    "item",
    "paragraph",
    "span",
    "strong",
    "code",
    "heading",
    "divider",
    "button",
    "link",
    "image",
    "input",
    "textarea",
    "checkbox",
    "select",
    "form",
    "scene",
    "mesh",
    "light",
    "group",
];
const PROPERTIES: &[&str] = &[
    "color",
    "background",
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "padding",
    "padding-x",
    "padding-y",
    "margin",
    "margin-x",
    "margin-y",
    "gap",
    "row-gap",
    "column-gap",
    "border",
    "radius",
    "font",
    "size",
    "weight",
    "line-height",
    "letter-spacing",
    "italic",
    "text-align",
    "decoration",
    "case",
    "white-space",
    "direction",
    "align",
    "align-self",
    "justify",
    "wrap",
    "grow",
    "shrink",
    "basis",
    "display",
    "columns",
    "rows",
    "span",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "inset",
    "z",
    "overflow",
    "overflow-x",
    "overflow-y",
    "opacity",
    "transform",
    "transition",
    "cursor",
    "shadow",
    "fit",
    "container",
    "user-select",
    "list-style",
];

fn get<'a>(j: &'a Json, path: &[&str]) -> Option<&'a Json> {
    path.iter().try_fold(j, |j, k| match j {
        Json::Obj(fields) => fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_ref()),
        _ => None,
    })
}

fn str_at<'a>(j: &'a Json, path: &[&str]) -> Option<&'a str> {
    match get(j, path)? {
        Json::Str(s) => Some(s),
        _ => None,
    }
}

fn raw(j: &Json) -> String {
    let mut s = String::new();
    j.write(&mut s);
    s
}

fn q(s: &str) -> String {
    let mut out = String::new();
    quote(s, &mut out);
    out
}

fn path_of(uri: &str) -> PathBuf {
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    // Percent-decoding (editors escape spaces and non-ASCII characters).
    let b = p.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    // `file:///C:/dir/file.vcwl` on Windows.
    match path.strip_prefix('/') {
        Some(rest) if cfg!(windows) && rest.get(1..2) == Some(":") => PathBuf::from(rest),
        _ => PathBuf::from(path),
    }
}

fn uri_of(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut s = String::from(if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for c in path.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' => s.push(c),
            _ => {
                let mut buf = [0; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    s.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    s
}

/// LSP position (line, UTF-16 column) of byte offset `off` in `text`.
fn position(text: &str, off: usize) -> String {
    let off = off.min(text.len());
    let before = &text[..text.floor_char_boundary(off)];
    let line = before.matches('\n').count();
    let col: usize = before
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .map(char::len_utf16)
        .sum();
    format!("{{\"line\":{line},\"character\":{col}}}")
}

fn range(text: &str, lo: usize, hi: usize) -> String {
    format!(
        "{{\"start\":{},\"end\":{}}}",
        position(text, lo),
        position(text, hi.max(lo + 1).min(text.len()))
    )
}

struct Server {
    out: std::io::Stdout,
    docs: HashMap<PathBuf, String>,
    /// Files that got diagnostics, so they can be cleared.
    flagged: Vec<PathBuf>,
}

impl Server {
    fn send(&mut self, body: &str) {
        let mut o = self.out.lock();
        let _ = write!(o, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = o.flush();
    }

    fn reply(&mut self, id: &Json, result: &str) {
        self.send(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{result}}}",
            raw(id)
        ));
    }

    /// Compiles what `path` belongs to and publishes diagnostics for every file involved.
    fn diagnose(&mut self, path: &Path) {
        let mut found: HashMap<PathBuf, Vec<String>> = HashMap::new();
        let canon = crate::project::canonical(path).unwrap_or(path.to_path_buf());
        let entry = crate::project::manifest_for(path.parent().unwrap_or(Path::new(".")))
            .map(|(root, m)| {
                root.join(
                    crate::project::manifest_value(&m, "entry")
                        .unwrap_or_else(|| "src/main.vcwl".into()),
                )
            })
            .filter(|e| e.is_file());
        let compile = |entry: &Path| {
            let mut sm = crate::diag::SourceMap::default();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::compile::compile(entry, &mut sm, &crate::compile::Options { dev: true })
            }));
            (r, sm)
        };
        let mut result = entry.as_deref().map(compile);
        let reaches = |sm: &crate::diag::SourceMap| {
            sm.files
                .iter()
                .any(|f| crate::project::canonical(Path::new(&f.path)).is_ok_and(|p| p == canon))
        };
        if !result.as_ref().is_some_and(|(_, sm)| reaches(sm)) {
            result = Some(compile(path));
        }
        let (r, sm) = result.unwrap();
        match r {
            Ok(Ok(_)) => {}
            Ok(Err(fail)) => {
                for d in &fail.diags {
                    let f = sm.file_of(d.span.lo);
                    let lo = (d.span.lo - f.base) as usize;
                    let hi = (d.span.hi.max(d.span.lo) - f.base) as usize;
                    let mut msg = d.msg.clone();
                    if let Some(h) = &d.help {
                        msg.push_str(&format!("\nhelp: {h}"));
                    }
                    let p = crate::project::canonical(Path::new(&f.path)).unwrap_or(PathBuf::from(&f.path));
                    found.entry(p).or_default().push(format!(
                        "{{\"range\":{},\"severity\":1,\"source\":\"vcwl\",\"message\":{}}}",
                        range(&f.text, lo, hi),
                        q(&msg)
                    ));
                }
            }
            Err(_) => found.entry(canon.clone()).or_default().push(format!(
                "{{\"range\":{},\"severity\":1,\"source\":\"vcwl\",\"message\":\"the compiler crashed on this file (a VCWL bug)\"}}",
                range("", 0, 0)
            )),
        }
        if let Some(text) = self.docs.get(&canon).cloned() {
            for l in crate::lint::lint(&canon.display().to_string(), &text) {
                found.entry(canon.clone()).or_default().push(format!(
                    "{{\"range\":{},\"severity\":2,\"source\":\"vcwl lint\",\"code\":{},\"message\":{}}}",
                    range(&text, l.lo, l.hi),
                    q(l.code),
                    q(&l.msg)
                ));
            }
        }
        let mut files: Vec<PathBuf> = std::mem::take(&mut self.flagged);
        files.push(canon);
        files.extend(found.keys().cloned());
        files.sort();
        files.dedup();
        for f in files {
            let diags = found.get(&f).map(|v| v.join(",")).unwrap_or_default();
            if !diags.is_empty() {
                self.flagged.push(f.clone());
            }
            let body = format!(
                "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":{},\"diagnostics\":[{diags}]}}}}",
                q(&uri_of(&f))
            );
            self.send(&body);
        }
    }

    fn open(&mut self, uri: &str, text: String) {
        let path = path_of(uri);
        let canon = crate::project::canonical(&path).unwrap_or(path.clone());
        OVERLAY.with(|o| o.borrow_mut().insert(canon.clone(), text.clone()));
        self.docs.insert(canon, text);
        self.diagnose(&path);
    }

    /// Handles one message; returns false on `exit`.
    fn handle(&mut self, msg: &Json) -> bool {
        let method = str_at(msg, &["method"]).unwrap_or("");
        let id = get(msg, &["id"]);
        let uri = str_at(msg, &["params", "textDocument", "uri"])
            .unwrap_or("")
            .to_string();
        match method {
            "initialize" => {
                let caps = "{\"capabilities\":{\"textDocumentSync\":{\"openClose\":true,\"change\":1,\"save\":true},\"documentFormattingProvider\":true,\"completionProvider\":{\"triggerCharacters\":[]}},\"serverInfo\":{\"name\":\"vcwl\"}}";
                self.reply(id.unwrap_or(&Json::Null), caps);
            }
            "textDocument/didOpen" => {
                let text = str_at(msg, &["params", "textDocument", "text"])
                    .unwrap_or("")
                    .to_string();
                self.open(&uri, text);
            }
            "textDocument/didChange" => {
                // Full sync: the last change holds the whole text.
                if let Some(Json::Arr(changes)) = get(msg, &["params", "contentChanges"])
                    && let Some(Json::Str(text)) = changes.last().and_then(|c| get(c, &["text"]))
                {
                    self.open(&uri, text.clone());
                }
            }
            "textDocument/didSave" => self.diagnose(&path_of(&uri)),
            "textDocument/didClose" => {
                let p = path_of(&uri);
                let canon = crate::project::canonical(&p).unwrap_or(p);
                OVERLAY.with(|o| o.borrow_mut().remove(&canon));
                self.docs.remove(&canon);
            }
            "textDocument/formatting" => {
                let p = path_of(&uri);
                let canon = crate::project::canonical(&p).unwrap_or(p);
                let text = self
                    .docs
                    .get(&canon)
                    .cloned()
                    .or_else(|| std::fs::read_to_string(&canon).ok())
                    .unwrap_or_default();
                let result = match crate::fmt::format(&canon.display().to_string(), &text) {
                    Ok(out) if out != text => format!(
                        "[{{\"range\":{},\"newText\":{}}}]",
                        range(&text, 0, text.len()),
                        q(&out)
                    ),
                    _ => "[]".into(),
                };
                self.reply(id.unwrap_or(&Json::Null), &result);
            }
            "textDocument/completion" => {
                // Kinds: 14 keyword, 3 function (elements), 10 property.
                let items: Vec<String> = KEYWORDS
                    .iter()
                    .map(|k| (k, 14))
                    .chain(ELEMENTS.iter().map(|k| (k, 3)))
                    .chain(PROPERTIES.iter().map(|k| (k, 10)))
                    .map(|(k, kind)| format!("{{\"label\":\"{k}\",\"kind\":{kind}}}"))
                    .collect();
                self.reply(id.unwrap_or(&Json::Null), &format!("[{}]", items.join(",")));
            }
            "shutdown" => self.reply(id.unwrap_or(&Json::Null), "null"),
            "exit" => return false,
            _ => {
                // Requests this server does not know get an error; notifications are ignored.
                if let Some(id) = id {
                    let body = format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":-32601,\"message\":\"unsupported\"}}}}",
                        raw(id)
                    );
                    self.send(&body);
                }
            }
        }
        true
    }
}

pub fn run(_args: &[String]) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut server = Server {
        out: std::io::stdout(),
        docs: HashMap::new(),
        flagged: Vec::new(),
    };
    loop {
        let mut len = None;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse::<usize>().ok();
            }
        }
        let Some(len) = len else { continue };
        let mut body = vec![0; len];
        input.read_exact(&mut body).map_err(|e| e.to_string())?;
        let Some(msg) = Json::parse(&String::from_utf8_lossy(&body)) else {
            continue;
        };
        if !server.handle(&msg) {
            return Ok(());
        }
    }
}
