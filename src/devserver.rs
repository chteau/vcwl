//! `vcwl dev [--port N]`: builds in development mode, serves the output directory on
//! 127.0.0.1 and rebuilds when a `.vcwl` file or `vcwl.toml` changes.
//!
//! `GET /__vcwl/status` returns the build version on the first line, followed by the
//! diagnostics of the last failed build (if any). `dev.js` polls it to reload the page
//! or show the errors.
use crate::project::{self, DEV_JS, Project, VCWL_JS};
use crate::server::{self, Response, function_call, read_request};
use std::io::Write;
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
/// The server functions of the last successful build.
type Functions = Arc<Mutex<Option<Arc<server::Functions>>>>;

pub fn run(args: &[String]) -> Result<(), String> {
    let port = match args.iter().position(|a| a == "--port") {
        Some(i) => args
            .get(i + 1)
            .and_then(|p| p.parse::<u16>().ok())
            .ok_or("usage: vcwl dev [--port N]")?,
        None => 4400,
    };
    let p = Project::find(Path::new("."))?;
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let status: Shared = Arc::default();
    let fns: Functions = Arc::default();
    rebuild(&p, &status, &fns);
    println!(
        "serving {} at http://{addr}/ (Ctrl-C to stop)",
        p.out.display()
    );
    let _ = std::io::stdout().flush();

    let (root, out, watched, reloaded) =
        (p.root.clone(), p.out.clone(), status.clone(), fns.clone());
    std::thread::spawn(move || {
        let mut last = fingerprint(&root, &out);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = fingerprint(&root, &out);
            if now != last {
                last = now;
                // Re-read the manifest: `entry` or `out` may have changed.
                match Project::find(&root) {
                    Ok(p) => rebuild(&p, &watched, &reloaded),
                    Err(e) => watched.lock().unwrap().error = e,
                }
            }
        }
    });

    for conn in listener.incoming().flatten() {
        let (out, status, fns) = (p.out.clone(), status.clone(), fns.clone());
        std::thread::spawn(move || {
            let _ = serve(conn, &out, &status, &fns);
        });
    }
    Ok(())
}

fn rebuild(p: &Project, status: &Shared, fns: &Functions) {
    let start = std::time::Instant::now();
    let result = project::build(&p.entry, &p.out, true)
        .and_then(|_| server::load_functions(p))
        .map(|f| *fns.lock().unwrap() = f.map(Arc::new));
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
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.')
                    && name != "node_modules"
                    && name != "target"
                    && path != skip
                {
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
    for pkg in project::packages(root) {
        if !pkg.starts_with(root) {
            walk(&pkg, skip, &mut v);
        }
    }
    v.sort();
    v
}

fn serve(conn: TcpStream, out: &Path, status: &Shared, fns: &Functions) -> std::io::Result<()> {
    let req = read_request(&conn)?;
    let res = if req.path == "/__vcwl/status" {
        let s = status.lock().unwrap();
        Response::new(
            "200 OK",
            "text/plain; charset=utf-8",
            format!("{}\n{}", s.version, s.error),
        )
    } else if let Some(res) = function_call(&req, fns.lock().unwrap().as_deref()) {
        res
    } else if req.method != "GET" && req.method != "HEAD" {
        Response::new("405 Method Not Allowed", "text/plain", "method not allowed")
    } else {
        static_file(out, &req.path)
    };
    res.write(&conn, req.method == "HEAD")
}

/// A file of the output directory, as a static host serves it: `/about` redirects to
/// `/about/`, which serves `about/index.html`, and unknown paths get `404.html` if it exists.
pub fn static_file(out: &Path, path: &str) -> Response {
    match resolve(out, path) {
        None => Response::new("400 Bad Request", "text/plain", "bad path"),
        Some(file) if file.is_dir() => {
            let mut r = Response::new("301 Moved Permanently", "text/plain", "");
            r.headers.push(format!("location: {path}/"));
            r
        }
        Some(file) => match std::fs::read(&file) {
            Ok(data) => Response::new("200 OK", content_type(&file), data),
            Err(_) => match (path, std::fs::read(out.join("404.html"))) {
                (p, Ok(page)) if p != "/" => {
                    Response::new("404 Not Found", "text/html; charset=utf-8", page)
                }
                // Before the first successful build, still serve a page that reports the errors.
                ("/" | "/index.html", _) => {
                    Response::new("200 OK", "text/html; charset=utf-8", FALLBACK_HTML)
                }
                ("/dev.js", _) => Response::new("200 OK", "text/javascript", DEV_JS),
                ("/vcwl.js", _) => Response::new("200 OK", "text/javascript", VCWL_JS),
                _ => Response::new("404 Not Found", "text/plain", "not found"),
            },
        },
    }
}

const FALLBACK_HTML: &str = "<!doctype html>\n<html lang=\"en\"><meta charset=\"utf-8\"><title>VCWL dev</title>\n\
<script type=\"module\" src=\"dev.js\"></script><body></body></html>\n";

/// Maps a URL path to a file under `out`, rejecting anything that could escape it.
fn resolve(out: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let rel = if rel.is_empty() || rel.ends_with('/') {
        format!("{rel}index.html")
    } else {
        rel.to_string()
    };
    let rel = Path::new(&rel);
    rel.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| out.join(rel))
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
