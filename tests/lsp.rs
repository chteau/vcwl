//! Tests of `vcwl lsp` and `vcwl lint`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

struct Client {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Client {
    fn send(&mut self, body: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self) -> String {
        let mut len = 0;
        loop {
            let mut line = String::new();
            self.out.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        self.out.read_exact(&mut body).unwrap();
        String::from_utf8(body).unwrap()
    }

    /// Messages until one contains `needle`.
    fn until(&mut self, needle: &str) -> String {
        for _ in 0..20 {
            let m = self.recv();
            if m.contains(needle) {
                return m;
            }
        }
        panic!("no message with {needle}");
    }
}

fn json_str(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

#[test]
fn language_server_reports_errors_lints_and_formats() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lsp");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("prog.vcwl");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();
    // `file:///C:/...` on Windows.
    let path = file.to_string_lossy().replace('\\', "/");
    let uri = format!(
        "file://{}{path}",
        if path.starts_with('/') { "" } else { "/" }
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_vcwl"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client { child, out };
    c.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    assert!(c.until("\"id\":1").contains("documentFormattingProvider"));

    // Unsaved text with a type error: the diagnostic points at it.
    let broken = "fn main() {\n    let x: Int = \"no\"\n}\n";
    c.send(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"vcwl","version":1,"text":{}}}}}}}"#,
        json_str(broken)
    ));
    let d = c.until("publishDiagnostics");
    assert!(
        d.contains("\"severity\":1") && d.contains("\"line\":1"),
        "{d}"
    );

    // Fixed, with an unused helper: a warning instead.
    let fixed = "fn helper() -> Int {\n  1\n}\n\nfn main() {\n}\n";
    c.send(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":{}}}]}}}}"#,
        json_str(fixed)
    ));
    let d = c.until("publishDiagnostics");
    assert!(
        !d.contains("\"severity\":1") && d.contains("function `helper` is never used"),
        "{d}"
    );

    c.send(&format!(r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/formatting","params":{{"textDocument":{{"uri":"{uri}"}},"options":{{}}}}}}"#));
    let f = c.until("\"id\":2");
    assert!(f.contains("newText") && f.contains("    1"), "{f}");

    c.send(r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#);
    c.until("\"id\":3");
    c.send(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert!(c.child.wait().unwrap().success());
}

#[test]
fn lint_command_lists_warnings() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lint");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.vcwl"),
        "fn unused() -> Int {\n    1\n}\n\nfn main() {\n    print(\"TODO: real text\")\n}\n",
    )
    .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_vcwl"))
        .args(["lint", "a.vcwl"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(!o.status.success());
    assert!(
        out.contains("a.vcwl:1:4: warning[unused-fn]: function `unused` is never used"),
        "{out}"
    );
    assert!(out.contains("a.vcwl:6:11: warning[placeholder]"), "{out}");
}
