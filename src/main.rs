use std::path::PathBuf;
use std::process::ExitCode;

use vcwl::compile::{self, Options};
use vcwl::diag::SourceMap;

fn usage() -> ExitCode {
    eprintln!("usage: vcwl <command> [args]

commands:
  check <file>          type-check a program and report diagnostics
  compile <file> [-o out.wasm]
                        compile a program to a WebAssembly module
  run <file>            compile and run `main` with Node.js");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { return usage() };
    let rest = &args[1..];
    match cmd.as_str() {
        "check" | "compile" | "run" => {
            let Some(file) = rest.first() else { return usage() };
            let mut sm = SourceMap::default();
            let out = match compile::compile(&PathBuf::from(file), &mut sm, &Options { dev: true }) {
                Ok(o) => o,
                Err(f) => {
                    for d in &f.diags {
                        eprint!("{}", d.render(&sm));
                    }
                    return ExitCode::FAILURE;
                }
            };
            match cmd.as_str() {
                "check" => {
                    eprintln!("ok: {file}");
                    ExitCode::SUCCESS
                }
                "compile" => {
                    let dest = rest.iter().position(|a| a == "-o").and_then(|i| rest.get(i + 1)).cloned()
                        .unwrap_or_else(|| PathBuf::from(file).with_extension("wasm").display().to_string());
                    if let Err(e) = std::fs::write(&dest, &out.wasm) {
                        eprintln!("error: cannot write {dest}: {e}");
                        return ExitCode::FAILURE;
                    }
                    eprintln!("wrote {dest} ({} bytes)", out.wasm.len());
                    ExitCode::SUCCESS
                }
                _ => match vcwl::node::run_main(&out.wasm, "main") {
                    Ok(r) => {
                        print!("{}", r.stdout);
                        eprint!("{}", r.stderr);
                        if r.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                },
            }
        }
        _ => usage(),
    }
}
