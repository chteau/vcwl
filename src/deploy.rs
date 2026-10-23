//! `vcwl deploy`: builds the project and ships `dist/` with a great deal of ceremony.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::{self, Project};
use crate::sha256;

const USAGE: &str = "usage: vcwl deploy <netlify|pages|folder DIR> [--yes]";

enum Target {
    Netlify,
    Pages,
    Folder(PathBuf),
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn json_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", dir.display());
    for e in std::fs::read_dir(dir).map_err(err)? {
        let p = e.map_err(err)?.path();
        if p.is_dir() {
            files(&p, out)?;
        } else {
            out.push(p);
        }
    }
    out.sort();
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let mut list = Vec::new();
    files(from, &mut list)?;
    for f in list {
        let dest = to.join(f.strip_prefix(from).unwrap());
        std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::copy(&f, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    Ok(())
}

fn step(n: usize, text: &str) {
    println!("[{n}] {text}");
}

fn exec(dry: bool, dir: Option<&Path>, program: &str, args: &[&str]) -> Result<(), String> {
    let shown = format!("{program} {}", args.join(" "));
    if dry {
        println!("    (dry run) {shown}");
        return Ok(());
    }
    let mut c = Command::new(program);
    c.args(args);
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let st = c.status().map_err(|e| format!("cannot run `{shown}`: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("`{shown}` failed"))
    }
}

fn push_pages(root: &Path, out: &Path) -> Result<(), String> {
    let remote = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or("no git remote `origin` to push gh-pages to")?;
    let scratch = root.join("target").join("vcwl-pages");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    copy_tree(out, &scratch)?;
    let d = Some(scratch.as_path());
    exec(false, d, "git", &["init", "-q", "-b", "gh-pages"])?;
    exec(false, d, "git", &["add", "-A"])?;
    exec(
        false,
        d,
        "git",
        &[
            "-c",
            "user.name=vcwl",
            "-c",
            "user.email=vcwl@localhost",
            "commit",
            "-q",
            "-m",
            "deploy",
        ],
    )?;
    exec(false, d, "git", &["push", "--force", &remote, "gh-pages"])
}

pub fn run(rest: &[String]) -> Result<(), String> {
    let yes = rest.iter().any(|a| a == "--yes");
    let pos: Vec<&String> = rest.iter().filter(|a| *a != "--yes").collect();
    let target = match pos.first().map(|s| s.as_str()) {
        Some("netlify") => Target::Netlify,
        Some("pages") => Target::Pages,
        Some("folder") => Target::Folder(PathBuf::from(pos.get(1).ok_or(USAGE)?)),
        _ => return Err(USAGE.to_string()),
    };
    let dry = std::env::var_os("VCWL_DEPLOY_DRY_RUN").is_some_and(|v| v == "1");
    let p = Project::find(Path::new("."))?;
    let name = project::manifest_value(&p.manifest, "name").unwrap_or_else(|| {
        p.root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app")
            .to_string()
    });

    println!("=== VCWL Deployment Ceremony: {name} ===");
    step(1, "Pre-flight checklist");
    println!("    [x] project found at {}", p.root.display());
    println!("    [x] entry {} exists", p.entry.display());
    println!("    [x] caffeine levels acceptable");
    if !yes {
        print!("    type the project name ({name}) to confirm: ");
        std::io::stdout().flush().ok();
        let mut s = String::new();
        std::io::stdin()
            .read_line(&mut s)
            .map_err(|e| e.to_string())?;
        if s.trim() != name {
            return Err("confirmation failed; nothing was deployed".to_string());
        }
    }

    step(2, "Building the release");
    let sizes = project::build(&p.entry, &p.out, false)?;
    println!("    {} files compiled", sizes.len());

    step(3, "Blessing the bytes");
    println!("    ~ the bytes have been blessed ~");

    step(4, "Hashing every file into the manifest");
    let manifest_path = p.out.join(".vcwl-manifest.json");
    let _ = std::fs::remove_file(&manifest_path);
    let mut list = Vec::new();
    files(&p.out, &mut list)?;
    let mut entries = Vec::new();
    for f in &list {
        let data = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let rel = f
            .strip_prefix(&p.out)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let h = hex(&sha256::digest(&data));
        println!("    {h}  {rel}");
        entries.push(format!(
            "    {{\"path\": \"{}\", \"sha256\": \"{h}\", \"bytes\": {}}}",
            json_str(&rel),
            data.len()
        ));
    }
    let json = format!(
        "{{\n  \"project\": \"{}\",\n  \"files\": [\n{}\n  ]\n}}\n",
        json_str(&name),
        entries.join(",\n")
    );
    std::fs::write(&manifest_path, json)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    step(5, "Launching");
    let out = p.out.to_string_lossy().to_string();
    match target {
        Target::Netlify => exec(
            dry,
            None,
            "npx",
            &["--yes", "netlify-cli", "deploy", "--prod", "--dir", &out],
        )?,
        Target::Pages => {
            if dry {
                println!("    (dry run) push {out} to branch gh-pages");
            } else {
                push_pages(&p.root, &p.out)?;
            }
        }
        Target::Folder(dir) => {
            if dry {
                println!("    (dry run) copy {out} to {}", dir.display());
            } else {
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                copy_tree(&p.out, &dir)?;
            }
        }
    }
    println!("=== Deployment complete. The ceremony is concluded. ===");
    Ok(())
}
