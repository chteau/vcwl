//! Project layout, artifact generation and the `new`/`build`/`clean` commands.

use std::path::{Path, PathBuf};

use crate::compile::{compile, Options, Output};
use crate::diag::SourceMap;

pub const VCWL_JS: &str = include_str!("../runtime/vcwl.js");
pub const BOOT_JS: &str = include_str!("../runtime/boot.js");
pub const DEV_JS: &str = include_str!("../runtime/dev.js");

pub struct Project {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub out: PathBuf,
}

impl Project {
    /// Finds `vcwl.toml` in `start` or its ancestors.
    pub fn find(start: &Path) -> Result<Project, String> {
        let start = start.canonicalize().map_err(|e| format!("{}: {e}", start.display()))?;
        let mut dir = Some(start.as_path());
        while let Some(d) = dir {
            let manifest = d.join("vcwl.toml");
            if manifest.is_file() {
                let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
                let entry = manifest_value(&text, "entry").unwrap_or_else(|| "src/main.vcwl".into());
                let out = manifest_value(&text, "out").unwrap_or_else(|| "dist".into());
                return Ok(Project { root: d.to_path_buf(), entry: d.join(entry), out: d.join(out) });
            }
            dir = d.parent();
        }
        Err(format!("no vcwl.toml found in {} or its parents (create a project with `vcwl new <name>`)", start.display()))
    }
}

/// Reads `key = "value"` from the manifest (the only form the manifest uses).
fn manifest_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

pub fn compile_entry(entry: &Path, dev: bool) -> Result<(Output, SourceMap), String> {
    let mut sm = SourceMap::default();
    match compile(entry, &mut sm, &Options { dev }) {
        Ok(o) => Ok((o, sm)),
        Err(f) => Err(f.diags.iter().map(|d| d.render(&sm)).collect()),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Content-Security-Policy derived from the app's capabilities.
pub fn csp(caps: &crate::app::Capabilities, dev: bool) -> String {
    // The app's own files (app.wasm) are always fetched from 'self'.
    let mut connect: Vec<String> = vec!["'self'".into()];
    for h in &caps.network {
        if h != "self" {
            connect.push(format!("https://{h}"));
        }
    }
    let _ = dev;
    let connect = connect.join(" ");
    // 'self' is always allowed for scripts and the module fetch of app.wasm.
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' https: data:; \
         connect-src {connect}; base-uri 'none'; form-action 'none'"
    )
}

pub fn index_html(title: &str, caps: &crate::app::Capabilities, dev: bool) -> String {
    let config = format!("{{\"caps\":{},\"dev\":{dev}}}", caps.to_json());
    let dev_script = if dev { "\n<script type=\"module\" src=\"dev.js\"></script>" } else { "" };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
<title>{title}</title>\n<link rel=\"icon\" href=\"data:,\">\n<link rel=\"stylesheet\" href=\"app.css\">\n\
<script type=\"application/json\" id=\"vcwl-config\">{config}</script>\n\
<script type=\"module\" src=\"boot.js\"></script>{dev_script}\n</head>\n\
<body>\n<noscript>This application needs JavaScript and WebAssembly enabled.</noscript>\n</body>\n</html>\n",
        csp = html_escape(&csp(caps, dev)),
        title = html_escape(title),
    )
}

/// Compiles the project and writes static artifacts to `out`. Returns the artifact sizes.
pub fn build(entry: &Path, out: &Path, dev: bool) -> Result<Vec<(String, usize)>, String> {
    let (o, _) = compile_entry(entry, dev)?;
    let Some(app) = &o.app else {
        return Err(format!("{}: no `app` declaration; nothing to build (use `vcwl run` for programs with `main`)", entry.display()));
    };
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut files: Vec<(&str, Vec<u8>)> = vec![
        ("index.html", index_html(&app.title, &app.caps, dev).into_bytes()),
        ("app.wasm", o.wasm.clone()),
        ("app.css", app.css.clone().into_bytes()),
        ("vcwl.js", VCWL_JS.as_bytes().to_vec()),
        ("boot.js", BOOT_JS.as_bytes().to_vec()),
        // GitHub Pages: serve files as-is (no Jekyll processing).
        (".nojekyll", Vec::new()),
    ];
    if dev {
        files.push(("dev.js", DEV_JS.as_bytes().to_vec()));
    } else {
        let _ = std::fs::remove_file(out.join("dev.js"));
    }
    let mut sizes = Vec::new();
    for (name, data) in files {
        std::fs::write(out.join(name), &data).map_err(|e| format!("{}: {e}", out.join(name).display()))?;
        if !name.starts_with('.') {
            sizes.push((name.to_string(), data.len()));
        }
    }
    Ok(sizes)
}

pub fn clean(p: &Project) -> Result<bool, String> {
    if !p.out.exists() {
        return Ok(false);
    }
    // Only remove the configured output directory, and only if it is inside the project.
    let out = p.out.canonicalize().map_err(|e| e.to_string())?;
    if !out.starts_with(&p.root) || out == p.root || out.join("vcwl.toml").exists() {
        return Err(format!("refusing to delete {}: not a build directory inside the project", out.display()));
    }
    std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(true)
}

pub fn new_project(dir: &Path) -> Result<(), String> {
    if dir.exists() && dir.read_dir().map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("app").to_string();
    let files = [
        ("vcwl.toml", format!("# VCWL project\nname = \"{name}\"\nentry = \"src/main.vcwl\"\nout = \"dist\"\n")),
        ("src/main.vcwl", STARTER.replace("{{name}}", &name)),
        ("src/components/counter.vcwl", STARTER_COUNTER.to_string()),
        ("tests/main_test.vcwl", STARTER_TEST.to_string()),
        (".gitignore", "dist/\n".to_string()),
    ];
    for (path, content) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, content).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}

const STARTER: &str = r#"app "{{name}}"

import "./components/counter.vcwl"

component App {
    style {
        background: #0b0d10
        color: #f5f5f5
        min-height: 100vh
        padding: 2rem
    }

    column(gap: 20px, align: center) {
        heading("Hello, {{name}}!", level: 1)
        paragraph("Edit src/main.vcwl and run `vcwl dev`.")
        Counter(label: "Clicks")
    }
}
"#;

const STARTER_COUNTER: &str = r#"export component Counter(label: String) {
    state count: Int = 0

    row(gap: 12px, align: center) {
        text("{label}: {count}")
        button("+1") {
            on click {
                count += 1
            }
            style {
                background: #b6ff6c
                color: #111111
                radius: 8px
                padding: 8px 16px
                border: none
            }
        }
    }
}
"#;

const STARTER_TEST: &str = r#"// Functions named `test_*` run with `vcwl test`; they pass unless they panic.

fn double(n: Int) -> Int {
    n * 2
}

fn test_double() {
    assert(double(21) == 42, "double(21) should be 42")
}
"#;
