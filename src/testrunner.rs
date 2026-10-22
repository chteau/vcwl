//! `vcwl test [paths...]`: compiles each test file and runs its `test_*` functions in Node.
//! A test passes unless it panics or traps. Without paths, runs `tests/**/*.vcwl` of the project.
use crate::node::{self, TEST_MARK};
use crate::project::{self, Project, compile_entry};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn run(args: &[String]) -> Result<(), String> {
    let roots: Vec<PathBuf> = if args.is_empty() {
        vec![Project::find(Path::new("."))?.root.join("tests")]
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    let mut files = Vec::new();
    for r in &roots {
        project::vcwl_files(r, &mut files)?;
    }
    if files.is_empty() {
        return Err("no test files (*.vcwl) found".into());
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let start = Instant::now();
    let (mut passed, mut failed) = (0, 0);
    for file in &files {
        println!("{}", file.strip_prefix(&cwd).unwrap_or(file).display());
        let (out, _) = match compile_entry(file, true) {
            Ok(o) => o,
            Err(diags) => {
                eprint!("{diags}");
                failed += 1;
                continue;
            }
        };
        let names: Vec<&str> = out
            .exports
            .iter()
            .map(String::as_str)
            .filter(|n| n.starts_with("test_"))
            .collect();
        if names.is_empty() {
            println!("  (no test_* functions)");
            continue;
        }
        let r =
            node::run_tests(&out.wasm, &names).map_err(|e| format!("cannot start node: {e}"))?;
        let mut pending = Vec::new();
        let mut reported = 0;
        for line in r.stdout.lines() {
            match line.strip_prefix(TEST_MARK) {
                Some(m) => {
                    let (ok, name) = m
                        .split_once(' ')
                        .map(|(s, n)| (s == "PASS", n))
                        .unwrap_or((false, m));
                    println!("  {} {name}", if ok { "ok  " } else { "FAIL" });
                    if ok {
                        passed += 1;
                    } else {
                        failed += 1;
                        for l in &pending {
                            println!("       {l}");
                        }
                    }
                    pending.clear();
                    reported += 1;
                }
                None => pending.push(line),
            }
        }
        if reported < names.len() {
            // Instantiation failed or the process died: count every unreported test as failed.
            failed += names.len() - reported;
            println!("  FAIL could not run {} test(s)", names.len() - reported);
            for l in pending.iter().copied().chain(r.stderr.lines()) {
                println!("       {l}");
            }
        }
    }
    println!(
        "\n{passed} passed, {failed} failed in {:.0?}",
        start.elapsed()
    );
    if failed == 0 {
        Ok(())
    } else {
        Err(String::new())
    }
}
