//! Runs compiled modules with Node.js (V8), an engine independent of this compiler.

use std::io::Write;
use std::process::{Command, Stdio};

pub const RUNTIME_JS: &str = include_str!("../runtime/vcwl.js");

/// Starts a `PASS name` / `FAIL name` line in `run_tests` output; a test's own output
/// (prints and the failure message) comes before its marker line.
pub const TEST_MARK: char = '\u{1}';

pub struct RunResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

fn runtime_url() -> String {
    format!(
        "data:text/javascript;base64,{}",
        base64(RUNTIME_JS.as_bytes())
    )
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Instantiates the module (read from stdin) and calls each export in `calls`, printing
/// `name => value` for functions that return a value.
fn script(calls: &[&str], tests: bool) -> String {
    format!(
        r#"import {{ createHost, instantiate, describeError }} from "{url}";
const chunks = [];
for await (const c of process.stdin) chunks.push(c);
const host = createHost();
let failed = false;
try {{
  const inst = await instantiate(Buffer.concat(chunks), host);
  for (const name of {calls:?}) {{
    try {{
      const r = inst.exports[name]();
      if ({tests}) console.log("{mark}PASS " + name);
      else if (r !== undefined) console.log(String(r));
    }} catch (e) {{
      failed = true;
      console.log(describeError(e));
      if ({tests}) console.log("{mark}FAIL " + name);
    }}
  }}
}} catch (e) {{
  failed = true;
  console.error(describeError(e));
}}
process.exitCode = failed ? 1 : 0;
"#,
        url = runtime_url(),
        calls = calls,
        tests = tests,
        mark = TEST_MARK.escape_unicode(),
    )
}

fn run(wasm: &[u8], script: &str) -> std::io::Result<RunResult> {
    // The script embeds the runtime, too long for a command line on Windows (32K): it goes in
    // a file.
    static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let file = std::env::temp_dir().join(format!(
        "vcwl-run-{}-{}.mjs",
        std::process::id(),
        RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&file, script)?;
    let out = Command::new("node")
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(wasm)?;
            child.wait_with_output()
        });
    let _ = std::fs::remove_file(&file);
    let out = out?;
    Ok(RunResult {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

pub fn run_main(wasm: &[u8], export: &str) -> std::io::Result<RunResult> {
    run(wasm, &script(&[export], false))
}

pub fn run_tests(wasm: &[u8], names: &[&str]) -> std::io::Result<RunResult> {
    run(wasm, &script(names, true))
}
