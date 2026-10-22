//! End-to-end compiler tests: programs are compiled, validated by `wasmparser` and executed
//! by Node.js; invalid programs must fail with the expected diagnostic.

use std::path::{Path, PathBuf};

use vcwl::compile::{Options, compile};
use vcwl::diag::SourceMap;

fn vcwl_files(dir: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "vcwl"))
        .collect();
    v.sort();
    v
}

fn directives(src: &str, key: &str) -> Vec<String> {
    let prefix = format!("// {key}: ");
    src.lines()
        .filter_map(|l| l.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

#[test]
fn programs_run_with_expected_output() {
    let mut failures = Vec::new();
    for path in vcwl_files("tests/programs") {
        let src = std::fs::read_to_string(&path).unwrap();
        let expected = directives(&src, "expect");
        let mut sm = SourceMap::default();
        let out = match compile(&path, &mut sm, &Options { dev: true }) {
            Ok(o) => o,
            Err(f) => {
                failures.push(format!(
                    "{}: failed to compile:\n{}",
                    path.display(),
                    f.diags.iter().map(|d| d.render(&sm)).collect::<String>()
                ));
                continue;
            }
        };
        vcwl::codegen::validate(&out.wasm).unwrap();
        let r = vcwl::node::run_main(&out.wasm, "main").expect("node is required for tests");
        let got: Vec<&str> = r.stdout.lines().collect();
        if got != expected || !r.ok {
            failures.push(format!(
                "{}:\n  expected {:?}\n  got      {:?}\n  stderr   {}",
                path.display(),
                expected,
                got,
                r.stderr
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn invalid_programs_are_rejected() {
    let mut failures = Vec::new();
    for path in vcwl_files("tests/errors") {
        let src = std::fs::read_to_string(&path).unwrap();
        let expected = directives(&src, "error");
        assert!(
            !expected.is_empty(),
            "{} has no `// error:` directive",
            path.display()
        );
        let mut sm = SourceMap::default();
        match compile(&path, &mut sm, &Options { dev: true }) {
            Ok(_) => failures.push(format!("{}: compiled but should fail", path.display())),
            Err(f) => {
                let text: String = f.diags.iter().map(|d| d.render(&sm)).collect();
                for e in &expected {
                    if !text.contains(e.as_str()) {
                        failures.push(format!(
                            "{}: expected diagnostic containing {e:?}, got:\n{text}",
                            path.display()
                        ));
                    }
                }
                if !text.contains(":") || !text.contains("-->") {
                    failures.push(format!(
                        "{}: diagnostic lacks a location:\n{text}",
                        path.display()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn apps_compile_to_valid_modules() {
    for path in vcwl_files("tests/apps") {
        let mut sm = SourceMap::default();
        let out = compile(&path, &mut sm, &Options { dev: false }).unwrap_or_else(|f| {
            panic!(
                "{}: {}",
                path.display(),
                f.diags.iter().map(|d| d.render(&sm)).collect::<String>()
            )
        });
        vcwl::codegen::validate(&out.wasm).unwrap();
        let css = &out.app.as_ref().unwrap().css;
        assert!(css.contains(":appear"), "{css}");
    }
}
