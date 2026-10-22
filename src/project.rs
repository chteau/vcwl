//! Project layout, artifact generation and the `new`/`build`/`clean` commands.

use std::path::{Path, PathBuf};

use crate::compile::{Options, Output, compile};
use crate::diag::SourceMap;

pub const VCWL_JS: &str = include_str!("../runtime/vcwl.js");
pub const BOOT_JS: &str = include_str!("../runtime/boot.js");
pub const DEV_JS: &str = include_str!("../runtime/dev.js");
const PRERENDER_JS: &str = include_str!("../runtime/prerender.mjs");
/// The renderer, built from `render/` by `build.rs`.
pub const RENDER_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/render.wasm"));

pub struct Project {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub out: PathBuf,
    /// Contents of `vcwl.toml`.
    pub manifest: String,
}

impl Project {
    /// Finds `vcwl.toml` in `start` or its ancestors.
    pub fn find(start: &Path) -> Result<Project, String> {
        let start = start
            .canonicalize()
            .map_err(|e| format!("{}: {e}", start.display()))?;
        let mut dir = Some(start.as_path());
        while let Some(d) = dir {
            let manifest = d.join("vcwl.toml");
            if manifest.is_file() {
                let text = std::fs::read_to_string(&manifest)
                    .map_err(|e| format!("{}: {e}", manifest.display()))?;
                let entry =
                    manifest_value(&text, "entry").unwrap_or_else(|| "src/main.vcwl".into());
                let out = manifest_value(&text, "out").unwrap_or_else(|| "dist".into());
                return Ok(Project {
                    root: d.to_path_buf(),
                    entry: d.join(entry),
                    out: d.join(out),
                    manifest: text,
                });
            }
            dir = d.parent();
        }
        Err(format!(
            "no vcwl.toml found in {} or its parents (create a project with `vcwl new <name>`)",
            start.display()
        ))
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
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Content-Security-Policy derived from the app's capabilities.
pub fn csp(app: &crate::app::AppOutput, pre: Option<&Prerendered>) -> String {
    let caps = &app.caps;
    // The app's own files (app.wasm) are always fetched from 'self'.
    let mut connect: Vec<String> = vec!["'self'".into()];
    for h in &caps.network {
        if h != "self" {
            connect.push(format!("https://{h}"));
        }
    }
    let connect = connect.join(" ");
    // 'self' is always allowed for scripts and the module fetch of app.wasm. Rendering uses a
    // canvas, so no style sheets are loaded.
    // A prerendered page carries one inline style sheet, allowed by its hash.
    let style = pre.map_or(String::new(), |p| {
        let hash = crate::node::base64(&crate::sha256::digest(p.style.as_bytes()));
        format!(" style-src 'sha256-{hash}';")
    });
    let font = if app.fonts.is_empty() {
        ""
    } else {
        " font-src 'self';"
    };
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval';{style}{font} img-src 'self' https: data:; \
         connect-src {connect}; base-uri 'none'; form-action 'none'"
    )
}

/// CSS `font-weight` and `font-style` descriptors of a declared font.
fn font_descriptors(f: &crate::ast::FontDecl) -> (String, &'static str) {
    let weight = match f.weight {
        Some((lo, hi)) if lo == hi => lo.to_string(),
        Some((lo, hi)) => format!("{lo} {hi}"),
        None => "normal".into(),
    };
    (weight, if f.italic { "italic" } else { "normal" })
}

fn font_face_css(fonts: &[crate::ast::FontDecl]) -> String {
    fonts
        .iter()
        .map(|f| {
            let (weight, style) = font_descriptors(f);
            format!(
                "@font-face{{font-family:\"{}\";src:url(\"{}\");font-weight:{weight};font-style:{style};font-display:swap}}",
                f.family, f.src
            )
        })
        .collect()
}

/// Prerendered pages: an inline style sheet and the markup shown until the canvas takes over.
pub struct Prerendered {
    pub style: String,
    pub body: String,
}

/// Shows the page whose route is the URL's hash (`:target`), or the home page.
const ROUTE_CSS: &str = ".vcwl-route{display:none}\
#vcwl-pre>:target,#vcwl-pre:not(:has(>:target))>.vcwl-home{display:block}";

/// Runs the built application in Node.js for each route (`prerender = true` in vcwl.toml).
fn prerender(out: &Path, app: &crate::app::AppOutput) -> Result<Prerendered, String> {
    let routes = &app.routes;
    let mut list = vec!["/".to_string()];
    list.extend(routes.iter().filter(|r| *r != "/").cloned());
    let quote = |r: &String| {
        let mut q = String::from('"');
        for c in r.chars() {
            match c {
                '"' | '\\' => {
                    q.push('\\');
                    q.push(c);
                }
                c if c.is_control() => q.push_str(&format!("\\u{:04x}", c as u32)),
                c => q.push(c),
            }
        }
        q + "\""
    };
    let json = format!("[{}]", list.iter().map(quote).collect::<Vec<_>>().join(","));
    let res = std::process::Command::new("node")
        .args(["--input-type=module", "-e", PRERENDER_JS])
        .arg(out)
        .arg(&json)
        .output()
        .map_err(|e| format!("prerendering (`prerender = true`) needs Node.js: {e}"))?;
    if !res.status.success() {
        return Err(format!(
            "prerendering failed:\n{}",
            String::from_utf8_lossy(&res.stderr).trim_end()
        ));
    }
    let stdout = String::from_utf8_lossy(&res.stdout);
    let parts: Vec<&str> = stdout.split('\0').collect();
    let (mut rules, mut body) = (Vec::new(), String::from("<div id=\"vcwl-pre\">"));
    for (i, [route, css, html]) in parts.as_chunks::<3>().0.iter().enumerate() {
        for rule in css.split_inclusive('}') {
            if !rules.contains(&rule) {
                rules.push(rule);
            }
        }
        let home = if i == 0 { " vcwl-home" } else { "" };
        body.push_str(&format!(
            "<div class=\"vcwl-route{home}\" id=\"{}\">{html}</div>",
            html_escape(route)
        ));
    }
    body.push_str("</div>");
    let style = format!(
        "{}{}{ROUTE_CSS}{}{}",
        font_face_css(&app.fonts),
        vcwl_render::engine::html::BASE_CSS,
        app.css,
        rules.concat()
    );
    Ok(Prerendered {
        // Nothing in a style sheet may end the element early.
        style: style.replace("</", "<\\/"),
        body,
    })
}

/// The page: metadata and one script. The app's configuration and styles travel inside
/// `app.wasm`; the renderer creates the only element, a canvas.
/// Page metadata from `vcwl.toml` (`description`, `theme_color`, `icon`, `image`, `url`).
#[derive(Default)]
pub struct Meta {
    pub description: Option<String>,
    pub theme_color: Option<String>,
    pub icon: Option<String>,
    pub image: Option<String>,
    pub url: Option<String>,
}

impl Meta {
    fn from_manifest(text: &str) -> Meta {
        let v = |k| manifest_value(text, k);
        Meta {
            description: v("description"),
            theme_color: v("theme_color"),
            icon: v("icon"),
            image: v("image"),
            url: v("url"),
        }
    }

