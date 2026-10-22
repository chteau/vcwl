//! Web application compilation: components, styles and capabilities.

use crate::ast;
use crate::check::Checker;
use crate::check_ui::{CompSig, TComp};
use crate::compile::Options;
use crate::diag::{Diag, Result, SourceMap, Span};
use crate::lexer::Tok;
use crate::lower::Lower;
use crate::style::Sheet;

pub struct AppInfo {
    pub title: String,
    pub fonts: Vec<ast::FontDecl>,
    pub caps: Capabilities,
    pub comps: Vec<TComp>,
    pub sigs: Vec<CompSig>,
    pub root: usize,
    pub css: String,
}

#[derive(Default, Clone, Debug)]
pub struct Capabilities {
    /// Hosts the application may contact (`self` = same origin).
    pub network: Vec<String>,
    /// `none`, `local` or `session`.
    pub storage: String,
}

impl Capabilities {
    pub fn to_json(&self) -> String {
        let hosts: Vec<String> = self.network.iter().map(|h| format!("\"{h}\"")).collect();
        format!(
            "{{\"network\":[{}],\"storage\":\"{}\"}}",
            hosts.join(","),
            self.storage
        )
    }
}

pub struct AppOutput {
    pub title: String,
    pub css: String,
    pub caps: Capabilities,
    pub fonts: Vec<ast::FontDecl>,
    /// Literal paths of every `routes` arm, in source order.
    pub routes: Vec<String>,
}

fn collect_routes(views: &[crate::check_ui::TView], out: &mut Vec<String>) {
    use crate::check_ui::{SwitchKind, TView};
    for v in views {
        match v {
            TView::Element(e) => collect_routes(&e.children, out),
            TView::Switch { kind, branches } => {
                if let SwitchKind::Routes(paths) = kind {
                    for p in paths.iter().flatten() {
                        if !out.contains(p) {
                            out.push(p.clone());
                        }
                    }
                }
                for b in branches {
                    collect_routes(b, out);
                }
            }
            TView::For { body, .. } | TView::Component { children: body, .. } => {
                collect_routes(body, out)
            }
            TView::Text(_) | TView::Slot => {}
        }
    }
}

