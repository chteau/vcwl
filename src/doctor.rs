//! `vcwl doctor`: checks the environment and the project.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::project::{self, Project};

fn line(ok: bool, text: &str) {
    println!("  {} {text}", if ok { "✔" } else { "✘" });
}

fn latest_cached() -> Option<String> {
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("vcwl")
    } else if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(x).join("vcwl")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".cache/vcwl")
    };
    let text = std::fs::read_to_string(dir.join("latest-version")).ok()?;
    let v = text.trim().split_once(' ')?.1.trim().to_string();
    (!v.is_empty()).then_some(v)
}

fn count_rs(dir: &Path, n: &mut usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if !matches!(name.as_ref(), "target" | "node_modules" | ".git" | "dist") {
                count_rs(&p, n);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            *n += 1;
        }
    }
}

pub fn run(_rest: &[String]) -> Result<(), String> {
    let mut failed = false;
    let current = env!("CARGO_PKG_VERSION");
    match latest_cached() {
        Some(l) if l.trim_start_matches('v') != current => {
            line(true, &format!("vcwl {current} (latest known: {l})"));
        }
        Some(_) => line(true, &format!("vcwl {current} (up to date)")),
        None => line(
            true,
            &format!("vcwl {current} (latest version not cached yet)"),
        ),
    }

    let node = Command::new("node")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match node {
        Ok(o) if o.status.success() => line(
            true,
            &format!("node {} on PATH", String::from_utf8_lossy(&o.stdout).trim()),
        ),
        _ => {
            failed = true;
            line(
                false,
                "node not found on PATH (needed for `vcwl run` and `vcwl test`)",
            );
        }
    }

    match Project::find(Path::new(".")) {
        Ok(p) => {
            line(
                true,
                &format!("vcwl.toml loads (entry {})", p.entry.display()),
            );
            match project::compile_entry(&p.entry, true) {
                Ok(_) => line(true, "entry compiles"),
                Err(e) => {
                    failed = true;
                    line(false, "entry does not compile");
                    for l in e.lines() {
                        println!("      {l}");
                    }
                }
            }
            let mut n = 0;
            count_rs(&p.root, &mut n);
            if n == 0 {
                line(
                    true,
                    "Rust contamination: VCWL is not Rust; 0 crabs detected",
                );
            } else {
                line(
                    true,
                    &format!("Rust contamination: {n} .rs file(s) found; {n} crab(s) detected"),
                );
            }
        }
        Err(e) => {
            failed = true;
            line(false, &format!("vcwl.toml: {}", e.trim()));
        }
    }

    line(
        true,
        "browser: needs WebAssembly GC (Chrome 119+, Firefox 120+, Safari 18.2+)",
    );
    if failed { Err(String::new()) } else { Ok(()) }
}