    fn html(&self, title: &str) -> String {
        let mut s = String::new();
        let mut tag = |attr: &str, name: &str, content: &str| {
            s.push_str(&format!(
                "<meta {attr}=\"{name}\" content=\"{}\">\n",
                html_escape(content)
            ));
        };
        if let Some(d) = &self.description {
            tag("name", "description", d);
            tag("property", "og:description", d);
        }
        tag("property", "og:title", title);
        tag("property", "og:type", "website");
        if let Some(u) = &self.url {
            tag("property", "og:url", u);
        }
        if let Some(i) = &self.image {
            let abs = format!("{}{i}", self.url.as_deref().unwrap_or(""));
            tag("property", "og:image", &abs);
            tag("name", "twitter:card", "summary_large_image");
        }
        if let Some(c) = &self.theme_color {
            tag("name", "theme-color", c);
        }
        s
    }
}

pub fn index_html(
    app: &crate::app::AppOutput,
    dev: bool,
    meta: &Meta,
    pre: Option<&Prerendered>,
) -> String {
    let title = &app.title;
    // Fonts download with the modules instead of after them.
    let fonts: String = app
        .fonts
        .iter()
        .map(|f| {
            let ext = f.src.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            format!(
                "<link rel=\"preload\" href=\"{}\" as=\"font\" type=\"font/{ext}\" crossorigin>\n",
                html_escape(&f.src)
            )
        })
        .collect();
    let dev_script = if dev {
        "\n<script type=\"module\" src=\"dev.js\"></script>"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
<title>{title}</title>\n{meta}<link rel=\"icon\" href=\"{icon}\">\n\
<link rel=\"modulepreload\" href=\"vcwl.js\">\n\
<link rel=\"preload\" href=\"render.wasm\" as=\"fetch\" crossorigin>\n\
<link rel=\"preload\" href=\"app.wasm\" as=\"fetch\" crossorigin>\n\
{fonts}<script type=\"module\" src=\"boot.js\"></script>{dev_script}\n{content}\n",
        content = match pre {
            Some(p) => format!("<style>{}</style>\n{}", p.style, p.body),
            None =>
                "<noscript>This application needs JavaScript and WebAssembly enabled.</noscript>"
                    .into(),
        },
        csp = html_escape(&csp(app, pre)),
        title = html_escape(title),
        meta = meta.html(title),
        icon = html_escape(meta.icon.as_deref().unwrap_or("data:,")),
    )
}

/// Appends a custom section to a WebAssembly module.
fn custom_section(wasm: &mut Vec<u8>, name: &str, data: &[u8]) {
    fn leb(out: &mut Vec<u8>, mut v: usize) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            out.push(if v == 0 { b } else { b | 0x80 });
            if v == 0 {
                break;
            }
        }
    }
    let mut body = Vec::new();
    leb(&mut body, name.len());
    body.extend(name.as_bytes());
    body.extend(data);
    wasm.push(0);
    leb(wasm, body.len());
    wasm.extend(body);
}