/// Checks components and the `app` declaration of the entry file (file 1).
pub fn check_app(ck: &mut Checker) -> Option<AppInfo> {
    let files = ck.files;
    let mut sheet = Sheet::default();
    let mut fonts = Vec::new();
    for f in files {
        for item in &f.items {
            if let ast::Item::Font(font) = item {
                match check_font(font) {
                    Ok(()) => fonts.push(font.clone()),
                    Err(d) => ck.diags.push(d),
                }
            }
            if let ast::Item::Style(s) = item {
                match sheet.block(&s.block) {
                    Ok(c) => sheet.named.push(c),
                    Err(d) => {
                        ck.diags.push(d);
                        sheet.named.push(String::new());
                    }
                }
            }
        }
    }
    let sigs = ck.declare_components();
    let mut comps = Vec::new();
    for sig in &sigs {
        let ast::Item::Component(c) = &files[sig.file].items[sig.item] else {
            unreachable!()
        };
        ck.file = sig.file;
        match ck.check_component(c, &sigs, &mut sheet) {
            Ok((tc, _)) => comps.push(tc),
            Err(d) => {
                ck.diags.push(d);
                return None;
            }
        }
    }
    let apps: Vec<&ast::AppDecl> = files
        .get(1)
        .map(|f| {
            f.items
                .iter()
                .filter_map(|i| {
                    if let ast::Item::App(a) = i {
                        Some(a)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    for f in files.iter().skip(2) {
        for i in &f.items {
            if let ast::Item::App(a) = i {
                ck.diags.push(Diag::compile(
                    a.span,
                    "`app` can only be declared in the entry file",
                ));
            }
        }
    }
    let app = match apps.as_slice() {
        [] => return None,
        [a] => *a,
        [_, b, ..] => {
            ck.diags.push(Diag::compile(
                b.span,
                "only one `app` declaration is allowed",
            ));
            return None;
        }
    };
    let Some(root) = sigs.iter().position(|s| s.name == "App" && s.file == 1) else {
        ck.diags.push(
            Diag::compile(
                app.span,
                "an app needs a component named `App` in the entry file",
            )
            .help("add `component App { ... }`"),
        );
        return None;
    };
    if !sigs[root].params.is_empty() {
        ck.diags.push(Diag::compile(
            app.span,
            "the `App` component cannot take properties",
        ));
        return None;
    }
    let caps = match capabilities(app) {
        // Server functions are called on the app's own origin.
        Ok(mut c) if ck.sigs.iter().any(|s| s.server) && !c.network.iter().any(|h| h == "self") => {
            c.network.push("self".into());
            c
        }
        Ok(c) => c,
        Err(d) => {
            ck.diags.push(d);
            return None;
        }
    };
    if let Some(span) = ck.fetch_span
        && caps.network.is_empty()
    {
        ck.diags.push(Diag::compile(span, "`fetch` requires the network capability").help(format!(
            "declare the hosts the app may contact: `app \"{}\" {{ capabilities {{ network: allow(\"self\") }} }}`",
            app.title
        )));
        return None;
    }
    Some(AppInfo {
        title: app.title.clone(),
        fonts,
        caps,
        comps,
        sigs,
        root,
        css: sheet.css,
    })
}

fn capabilities(app: &ast::AppDecl) -> Result<Capabilities> {
    let mut caps = Capabilities {
        network: vec![],
        storage: "none".into(),
    };
    let mut seen: Vec<&str> = Vec::new();
    for (name, toks, span) in &app.capabilities {
        if seen.contains(&name.name.as_str()) {
            return Err(Diag::compile(
                name.span,
                format!("capability `{}` is declared twice", name.name),
            ));
        }
        seen.push(&name.name);
        let words: Vec<&Tok> = toks.iter().map(|t| &t.tok).collect();
        match name.name.as_str() {
            "network" => match words.as_slice() {
                [Tok::Ident(n)] if n == "none" => {}
                [Tok::Ident(a), Tok::Punct("("), rest @ .., Tok::Punct(")")] if a == "allow" => {
                    for (i, t) in rest.iter().enumerate() {
                        match (i % 2, t) {
                            (0, Tok::Str(p)) => {
                                let host = match p.as_slice() {
                                    [crate::lexer::StrPiece::Lit(s)] => s.clone(),
                                    _ => {
                                        return Err(Diag::compile(
                                            *span,
                                            "hosts cannot contain interpolation",
                                        ));
                                    }
                                };
                                if host != "self" && !valid_host(&host) {
                                    return Err(Diag::compile(*span, format!("invalid host \"{host}\""))
                                        .help("write a host name like \"api.example.com\" (no scheme or path), or \"self\" for the app's own origin"));
                                }
                                caps.network.push(host);
                            }
                            (1, Tok::Punct(",")) => {}
                            _ => return Err(cap_err(*span, "network")),
                        }
                    }
                    if caps.network.is_empty() {
                        return Err(cap_err(*span, "network"));
                    }
                }
                _ => return Err(cap_err(*span, "network")),
            },
            "storage" => match words.as_slice() {
                [Tok::Ident(s)] if ["none", "local", "session"].contains(&s.as_str()) => {
                    caps.storage = s.clone()
                }
                _ => return Err(cap_err(*span, "storage")),
            },
            other => {
                return Err(
                    Diag::compile(name.span, format!("unknown capability `{other}`"))
                        .help("supported capabilities: network, storage"),
                );
            }
        }
    }
    Ok(caps)
}

fn valid_host(h: &str) -> bool {
    let (host, port) = h.split_once(':').unwrap_or((h, ""));
    !host.is_empty()
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && port.chars().all(|c| c.is_ascii_digit())
}

/// Font names and paths end up in style sheets and URLs: keep them to plain text.
fn check_font(f: &ast::FontDecl) -> Result<()> {
    let plain = |s: &str| {
        !s.chars()
            .any(|c| c.is_control() || "\"'\\;{}()<>".contains(c))
    };
    if f.family.trim().is_empty() || !plain(&f.family) {
        return Err(Diag::compile(f.span, "invalid font family name")
            .help("use letters, digits, spaces and dashes, e.g. `font \"Inter\" from ...`"));
    }
    let ext = f.src.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let relative =
        !f.src.starts_with('/') && !f.src.contains(':') && !f.src.split('/').any(|p| p == "..");
    if !plain(&f.src) || f.src.contains(' ') || !relative {
        return Err(Diag::compile(
            f.span,
            "the font file must be a relative path inside `public/`",
        )
        .help("e.g. `font \"Inter\" from \"fonts/Inter.woff2\"` for public/fonts/Inter.woff2"));
    }
    if !matches!(ext.as_deref(), Some("woff2" | "woff" | "ttf" | "otf")) {
        return Err(Diag::compile(
            f.span,
            "font files must be .woff2, .woff, .ttf or .otf",
        ));
    }
    if let Some((lo, hi)) = f.weight
        && !(1 <= lo && lo <= hi && hi <= 1000)
    {
        return Err(Diag::compile(
            f.span,
            "font weights go from 1 to 1000 (`weight 400` or `weight 100..900`)",
        ));
    }
    Ok(())
}

fn cap_err(span: Span, name: &str) -> Diag {
    let help = match name {
        "network" => "write `network: allow(\"api.example.com\", \"self\")` or `network: none`",
        _ => "write `storage: local`, `storage: session` or `storage: none`",
    };
    Diag::compile(span, format!("invalid `{name}` capability")).help(help)
}

pub fn lower_app(
    lw: &mut Lower,
    sm: &SourceMap,
    app: AppInfo,
    opts: &Options,
) -> Result<AppOutput> {
    let mut routes = Vec::new();
    for c in &app.comps {
        collect_routes(&c.body, &mut routes);
    }
    let ui =
        crate::lower_ui::UiState::new(app.comps, app.sigs, sm.clone(), opts.dev, app.title.clone());
    lw.ui = Some(Box::new(ui));
    lw.lower_components(app.root);
    Ok(AppOutput {
        title: app.title,
        css: app.css,
        caps: app.caps,
        fonts: app.fonts,
        routes,
    })
}
