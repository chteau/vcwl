//! Tests of the `vcwl` command-line tool.

use std::path::Path;
use std::process::{Command, Output};

fn vcwl(dir: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vcwl"))
        .args(args)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(dir))
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn test_command_runs_project_tests() {
    let o = vcwl("examples/todo", &["test"]);
    assert!(
        o.status.success(),
        "{}{}",
        stdout(&o),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(stdout(&o).contains("4 passed, 0 failed"), "{}", stdout(&o));
}

#[test]
fn test_command_reports_failures() {
    let o = vcwl("tests/cli", &["test", "failing"]);
    let out = stdout(&o);
    assert!(!o.status.success());
    assert!(out.contains("ok   test_passes"), "{out}");
    assert!(
        out.contains("FAIL test_fails\n       context line\n       panic: deliberate failure"),
        "{out}"
    );
    assert!(out.contains("1 passed, 1 failed"), "{out}");
}

fn http_get(addr: &str, path: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nhost: {addr}\r\n\r\n").unwrap();
    let mut r = Vec::new();
    s.read_to_end(&mut r).unwrap();
    String::from_utf8_lossy(&r).into_owned()
}

fn wait_for(addr: &str, path: &str, cond: impl Fn(&str) -> bool) -> String {
    for _ in 0..100 {
        let r = http_get(addr, path);
        if cond(&r) {
            return r;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("timed out waiting on {path}: {}", http_get(addr, path));
}

#[test]
fn dev_server_serves_and_rebuilds() {
    use std::io::{BufRead, BufReader};
    let dir = std::env::temp_dir().join(format!("vcwl-dev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("vcwl.toml"),
        "entry = \"src/main.vcwl\"\nout = \"dist\"\n",
    )
    .unwrap();
    let main = dir.join("src/main.vcwl");
    let app = "app \"Dev\"\n\ncomponent App {\n    paragraph(\"one\")\n}\n";
    std::fs::write(&main, app).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_vcwl"))
        .args(["dev", "--port", "0"])
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let serving = lines
        .by_ref()
        .map(Result::unwrap)
        .find(|l| l.contains("http://"));
    // Keep draining the server's output so its writes never block.
    std::thread::spawn(move || lines.for_each(drop));

    let result = std::panic::catch_unwind(|| {
        let serving = serving.expect("dev server did not start");
        let addr = serving
            .split("http://")
            .nth(1)
            .and_then(|a| a.split('/').next())
            .unwrap()
            .to_string();
        assert!(http_get(&addr, "/").contains("src=\"dev.js\""));
        assert!(http_get(&addr, "/app.wasm").contains("content-type: application/wasm"));
        assert!(http_get(&addr, "/__vcwl/status").ends_with("\r\n\r\n1\n"));
        assert!(http_get(&addr, "/../vcwl.toml").starts_with("HTTP/1.1 400"));
        assert!(http_get(&addr, "/missing.js").starts_with("HTTP/1.1 404"));

        std::fs::write(&main, app.replace("\"one\"", "1 + ")).unwrap();
        let r = wait_for(&addr, "/__vcwl/status", |r| r.contains("main.vcwl:"));
        assert!(
            r.contains("\r\n\r\n1\n"),
            "a failed build keeps the version: {r}"
        );

        std::fs::write(&main, app.replace("\"one\"", "\"two\"")).unwrap();
        wait_for(&addr, "/__vcwl/status", |r| r.ends_with("\r\n\r\n2\n"));
    });
    child.kill().unwrap();
    child.wait().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    result.unwrap();
}

#[test]
fn repository_sources_are_formatted() {
    let o = vcwl(
        ".",
        &[
            "fmt",
            "--check",
            "src",
            "examples",
            "tests/programs",
            "tests/cli",
            "tests/browser/fixtures",
        ],
    );
    assert!(
        o.status.success(),
        "{}{}",
        stdout(&o),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn build_checks_fonts_and_allows_the_prerendered_style_by_hash() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("font-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("vcwl.toml"),
        "entry = \"src/main.vcwl\"\nout = \"dist\"\nprerender = true\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.vcwl"),
        "app \"T\"\n\nfont \"Tuffy\" from \"fonts/t.ttf\"\n\ncomponent App {\n    text(\"<hi>\")\n}\n",
    )
    .unwrap();
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_vcwl"))
            .arg("build")
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let o = build();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("fonts/t.ttf not found"),
        "{err}"
    );
    std::fs::create_dir_all(dir.join("public/fonts")).unwrap();
    let font = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/browser/fixtures/fonts/public/fonts/tuffy-bold.ttf");
    std::fs::copy(font, dir.join("public/fonts/t.ttf")).unwrap();
    let o = build();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let html = std::fs::read_to_string(dir.join("dist/index.html")).unwrap();
    let style = html
        .split("<style>")
        .nth(1)
        .unwrap()
        .split("</style>")
        .next()
        .unwrap();
    let hash = vcwl::node::base64(&vcwl::sha256::digest(style.as_bytes()));
    assert!(
        html.contains(&format!("style-src 'sha256-{hash}'")),
        "{html}"
    );
    assert!(html.contains("font-src 'self'"), "{html}");
    assert!(html.contains(">&lt;hi&gt;<"), "escaped text: {html}");
    // Google Fonts: no file, a style sheet link, and the CSP allows Google's hosts.
    std::fs::write(
        dir.join("src/main.vcwl"),
        "app \"T\"\n\nfont \"Tuffy\" from \"fonts/t.ttf\"\nfont \"Space Grotesk\" from google weight 300..700\nfont \"Space Grotesk\" from google weight 400 italic\n\ncomponent App {\n    text(\"hi\")\n}\n",
    )
    .unwrap();
    let o = build();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let html = std::fs::read_to_string(dir.join("dist/index.html")).unwrap();
    assert!(
        html.contains("href=\"https://fonts.googleapis.com/css2?family=Space+Grotesk:ital,wght@0,300..700;1,400&amp;display=swap\""),
        "{html}"
    );
    assert!(
        html.contains("font-src 'self' https://fonts.gstatic.com;"),
        "{html}"
    );
    assert!(html.contains("https://fonts.googleapis.com;"), "{html}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn path_routing_writes_a_page_per_route_and_rejects_unsafe_routes() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("path-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let write = |routing: &str, route: &str| {
        std::fs::write(
            dir.join("vcwl.toml"),
            format!("entry = \"src/main.vcwl\"\nout = \"dist\"\nrouting = \"{routing}\"\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("src/main.vcwl"),
            format!(
                "app \"T\"\n\ncomponent App {{\n    routes {{\n        \"{route}\" => text(\"a\")\n        _ => text(\"b\")\n    }}\n}}\n"
            ),
        )
        .unwrap();
        Command::new(env!("CARGO_BIN_EXE_vcwl"))
            .arg("build")
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let o = write("pushstate", "/a");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("routing = \"pushstate\""),
        "{err}"
    );
    let o = write("path", "/a/../b");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("cannot be a page"),
        "{err}"
    );
    let o = write("path", "/docs/a");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let page = std::fs::read_to_string(dir.join("dist/docs/a/index.html")).unwrap();
    assert!(page.contains("src=\"../../boot.js\""), "{page}");
    let missing = std::fs::read_to_string(dir.join("dist/404.html")).unwrap();
    assert!(missing.contains("src=\"/boot.js\""), "{missing}");
    assert!(!dir.join("dist/sitemap.xml").exists(), "no url, no sitemap");
}

#[test]
fn packages_resolve_from_dependencies() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("package-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let kit = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/kit");
    std::fs::write(
        dir.join("vcwl.toml"),
        format!(
            "entry = \"src/main.vcwl\"\nout = \"dist\"\n\n[dependencies]\nkit = \"{}\"\n",
            kit.display()
        ),
    )
    .unwrap();
    let build = |import: &str| {
        std::fs::write(
            dir.join("src/main.vcwl"),
            format!("app \"T\"\n\nimport \"{import}\"\n\ncomponent App {{\n    Card(title: \"x\") {{\n        text(\"y\")\n    }}\n}}\n"),
        )
        .unwrap();
        Command::new(env!("CARGO_BIN_EXE_vcwl"))
            .arg("build")
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let o = build("@nope");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("unknown package `@nope`") && err.contains("@kit"),
        "{err}"
    );
    let o = build("@kit/src/lib.vcwl");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = build("@kit");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        dir.join("dist/kit/README.txt").is_file(),
        "package assets are copied"
    );
    assert!(
        dir.join("dist-server/server.wasm").is_file(),
        "the package's server function"
    );
}