/// `app.wasm` with the app's styles and configuration attached.
pub fn app_wasm(wasm: &[u8], app: &crate::app::AppOutput, dev: bool) -> Vec<u8> {
    let mut out = wasm.to_vec();
    custom_section(&mut out, "vcwl.styles", app.css.as_bytes());
    // Font names and paths are checked to contain no quotes or backslashes.
    let fonts: Vec<String> = app
        .fonts
        .iter()
        .map(|f| {
            let (weight, style) = font_descriptors(f);
            format!(
                "{{\"family\":\"{}\",\"src\":\"{}\",\"weight\":\"{weight}\",\"style\":\"{style}\"}}",
                f.family, f.src
            )
        })
        .collect();
    let config = format!(
        "{{\"caps\":{},\"dev\":{dev},\"fonts\":[{}]}}",
        app.caps.to_json(),
        fonts.join(",")
    );
    custom_section(&mut out, "vcwl.config", config.as_bytes());
    out
}

/// Compiles the project and writes static artifacts to `out`. Returns the artifact sizes.
pub fn build(entry: &Path, out: &Path, dev: bool) -> Result<Vec<(String, usize)>, String> {
    let (o, _) = compile_entry(entry, dev)?;
    let Some(app) = &o.app else {
        return Err(format!(
            "{}: no `app` declaration; nothing to build (use `vcwl run` for programs with `main`)",
            entry.display()
        ));
    };
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let project = entry.parent().and_then(|d| Project::find(d).ok());
    let meta = project
        .as_ref()
        .map(|p| Meta::from_manifest(&p.manifest))
        .unwrap_or_default();
    let mut files: Vec<(&str, Vec<u8>)> = vec![
        ("app.wasm", app_wasm(&o.wasm, app, dev)),
        ("render.wasm", RENDER_WASM.to_vec()),
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
    // Left over from builds before the canvas renderer.
    let _ = std::fs::remove_file(out.join("app.css"));
    let mut sizes = Vec::new();
    for (name, data) in files {
        std::fs::write(out.join(name), &data)
            .map_err(|e| format!("{}: {e}", out.join(name).display()))?;
        if !name.starts_with('.') {
            sizes.push((name.to_string(), data.len()));
        }
    }
    // Static assets (images, fonts, data) are copied as they are.
    if let Some(p) = &project {
        let public = p.root.join("public");
        if public.is_dir() {
            copy_dir(&public, out, &public)?;
        }
    }
    for f in &app.fonts {
        if !out.join(&f.src).is_file() {
            return Err(format!(
                "font \"{}\": {} not found (put the file in public/{})",
                f.family, f.src, f.src
            ));
        }
    }
    // The page comes last: prerendering runs the files written above.
    let wants_pre = project
        .as_ref()
        .is_some_and(|p| manifest_value(&p.manifest, "prerender").as_deref() == Some("true"));
    let pre = match wants_pre && !dev {
        true => Some(prerender(out, app)?),
        false => None,
    };
    let html = index_html(app, dev, &meta, pre.as_ref());
    std::fs::write(out.join("index.html"), &html)
        .map_err(|e| format!("{}: {e}", out.join("index.html").display()))?;
    sizes.insert(0, ("index.html".to_string(), html.len()));
    Ok(sizes)
}

fn copy_dir(dir: &Path, out: &Path, base: &Path) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    for e in std::fs::read_dir(dir).map_err(|e| err(dir, e))? {
        let path = e.map_err(|e| err(dir, e))?.path();
        let rel = path.strip_prefix(base).unwrap();
        let dest = out.join(rel);
        if path.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| err(&dest, e))?;
            copy_dir(&path, out, base)?;
        } else {
            if rel.parent() == Some(Path::new(""))
                && [
                    "index.html",
                    "app.wasm",
                    "render.wasm",
                    "vcwl.js",
                    "boot.js",
                    "dev.js",
                ]
                .contains(&rel.to_str().unwrap_or(""))
            {
                return Err(format!(
                    "{}: conflicts with a generated file",
                    path.display()
                ));
            }
            std::fs::copy(&path, &dest).map_err(|e| err(&path, e))?;
        }
    }
    Ok(())
}

