use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vcwl::project::{self, Project};

const USAGE: &str = "usage: vcwl <command> [args]

commands:
  new <dir>              create a starter application
  dev [--port N]         build in development mode, serve and rebuild on changes
  check [file]           parse and type-check without producing artifacts
  build [--dev]          compile the project to static files in dist/
  serve [--port N]       release build served with its server functions (`server fn`)
  test [paths]           run `test_*` functions (default: tests/**/*.vcwl)
  fmt [--check] [files]  format source files
  clean                  remove build artifacts
  run <file>             compile a program and run its `main` with Node.js
  compile <file> [-o f]  compile a single file to a .wasm module

packages (the registry is the `registry` branch of a git repository):
  add <name>[@req]       add a registry dependency and install it
  remove <name>          remove a dependency
  install                install registry dependencies into vcwl_packages/ (writes vcwl.lock)
  publish [--dry-run]    publish this package to the registry
  search [word]          list registry packages";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    let r = match cmd.as_str() {
        "new" => cmd_new(rest),
        "build" => cmd_build(rest),
        "check" => cmd_check(rest),
        "clean" => cmd_clean(),
        "run" => cmd_run(rest),
        "compile" => cmd_compile(rest),
        "dev" => vcwl::devserver::run(rest),
        "serve" => vcwl::server::run(rest),
        "test" => vcwl::testrunner::run(rest),
        "fmt" => vcwl::fmt::run(rest),
        "install" | "add" | "remove" | "publish" | "search" => vcwl::pkg::run(cmd, rest),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprint!("{e}");
            if !e.ends_with('\n') {
                eprintln!();
            }
            ExitCode::FAILURE
        }
    }
}

fn project() -> Result<Project, String> {
    Project::find(Path::new("."))
}

fn cmd_new(rest: &[String]) -> Result<(), String> {
    let dir = rest.first().ok_or("usage: vcwl new <dir>")?;
    project::new_project(Path::new(dir))?;
    println!("created {dir}\n\n  cd {dir}\n  vcwl dev");
    Ok(())
}

fn cmd_build(rest: &[String]) -> Result<(), String> {
    let p = project()?;
    let dev = rest.iter().any(|a| a == "--dev");
    let start = std::time::Instant::now();
    let sizes = project::build(&p.entry, &p.out, dev)?;
    let total: usize = sizes.iter().map(|s| s.1).sum();
    for (name, n) in &sizes {
        println!("  {name:<12} {n:>8} bytes");
    }
    println!(
        "built {} ({} bytes, {} mode) in {:.0?}",
        p.out.display(),
        total,
        if dev { "dev" } else { "release" },
        start.elapsed()
    );
    Ok(())
}

fn cmd_check(rest: &[String]) -> Result<(), String> {
    let entry = match rest.first() {
        Some(f) => PathBuf::from(f),
        None => project()?.entry,
    };
    project::compile_entry(&entry, true)?;
    println!("ok: {}", entry.display());
    Ok(())
}

fn cmd_clean() -> Result<(), String> {
    let p = project()?;
    if project::clean(&p)? {
        println!("removed {}", p.out.display());
    } else {
        println!("nothing to clean");
    }
    Ok(())
}

fn cmd_compile(rest: &[String]) -> Result<(), String> {
    let file = rest
        .first()
        .ok_or("usage: vcwl compile <file> [-o out.wasm]")?;
    let (o, _) = project::compile_entry(Path::new(file), false)?;
    let dest = rest
        .iter()
        .position(|a| a == "-o")
        .and_then(|i| rest.get(i + 1))
        .cloned()
        .unwrap_or_else(|| {
            PathBuf::from(file)
                .with_extension("wasm")
                .display()
                .to_string()
        });
    std::fs::write(&dest, &o.wasm).map_err(|e| format!("cannot write {dest}: {e}"))?;
    println!("wrote {dest} ({} bytes)", o.wasm.len());
    Ok(())
}

fn cmd_run(rest: &[String]) -> Result<(), String> {
    let file = rest.first().ok_or("usage: vcwl run <file>")?;
    let (o, _) = project::compile_entry(Path::new(file), true)?;
    if !o.exports.iter().any(|e| e == "main") {
        return Err(format!("{file}: no `fn main` to run"));
    }
    let r = vcwl::node::run_main(&o.wasm, "main").map_err(|e| format!("cannot start node: {e}"))?;
    print!("{}", r.stdout);
    eprint!("{}", r.stderr);
    if r.ok { Ok(()) } else { Err(String::new()) }
}
