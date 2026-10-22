//! Tests of the package manager against a registry in a temporary git repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("registry")).unwrap();
        let reg = dir.join("registry");
        for args in [
            &["init", "--quiet"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "start",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&reg)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        Env { dir }
    }

    /// Writes `files` (path, contents) under `rel`.
    fn write(&self, rel: &str, files: &[(&str, &str)]) {
        for (p, text) in files {
            let path = self.dir.join(rel).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn package(&self, name: &str, version: &str, deps: &str, lib: &str) {
        let manifest = format!(
            "name = \"{name}\"\nversion = \"{version}\"\nregistry = \"../registry\"\n\n[dependencies]\n{deps}"
        );
        self.write(
            &format!("{name}-{version}"),
            &[("vcwl.toml", &manifest), ("src/lib.vcwl", lib)],
        );
    }

    fn vcwl(&self, rel: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcwl"))
            .args(args)
            .current_dir(self.dir.join(rel))
            .env("VCWL_HOME", self.dir.join("home"))
            .env_remove("VCWL_REGISTRY")
            .output()
            .unwrap()
    }

    fn ok(&self, rel: &str, args: &[&str]) -> String {
        let o = self.vcwl(rel, args);
        let out = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(o.status.success(), "vcwl {args:?} in {rel}: {out}");
        out
    }

    fn fails(&self, rel: &str, args: &[&str]) -> String {
        let o = self.vcwl(rel, args);
        assert!(!o.status.success(), "vcwl {args:?} in {rel} should fail");
        String::from_utf8_lossy(&o.stderr).into_owned()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.join(rel)).unwrap_or_default()
    }
}

const APP: &str = "entry = \"src/main.vcwl\"\nregistry = \"../registry\"\n";

#[test]
fn publish_add_install_and_remove() {
    let e = Env::new("pkg-flow");
    e.package(
        "ints",
        "1.0.0",
        "",
        "export fn twice(n: Int) -> Int {\n    n * 2\n}\n",
    );
    e.package("ints", "1.1.0", "", "export fn twice(n: Int) -> Int {\n    n + n\n}\n\nexport fn thrice(n: Int) -> Int {\n    n * 3\n}\n");
    e.package(
        "more",
        "0.1.0",
        "ints = \"^1.1\"\n",
        "import \"@ints\"\n\nexport fn six(n: Int) -> Int {\n    twice(thrice(n))\n}\n",
    );
    assert!(
        e.ok("ints-1.0.0", &["publish"])
            .contains("published ints 1.0.0")
    );
    e.ok("ints-1.1.0", &["publish"]);
    // Publishing `more` installs its own dependency to check that it compiles.
    e.ok("more-0.1.0", &["publish"]);
    assert!(
        e.fails("ints-1.0.0", &["publish"])
            .contains("already published")
    );
    assert!(e.ok("more-0.1.0", &["search", "int"]).contains("ints"));

    e.write(
        "app",
        &[
            ("vcwl.toml", APP),
            (
                "src/main.vcwl",
                "import \"@more\"\n\nfn main() {\n    print(six(7).to_string())\n}\n",
            ),
        ],
    );
    let out = e.ok("app", &["add", "more"]);
    assert!(
        out.contains("installed ints 1.1.0") && out.contains("added more = \"^0.1.0\""),
        "{out}"
    );
    assert!(
        e.read("app/vcwl.toml")
            .contains("[dependencies]\nmore = \"^0.1.0\"")
    );
    let lock = e.read("app/vcwl.lock");
    assert!(
        lock.contains("ints = \"1.1.0\"\nmore = \"0.1.0\""),
        "{lock}"
    );
    // The transitive dependency resolves from the app's vcwl_packages.
    assert!(e.ok("app", &["check", "src/main.vcwl"]).contains("ok"));

    // A requirement the package's own requirement rules out.
    let err = e.fails("app", &["add", "ints@=1.0.0"]);
    assert!(err.contains("`ints`: more 0.1.0 needs a version"), "{err}");
    assert!(
        !e.read("app/vcwl.toml").contains("ints"),
        "a failed add leaves vcwl.toml as it was"
    );

    e.ok("app", &["remove", "more"]);
    assert!(!e.dir.join("app/vcwl_packages").exists());
    assert!(
        e.fails("app", &["remove", "more"])
            .contains("not a dependency")
    );
}

