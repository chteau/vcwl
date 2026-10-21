//! `vcwl dev [--port N]`: builds in development mode, serves the output directory on
//! 127.0.0.1 and rebuilds when a `.vcwl` file or `vcwl.toml` changes.
//!
//! `GET /__vcwl/status` returns the build version on the first line, followed by the
//! diagnostics of the last failed build (if any). `dev.js` polls it to reload the page
//! or show the errors.
use crate::project::{self, Project, DEV_JS, VCWL_JS};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

#[derive(Default)]
struct Status {
    version: u64,
    error: String,
}

type Shared = Arc<Mutex<Status>>;

pub fn run(args: &[String]) -> Result<(), String> {
    let port = match args.iter().position(|a| a == "--port") {
        Some(i) => args.get(i + 1).and_then(|p| p.parse::<u16>().ok()).ok_or("usage: vcwl dev [--port N]")?,
        None => 4400,
    };
    let p = Project::find(Path::new("."))?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let status: Shared = Arc::default();
    rebuild(&p, &status);
    println!("serving {} at http://{addr}/ (Ctrl-C to stop)", p.out.display());
    let _ = std::io::stdout().flush();

    let (root, out, watched) = (p.root.clone(), p.out.clone(), status.clone());
    std::thread::spawn(move || {
        let mut last = fingerprint(&root, &out);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = fingerprint(&root, &out);
            if now != last {
                last = now;
                // Re-read the manifest: `entry` or `out` may have changed.
                match Project::find(&root) {
                    Ok(p) => rebuild(&p, &watched),
                    Err(e) => watched.lock().unwrap().error = e,
                }
            }
        }
    });

    for conn in listener.incoming().flatten() {
        let (out, status) = (p.out.clone(), status.clone());
        std::thread::spawn(move || {
            let _ = serve(conn, &out, &status);
        });
    }
    Ok(())
}

fn rebuild(p: &Project, status: &Shared) {
    let start = std::time::Instant::now();
    let result = project::build(&p.entry, &p.out, true);
    let mut s = status.lock().unwrap();
    match result {
        Ok(_) => {
            s.version += 1;
            s.error.clear();
            println!("built in {:.0?}", start.elapsed());
        }
        Err(e) => {
            eprint!("{e}");
            s.error = e;
        }
    }
}

/// Paths and modification times of every source file in the project.
fn fingerprint(root: &Path, skip: &Path) -> Vec<(PathBuf, SystemTime)> {
    fn walk(dir: &Path, skip: &Path, out: &mut Vec<(PathBuf, SystemTime)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && name != "target" && path != skip {
                    walk(&path, skip, out);
                }
            } else if (name.ends_with(".vcwl") || name == "vcwl.toml")
                && let Ok(m) = e.metadata().and_then(|m| m.modified())
            {
                out.push((path, m));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, skip, &mut v);
    v.sort();
    v
}

fn serve(conn: TcpStream, out: &Path, status: &Shared) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(&conn);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // Drain the headers; requests have no body we care about.
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        line.clear();
    }
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let (code, ctype, body): (&str, &str, Vec<u8>) = if method != "GET" && method != "HEAD" {
        ("405 Method Not Allowed", "text/plain", b"method not allowed".to_vec())
    } else if path == "/__vcwl/status" {
        let s = status.lock().unwrap();
        ("200 OK", "text/plain; charset=utf-8", format!("{}\n{}", s.version, s.error).into_bytes())
    } else {
        match resolve(out, path) {
            None => ("400 Bad Request", "text/plain", b"bad path".to_vec()),
            Some(file) => match std::fs::read(&file) {
                Ok(data) => ("200 OK", content_type(&file), data),
                // Before the first successful build, still serve a page that reports the errors.
                Err(_) => match path {
                    "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", FALLBACK_HTML.as_bytes().to_vec()),
                    "/dev.js" => ("200 OK", "text/javascript", DEV_JS.as_bytes().to_vec()),
                    "/vcwl.js" => ("200 OK", "text/javascript", VCWL_JS.as_bytes().to_vec()),
                    _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
                },
            },
        }
    };
    let mut w = &conn;
    write!(w, "HTTP/1.1 {code}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n", body.len())?;
    if method != "HEAD" {
        w.write_all(&body)?;
    }
    w.flush()
}

const FALLBACK_HTML: &str = "<!doctype html>\n<html lang=\"en\"><meta charset=\"utf-8\"><title>VCWL dev</title>\n\
<script type=\"module\" src=\"dev.js\"></script><body></body></html>\n";

/// Maps a URL path to a file under `out`, rejecting anything that could escape it.
fn resolve(out: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let rel = if rel.is_empty() || rel.ends_with('/') { format!("{rel}index.html") } else { rel.to_string() };
    let rel = Path::new(&rel);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| out.join(rel))
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}