pub fn clean(p: &Project) -> Result<bool, String> {
    if !p.out.exists() {
        return Ok(false);
    }
    // Only remove the configured output directory, and only if it is inside the project.
    let out = p.out.canonicalize().map_err(|e| e.to_string())?;
    if !out.starts_with(&p.root) || out == p.root || out.join("vcwl.toml").exists() {
        return Err(format!(
            "refusing to delete {}: not a build directory inside the project",
            out.display()
        ));
    }
    std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(true)
}

pub fn new_project(dir: &Path) -> Result<(), String> {
    if dir.exists()
        && dir
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app")
        .to_string();
    let files = [
        (
            "vcwl.toml",
            format!(
                "# VCWL project\nname = \"{name}\"\nentry = \"src/main.vcwl\"\nout = \"dist\"\n"
            ),
        ),
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

/// Collects `path` if it is a file, else every `.vcwl` file below it (sorted), skipping
/// hidden directories, `node_modules` and `target`.
pub fn vcwl_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let rd = std::fs::read_dir(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let hidden = p.file_name().is_some_and(|n| {
            n.to_string_lossy().starts_with('.') || n == "node_modules" || n == "target"
        });
        if p.is_dir() {
            if !hidden {
                vcwl_files(&p, out)?;
            }
        } else if p.extension().is_some_and(|e| e == "vcwl") {
            out.push(p);
        }
    }
    Ok(())
}
