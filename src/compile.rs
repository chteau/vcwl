//! Compiler driver: load files, check, lower, emit, validate.

use std::path::{Path, PathBuf};

use crate::ast;
use crate::check::Checker;
use crate::diag::{Diag, SourceMap, Span};
use crate::{codegen, lexer, lower, opt, parser};

pub const PRELUDE: &str = include_str!("prelude.vcwl");

pub struct Output {
    pub wasm: Vec<u8>,
    /// Exported function names (for tests and the `run` command).
    pub exports: Vec<String>,
    pub app: Option<crate::app::AppOutput>,
    /// The server module (`server fn`s exported as `__fn_<name>`), if the program has any.
    pub server: Option<Vec<u8>>,
}

pub struct Failure {
    pub diags: Vec<Diag>,
}

/// Loads `entry` and everything it imports. File 0 is always the prelude.
pub fn load(
    entry: &Path,
    sm: &mut SourceMap,
) -> Result<(Vec<ast::File>, Vec<Vec<usize>>), Vec<Diag>> {
    let mut files = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut imports: Vec<Vec<usize>> = Vec::new();
    let mut next_id = 0u32;

    let base = sm.add("<prelude>", PRELUDE.to_string());
    let toks = lexer::lex(PRELUDE, base).map_err(|d| vec![d])?;
    files.push(parser::parse_file("<prelude>", &toks, &mut next_id).map_err(|d| vec![d])?);
    paths.push(PathBuf::new());
    imports.push(Vec::new());

    let mut queue = vec![(entry.to_path_buf(), None::<(usize, Span)>)];
    while let Some((path, from)) = queue.pop() {
        let canon = path.canonicalize().unwrap_or(path.clone());
        if let Some(i) = paths.iter().position(|p| *p == canon) {
            if let Some((f, _)) = from {
                imports[f].push(i);
            }
            continue;
        }
        let overlay = OVERLAY.with(|o| o.borrow().get(&canon).cloned());
        let text = match overlay.map_or_else(|| std::fs::read_to_string(&path), Ok) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("cannot read `{}`: {e}", path.display());
                return Err(vec![match from {
                    Some((_, span)) => Diag::compile(span, msg),
                    None => Diag::compile(Span::default(), msg),
                }]);
            }
        };
        let display = path.display().to_string();
        let base = sm.add(&display, text.clone());
        let toks = lexer::lex(&text, base).map_err(|d| vec![d])?;
        let file = parser::parse_file(&display, &toks, &mut next_id).map_err(|d| vec![d])?;
        let idx = files.len();
        if let Some((f, _)) = from {
            imports[f].push(idx);
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for item in &file.items {
            if let ast::Item::Import(im) = item {
                if let Some(spec) = im.path.strip_prefix('@') {
                    let file = crate::project::resolve_package(&dir, spec)
                        .map_err(|e| vec![Diag::compile(im.span, e)])?;
                    queue.push((file, Some((idx, im.span))));
                    continue;
                }
                if !im.path.ends_with(".vcwl") {
                    return Err(vec![
                        Diag::compile(im.span, "import paths must name a `.vcwl` file")
                            .help("e.g. `import \"./components/card.vcwl\"`"),
                    ]);
                }
                queue.push((dir.join(&im.path), Some((idx, im.span))));
            }
        }
        files.push(file);
        paths.push(canon);
        imports.push(Vec::new());
    }
    Ok((files, imports))
}

thread_local! {
    /// Unsaved editor contents by canonical path, read instead of the files (language server).
    pub static OVERLAY: std::cell::RefCell<std::collections::HashMap<PathBuf, String>> = Default::default();
}

pub struct Options {
    pub dev: bool,
}

pub fn compile(entry: &Path, sm: &mut SourceMap, opts: &Options) -> Result<Output, Failure> {
    let (files, imports) = load(entry, sm).map_err(|diags| Failure { diags })?;
    let mut ck = Checker::new(&files);
    ck.declare(&imports);
    if !ck.diags.is_empty() {
        return Err(Failure {
            diags: std::mem::take(&mut ck.diags),
        });
    }
    ck.check_fns();
    let app = crate::app::check_app(&mut ck);
    let mut entries = Vec::new();
    for f in 0..ck.sigs.len() {
        if !ck.sigs[f].server {
            continue;
        }
        let ast::Item::Fn(decl) = &ck.files[ck.sigs[f].file].items[ck.sigs[f].item] else {
            unreachable!()
        };
        match ck.server_entry(f, decl.name.span) {
            Ok(id) => entries.push((id, ck.sigs[f].name.clone())),
            Err(d) => ck.diags.push(d),
        }
    }
    if !ck.diags.is_empty() {
        return Err(Failure {
            diags: std::mem::take(&mut ck.diags),
        });
    }
    let fns: Vec<_> = ck.fns.iter().map(|f| f.clone().unwrap()).collect();
    let mut lw = lower::Lower::new(&ck.adts, &fns);
    let mut exports = Vec::new();
    // `main` in the entry file is exported; so are `test_*` functions in test files.
    for (id, sig) in ck.sigs.iter().enumerate() {
        let entry_file = sig.file == 1;
        let is_test =
            sig.name.starts_with("test_") && sig.params.is_empty() && sig.generics.is_empty();
        if sig.file != 0 && entry_file && (sig.name == "main" && sig.generics.is_empty() || is_test)
        {
            lw.export_fn(id, &sig.name);
            exports.push(sig.name.clone());
        }
    }
    let app_out = match app {
        Some(a) => Some(
            crate::app::lower_app(&mut lw, sm, a, opts).map_err(|d| Failure { diags: vec![d] })?,
        ),
        None => None,
    };
    lw.drain();
    let wasm = finish(lw.m)?;
    if let Ok(path) = std::env::var("VCWL_DUMP_WASM") {
        let _ = std::fs::write(path, &wasm);
    }
    // Server functions only reach the client as URLs: their code is in this module alone.
    let server = if entries.is_empty() {
        None
    } else {
        let mut sw = lower::Lower::new(&ck.adts, &fns);
        for (id, name) in &entries {
            sw.export_fn(*id, &format!("__fn_{name}"));
        }
        sw.drain();
        Some(finish(sw.m)?)
    };
    Ok(Output {
        wasm,
        exports,
        app: app_out,
        server,
    })
}

fn finish(mut module: crate::ir::Module) -> Result<Vec<u8>, Failure> {
    opt::optimize(&mut module);
    let wasm = codegen::emit(&module);
    if let Err(e) = codegen::validate(&wasm) {
        return Err(Failure {
            diags: vec![Diag::compile(
                Span::default(),
                format!("internal compiler error: generated module is invalid: {e}"),
            )],
        });
    }
    Ok(wasm)
}