#[test]
fn builds_install_missing_packages_and_honor_the_lock() {
    let e = Env::new("pkg-lock");
    e.package("ints", "1.0.0", "", "export fn one() -> Int {\n    1\n}\n");
    e.ok("ints-1.0.0", &["publish"]);
    e.write(
        "app",
        &[
            (
                "vcwl.toml",
                &format!("{APP}\n[dependencies]\nints = \"^1.0\"\n"),
            ),
            (
                "src/main.vcwl",
                "import \"@ints\"\n\nfn main() {\n    print(one().to_string())\n}\n",
            ),
        ],
    );
    e.ok("app", &["install"]);
    // A newer version does not replace the locked one...
    e.package("ints", "1.2.0", "", "export fn one() -> Int {\n    1\n}\n");
    e.ok("ints-1.2.0", &["publish"]);
    e.ok("app", &["install"]);
    assert!(e.read("app/vcwl.lock").contains("ints = \"1.0.0\""));
    // ...and a fresh checkout without vcwl_packages gets it back on the next command.
    std::fs::remove_dir_all(e.dir.join("app/vcwl_packages")).unwrap();
    e.ok("app", &["check"]);
    assert_eq!(e.read("app/vcwl_packages/ints/.vcwl-version"), "1.0.0");
}

#[test]
fn path_dependencies_cannot_be_published() {
    let e = Env::new("pkg-path");
    e.package(
        "loose",
        "1.0.0",
        "kit = \"../kit\"\n",
        "export fn x() -> Int {\n    1\n}\n",
    );
    assert!(e.fails("loose-1.0.0", &["publish"]).contains("is a path"));
}

#[test]
fn the_security_review_blocks_dangerous_packages_until_trusted() {
    let e = Env::new("pkg-review");
    let spy = "export server fn leak() -> String {\n    env(\"DATABASE_URL\").unwrap_or(\"\")\n}\n";
    e.package("spy", "1.0.0", "", spy);
    e.write(
        "spy-1.0.0",
        &[("public/index2.html", "<script>alert(1)</script>")],
    );
    let out = e.ok("spy-1.0.0", &["publish"]);
    assert!(out.contains("security review"), "{out}");
    e.write(
        "app",
        &[("vcwl.toml", APP), ("src/main.vcwl", "fn main() {\n}\n")],
    );
    let err = e.fails("app", &["add", "spy"]);
    assert!(err.contains("refusing to install spy 1.0.0"), "{err}");
    assert!(err.contains("DANGER  reads your server's environment variables (where secrets live): \"DATABASE_URL\""), "{err}");
    assert!(
        err.contains("public/index2.html would be served from your site's origin"),
        "{err}"
    );
    assert!(err.contains("[trust]\nspy = \"1.0.0\""), "{err}");
    assert!(
        !e.dir.join("app/vcwl_packages/spy").exists()
            && !e.dir.join("app/vcwl_packages/.spy.review").exists()
    );
    assert!(!e.read("app/vcwl.toml").contains("spy"));
    // Trusting the exact version lets it in; the audit then passes.
    e.write(
        "app",
        &[(
            "vcwl.toml",
            &format!("{APP}\n[dependencies]\nspy = \"^1.0\"\n\n[trust]\nspy = \"1.0.0\"\n"),
        )],
    );
    assert!(e.ok("app", &["install"]).contains("DANGER"));
    assert!(e.ok("app", &["audit"]).contains("spy 1.0.0:"));
    // Withdrawing the trust makes the audit fail.
    e.write(
        "app",
        &[(
            "vcwl.toml",
            &format!("{APP}\n[dependencies]\nspy = \"^1.0\"\n"),
        )],
    );
    assert!(
        e.fails("app", &["audit"])
            .contains("untrusted packages with dangers: spy")
    );
}
