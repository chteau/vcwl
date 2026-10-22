//! The package manager. A registry is the `registry` branch of a git repository: package
//! `name` version `v` is the directory `packages/<name>/<v>/` of that branch (the package's
//! files, with its vcwl.toml). Registry dependencies (`name = "^1.2"`) are resolved to one
//! version per name, written to `vcwl.lock` and copied into `vcwl_packages/<name>/`.
//! Everything goes through the `git` command; fetched branches are cached in a bare repository
//! under `$VCWL_HOME` (default `~/.cache/vcwl`).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::project::{manifest_table, manifest_value};

pub const PACKAGES_DIR: &str = "vcwl_packages";
const BRANCH: &str = "registry";

// ----- versions -----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let p: Vec<u64> = s
            .split('.')
            .map(|x| x.parse().ok())
            .collect::<Option<_>>()?;
        match p[..] {
            [a, b, c] => Some(Version(a, b, c)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A version requirement: `*`, `=1.2.3`, `~1.2.3` (same minor), `^1.2.3` or `1.2.3` (same
/// first non-zero part); missing parts count as 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Req {
    low: Version,
    high: Option<Version>,
    exact: bool,
}

impl Req {
    pub fn parse(s: &str) -> Option<Req> {
        let s = s.trim();
        if s == "*" {
            return Some(Req {
                low: Version(0, 0, 0),
                high: None,
                exact: false,
            });
        }
        let (op, rest) = match s.chars().next()? {
            c @ ('^' | '~' | '=') => (c, &s[1..]),
            c if c.is_ascii_digit() => ('^', s),
            _ => return None,
        };
        let parts: Vec<u64> = rest
            .split('.')
            .map(|x| x.parse().ok())
            .collect::<Option<_>>()?;
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        let get = |i: usize| parts.get(i).copied().unwrap_or(0);
        let low = Version(get(0), get(1), get(2));
        let high = match op {
            '=' => {
                return (parts.len() == 3).then_some(Req {
                    low,
                    high: None,
                    exact: true,
                });
            }
            '~' if parts.len() == 1 => Version(low.0 + 1, 0, 0),
            '~' => Version(low.0, low.1 + 1, 0),
            _ if low.0 > 0 || parts.len() == 1 => Version(low.0 + 1, 0, 0),
            _ if low.1 > 0 || parts.len() == 2 => Version(0, low.1 + 1, 0),
            _ => Version(0, 0, low.2 + 1),
        };
        Some(Req {
            low,
            high: Some(high),
            exact: false,
        })
    }

    pub fn matches(&self, v: Version) -> bool {
        if self.exact {
            return v == self.low;
        }
        v >= self.low && self.high.is_none_or(|h| v < h)
    }
}

/// Whether a dependency value names a registry version (otherwise it is a path).
pub fn is_registry_dep(value: &str) -> bool {
    Req::parse(value).is_some()
}

/// Where dependency `name = value` of the project at `root` lives: a path relative to it, or
/// for a registry dependency the `vcwl_packages/<name>` of the nearest project that installed
/// it (packages are installed flat into the top project's `vcwl_packages`).
pub fn dep_dir(root: &Path, name: &str, value: &str) -> PathBuf {
    if !is_registry_dep(value) {
        return root.join(value);
    }
    let mut d = Some(root);
    while let Some(cur) = d {
        let p = cur.join(PACKAGES_DIR).join(name);
        if p.is_dir() {
            return p;
        }
        d = cur.parent();
    }
    root.join(PACKAGES_DIR).join(name)
}

// ----- git -----

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_env(dir, args, &[], None)
}

fn git_env(
    dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    input: Option<&[u8]>,
) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).envs(env.iter().copied());
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    if let Some(data) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(data)
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A registry: the URL it was given as, and the local bare repository caching its branch.
pub struct Registry {
    pub url: String,
    cache: PathBuf,
}

fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("VCWL_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("vcwl")
}

impl Registry {
    /// The registry of the project at `root`: `$VCWL_REGISTRY`, or its manifest's `registry`
    /// (a git URL, or a path relative to the manifest).
    pub fn for_project(root: &Path, manifest: &str) -> Result<Registry, String> {
        let url = std::env::var("VCWL_REGISTRY")
            .ok()
            .or_else(|| manifest_value(manifest, "registry"))
            .ok_or("no registry: set `registry = \"<git url or path>\"` in vcwl.toml (or $VCWL_REGISTRY)")?;
        let url = if url.contains("://") || url.contains('@') || Path::new(&url).is_absolute() {
            url
        } else {
            let p = root.join(&url);
            p.canonicalize().unwrap_or(p).to_string_lossy().into_owned()
        };
        // FNV-1a of the URL names its cache.
        let hash = url.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        let cache = home().join(format!("registry-{hash:016x}.git"));
        if !cache.join("HEAD").exists() {
            std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
            git(&cache, &["init", "--quiet", "--bare"])?;
        }
        Ok(Registry { url, cache })
    }

    /// Fetches the registry branch; returns whether it exists. Without network, a branch
    /// fetched before is used (with a warning).
    pub fn fetch(&self) -> Result<bool, String> {
        let spec = format!("+refs/heads/{BRANCH}:refs/heads/{BRANCH}");
        match git(&self.cache, &["fetch", "--quiet", &self.url, &spec]) {
            Ok(_) => Ok(true),
            Err(e) if e.contains("couldn't find remote ref") => Ok(false),
            Err(e) if self.head().is_some() => {
                eprintln!("warning: using the cached registry ({e})");
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    fn head(&self) -> Option<String> {
        git(
            &self.cache,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{BRANCH}"),
            ],
        )
        .ok()
        .map(|s| s.trim().to_string())
    }

    fn list(&self, path: &str) -> Vec<String> {
        git(&self.cache, &["ls-tree", "--name-only", BRANCH, path])
            .map(|s| {
                s.lines()
                    .map(|l| l.rsplit('/').next().unwrap_or(l).to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn names(&self) -> Vec<String> {
        self.list("packages/")
    }

    /// Published versions of `name`, newest first.
    pub fn versions(&self, name: &str) -> Vec<Version> {
        let mut v: Vec<Version> = self
            .list(&format!("packages/{name}/"))
            .iter()
            .filter_map(|s| Version::parse(s))
            .collect();
        v.sort_by(|a, b| b.cmp(a));
        v
    }

    pub fn manifest(&self, name: &str, v: Version) -> Result<String, String> {
        git(
            &self.cache,
            &["show", &format!("{BRANCH}:packages/{name}/{v}/vcwl.toml")],
        )
    }

    /// Writes the files of `name` `v` into `dest` (emptied first).
    fn extract(&self, name: &str, v: Version, dest: &Path) -> Result<(), String> {
        let prefix = format!("packages/{name}/{v}/");
        let listing = git(&self.cache, &["ls-tree", "-r", "-z", BRANCH, &prefix])?;
        let files: Vec<(String, String)> = listing
            .split('\0')
            .filter_map(|e| {
                let (meta, path) = e.split_once('\t')?;
                Some((
                    meta.split(' ').nth(2)?.to_string(),
                    path.strip_prefix(&prefix)?.to_string(),
                ))
            })
            .collect();
        // One `git cat-file --batch` for all blobs: "<sha> blob <size>\n<bytes>\n" each.
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.cache)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input: String = files.iter().map(|(sha, _)| format!("{sha}\n")).collect();
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let mut data = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        writer
            .join()
            .map_err(|_| "git cat-file failed")?
            .map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(dest);
        let mut at = 0;
        for (_, rel) in &files {
            let nl = data[at..]
                .iter()
                .position(|&b| b == b'\n')
                .ok_or("truncated git output")?
                + at;
            let size: usize = String::from_utf8_lossy(&data[at..nl])
                .rsplit(' ')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or("bad git output")?;
            let body = &data[nl + 1..nl + 1 + size];
            at = nl + 2 + size;
            // A registry branch is not trusted to keep files inside the package.
            if rel.split('/').any(|p| {
                p.is_empty() || p == ".." || p == "." || p.contains('\\') || p.contains(':')
            }) {
                return Err(format!("{name} {v}: refusing the file path `{rel}`"));
            }
            let path = dest.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    }
}

// ----- install -----

/// Registry dependencies of a manifest: name and requirement.
fn registry_deps(manifest: &str) -> Result<Vec<(String, Req)>, String> {
    Ok(manifest_table(manifest, "dependencies")
        .into_iter()
        .filter_map(|(k, v)| Req::parse(&v).map(|r| (k, r)))
        .collect())
}

/// The versions in `vcwl.lock`.
fn read_lock(root: &Path) -> BTreeMap<String, Version> {
    let text = std::fs::read_to_string(root.join("vcwl.lock")).unwrap_or_default();
    manifest_table(&text, "packages")
        .into_iter()
        .filter_map(|(k, v)| Some((k, Version::parse(&v)?)))
        .collect()
}

/// Picks one version of every registry package the project needs, preferring locked ones.
fn resolve(
    reg: &Registry,
    deps: Vec<(String, Req)>,
    lock: &BTreeMap<String, Version>,
) -> Result<BTreeMap<String, Version>, String> {
    let mut chosen: BTreeMap<String, (Version, String)> = BTreeMap::new();
    let mut queue: Vec<(String, Req, String)> = deps
        .into_iter()
        .map(|(n, r)| (n, r, "the project".to_string()))
        .collect();
    while let Some((name, req, by)) = queue.pop() {
        if let Some((v, first)) = chosen.get(&name) {
            if !req.matches(*v) {
                return Err(format!(
                    "`{name}`: {by} needs a version that {first} does not allow ({v} was picked for {first})"
                ));
            }
            continue;
        }
        let versions = reg.versions(&name);
        if versions.is_empty() {
            return Err(format!(
                "no package `{name}` in the registry {} (needed by {by})",
                reg.url
            ));
        }
        let v = lock
            .get(&name)
            .copied()
            .filter(|v| req.matches(*v) && versions.contains(v))
            .or_else(|| versions.iter().copied().find(|v| req.matches(*v)))
            .ok_or_else(|| {
                let list: Vec<String> = versions.iter().map(|v| v.to_string()).collect();
                format!(
                    "no version of `{name}` matches {by}'s requirement (published: {})",
                    list.join(", ")
                )
            })?;
        let manifest = reg.manifest(&name, v)?;
        for (k, value) in manifest_table(&manifest, "dependencies") {
            if !is_registry_dep(&value) {
                return Err(format!(
                    "{name} {v} depends on `{k}` by path, which a registry package cannot"
                ));
            }
        }
        for (dep, r) in registry_deps(&manifest)? {
            queue.push((dep, r, format!("{name} {v}")));
        }
        chosen.insert(name.clone(), (v, by));
    }
    Ok(chosen.into_iter().map(|(k, (v, _))| (k, v)).collect())
}

/// Installs the registry dependencies of the project at `root`. Returns the installed versions.
pub fn install(root: &Path, quiet: bool) -> Result<BTreeMap<String, Version>, String> {
    let manifest =
        std::fs::read_to_string(root.join("vcwl.toml")).map_err(|e| format!("vcwl.toml: {e}"))?;
    let deps = registry_deps(&manifest)?;
    let dir = root.join(PACKAGES_DIR);
    if deps.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(root.join("vcwl.lock"));
        return Ok(BTreeMap::new());
    }
    let reg = Registry::for_project(root, &manifest)?;
    if !reg.fetch()? {
        return Err(format!(
            "{} has no `{BRANCH}` branch (publish a package first)",
            reg.url
        ));
    }
    let trusted: BTreeMap<String, String> =
        manifest_table(&manifest, "trust").into_iter().collect();
    let picked = resolve(&reg, deps, &read_lock(root))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, v) in &picked {
        let dest = dir.join(name);
        let stamp = dest.join(".vcwl-version");
        if std::fs::read_to_string(&stamp).ok().as_deref() == Some(&v.to_string()) {
            continue;
        }
        // Unpacked beside the others and reviewed before it takes its place.
        let staging = dir.join(format!(".{name}.review"));
        reg.extract(name, *v, &staging)?;
        let findings = crate::review::review(name, &staging);
        let dangers = findings
            .iter()
            .any(|f| f.level == crate::review::Level::Danger);
        if dangers && trusted.get(name) != Some(&v.to_string()) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!(
                "refusing to install {name} {v}: the security review found\n{}\nRead the package (`git show {BRANCH}:packages/{name}/{v}/` in the registry). To install it anyway,\nadd to vcwl.toml:\n\n[trust]\n{name} = \"{v}\"\n",
                crate::review::report(name, &v.to_string(), &findings).trim_end()
            ));
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&staging, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        std::fs::write(&stamp, v.to_string()).map_err(|e| e.to_string())?;
        if !quiet {
            println!("  installed {name} {v}");
            if !findings.is_empty() {
                print!(
                    "{}",
                    crate::review::report(name, &v.to_string(), &findings).replacen(
                        &format!("  {name} {v}:\n"),
                        "",
                        1
                    )
                );
            }
        }
    }
    // Packages no longer needed.
    for e in std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let n = e.file_name().to_string_lossy().into_owned();
        if !picked.contains_key(&n) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let mut lock = format!(
        "# Written by `vcwl install`; commit it so everyone gets the same versions.\nregistry = \"{}\"\ncommit = \"{}\"\n\n[packages]\n",
        manifest_value(&manifest, "registry").unwrap_or_else(|| reg.url.clone()),
        reg.head().unwrap_or_default()
    );
    for (name, v) in &picked {
        lock.push_str(&format!("{name} = \"{v}\"\n"));
    }
    std::fs::write(root.join("vcwl.lock"), lock).map_err(|e| e.to_string())?;
    Ok(picked)
}

/// Installs registry dependencies that are missing from `vcwl_packages` (before a build).
pub fn ensure(root: &Path, manifest: &str) -> Result<(), String> {
    let deps = registry_deps(manifest)?;
    let lock = read_lock(root);
    let missing = deps.iter().any(|(name, req)| {
        let stamp =
            std::fs::read_to_string(root.join(PACKAGES_DIR).join(name).join(".vcwl-version")).ok();
        !stamp
            .and_then(|s| Version::parse(&s))
            .is_some_and(|v| req.matches(v) && lock.get(name) == Some(&v))
    });
    if missing {
        install(root, false)?;
    }
    Ok(())
}

// ----- manifest editing -----

/// `text` with `name = "value"` set in `[dependencies]` (or removed for `None`).
pub fn set_dependency(text: &str, name: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let (mut section, mut done, mut deps_end) = (String::new(), false, None);
    for l in text.lines() {
        let t = l.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.trim().to_string();
        } else if section == "dependencies"
            && t.split('=').next().map(str::trim) == Some(name)
            && t.contains('=')
        {
            if let Some(v) = value {
                out.push(format!("{name} = \"{v}\""));
            }
            done = true;
            deps_end = Some(out.len());
            continue;
        }
        out.push(l.to_string());
        if section == "dependencies" && !t.is_empty() {
            deps_end = Some(out.len());
        }
    }
    if let (Some(v), false) = (value, done) {
        let line = format!("{name} = \"{v}\"");
        match deps_end {
            Some(i) => out.insert(i, line),
            None => {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push("[dependencies]".into());
                out.push(line);
            }
        }
    }
    out.join("\n") + "\n"
}

// ----- publishing -----

/// Files of the package at `root`, relative to it (build output, installed packages and
/// hidden files left out).
fn package_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(root.join(&rel))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || (rel.as_os_str().is_empty()
                    && matches!(
                        name.as_str(),
                        "dist" | "dist-server" | PACKAGES_DIR | "target" | "vcwl.lock"
                    ))
            {
                continue;
            }
            let r = rel.join(&name);
            if e.file_type().map_err(|e| e.to_string())?.is_dir() {
                stack.push(r);
            } else {
                out.push(r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Adds the package at `root` to its registry's branch as a new commit and pushes it.
pub fn publish(root: &Path, dry_run: bool) -> Result<String, String> {
    let manifest =
        std::fs::read_to_string(root.join("vcwl.toml")).map_err(|e| format!("vcwl.toml: {e}"))?;
    let name =
        manifest_value(&manifest, "name").ok_or("vcwl.toml needs `name = \"...\"` to publish")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "package name `{name}`: use lowercase letters, digits and `-`"
        ));
    }
    let version = manifest_value(&manifest, "version")
        .and_then(|v| Version::parse(&v))
        .ok_or("vcwl.toml needs `version = \"x.y.z\"` to publish")?;
    for (k, v) in manifest_table(&manifest, "dependencies") {
        if !is_registry_dep(&v) {
            return Err(format!(
                "dependency `{k}` is a path; published packages can only depend on registry packages"
            ));
        }
    }
    let lib = manifest_value(&manifest, "lib").unwrap_or_else(|| "src/lib.vcwl".into());
    if !root.join(&lib).is_file() {
        return Err(format!("{lib}: not found (set `lib` in vcwl.toml)"));
    }
    ensure(root, &manifest)?;
    crate::project::compile_entry(&root.join(&lib), true)
        .map_err(|e| format!("the package does not compile:\n{e}"))?;
    let reg = Registry::for_project(root, &manifest)?;
    let exists = reg.fetch()?;
    if exists && reg.versions(&name).contains(&version) {
        return Err(format!(
            "{name} {version} is already published; versions cannot change (bump `version`)"
        ));
    }
    let files = package_files(root)?;
    if dry_run {
        return Ok(format!(
            "would publish {name} {version} to {} ({} files)",
            reg.url,
            files.len()
        ));
    }
    let paths: String = files
        .iter()
        .map(|f| format!("{}\n", root.join(f).display()))
        .collect();
    let shas = git_env(
        &reg.cache,
        &["hash-object", "-w", "--stdin-paths"],
        &[],
        Some(paths.as_bytes()),
    )?;
    let index = reg
        .cache
        .join(format!("publish-{}.index", std::process::id()));
    let index_s = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_s.as_str())];
    let result = (|| {
        if exists {
            git_env(&reg.cache, &["read-tree", BRANCH], &env, None)?;
        } else {
            git_env(&reg.cache, &["read-tree", "--empty"], &env, None)?;
        }
        let info: String = shas
            .lines()
            .zip(&files)
            .map(|(sha, f)| format!("100644 {sha}\tpackages/{name}/{version}/{f}\n"))
            .collect();
        git_env(
            &reg.cache,
            &["update-index", "--add", "--index-info"],
            &env,
            Some(info.as_bytes()),
        )?;
        let tree = git_env(&reg.cache, &["write-tree"], &env, None)?;
        let msg = format!("publish {name} {version}");
        let mut args = vec!["commit-tree", tree.trim(), "-m", &msg];
        // `commit-tree` ignores commit.gpgSign: sign when the user signs their commits.
        if git(&reg.cache, &["config", "--bool", "commit.gpgsign"])
            .is_ok_and(|v| v.trim() == "true")
        {
            args.push("-S");
        }
        let parent = reg.head();
        if exists && let Some(p) = &parent {
            args.extend(["-p", p.as_str()]);
        }
        // Without a configured git identity, commits are by "vcwl".
        let named = git(&reg.cache, &["config", "user.name"]).is_ok_and(|s| !s.trim().is_empty());
        let ident: &[(&str, &str)] = if named {
            &[]
        } else {
            &[
                ("GIT_AUTHOR_NAME", "vcwl"),
                ("GIT_AUTHOR_EMAIL", "vcwl@localhost"),
                ("GIT_COMMITTER_NAME", "vcwl"),
                ("GIT_COMMITTER_EMAIL", "vcwl@localhost"),
            ]
        };
        let commit = git_env(&reg.cache, &args, ident, None)?;
        let commit = commit.trim();
        git(
            &reg.cache,
            &[
                "push",
                "--quiet",
                &reg.url,
                &format!("{commit}:refs/heads/{BRANCH}"),
            ],
        )?;
        git(
            &reg.cache,
            &["update-ref", &format!("refs/heads/{BRANCH}"), commit],
        )?;
        Ok::<_, String>(())
    })();
    let _ = std::fs::remove_file(&index);
    result?;
    // What installers' reviews will say about it.
    let findings = crate::review::review(&name, root);
    Ok(format!(
        "{}published {name} {version} to {} ({} files)",
        if findings.is_empty() {
            String::new()
        } else {
            format!(
                "security review (shown to everyone who installs it):\n{}",
                crate::review::report(&name, &version.to_string(), &findings)
            )
        },
        manifest_value(&manifest, "registry").unwrap_or_else(|| reg.url.clone()),
        files.len()
    ))
}

// ----- commands -----

fn project_root() -> Result<(PathBuf, String), String> {
    let p = crate::project::Project::find_only(Path::new("."))?;
    Ok((p.root, p.manifest))
}

/// `vcwl install`, `add`, `remove`, `publish` and `search`.
pub fn run(cmd: &str, rest: &[String]) -> Result<(), String> {
    match cmd {
        "install" => {
            let (root, _) = project_root()?;
            let picked = install(&root, false)?;
            println!(
                "{} registry package{} ready",
                picked.len(),
                if picked.len() == 1 { "" } else { "s" }
            );
            Ok(())
        }
        "add" => {
            let spec = rest.first().ok_or("usage: vcwl add <name>[@requirement]")?;
            let (root, manifest) = project_root()?;
            let (name, req) = match spec.split_once('@') {
                Some((n, r)) => (n.to_string(), Some(r.to_string())),
                None => (spec.clone(), None),
            };
            let req = match req {
                Some(r) => {
                    Req::parse(&r).ok_or_else(|| {
                        format!("`{r}` is not a version requirement (like ^1.2 or =1.0.0)")
                    })?;
                    r
                }
                None => {
                    let reg = Registry::for_project(&root, &manifest)?;
                    reg.fetch()?;
                    let v = *reg
                        .versions(&name)
                        .first()
                        .ok_or_else(|| format!("no package `{name}` in {}", reg.url))?;
                    format!("^{v}")
                }
            };
            let path = root.join("vcwl.toml");
            let before = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            std::fs::write(&path, set_dependency(&before, &name, Some(&req)))
                .map_err(|e| e.to_string())?;
            if let Err(e) = install(&root, false) {
                std::fs::write(&path, before).map_err(|e| e.to_string())?;
                return Err(e);
            }
            println!("added {name} = \"{req}\"");
            Ok(())
        }
        "remove" => {
            let name = rest.first().ok_or("usage: vcwl remove <name>")?;
            let (root, manifest) = project_root()?;
            if !manifest_table(&manifest, "dependencies")
                .iter()
                .any(|(k, _)| k == name)
            {
                return Err(format!("`{name}` is not a dependency"));
            }
            std::fs::write(
                root.join("vcwl.toml"),
                set_dependency(&manifest, name, None),
            )
            .map_err(|e| e.to_string())?;
            install(&root, true)?;
            println!("removed {name}");
            Ok(())
        }
        "publish" => {
            let (root, _) = project_root()?;
            println!("{}", publish(&root, rest.iter().any(|a| a == "--dry-run"))?);
            Ok(())
        }
        "audit" => {
            let (root, manifest) = project_root()?;
            let trusted: BTreeMap<String, String> =
                manifest_table(&manifest, "trust").into_iter().collect();
            let mut blocked = Vec::new();
            for (name, v) in read_lock(&root) {
                let findings = crate::review::review(&name, &root.join(PACKAGES_DIR).join(&name));
                print!(
                    "{}",
                    crate::review::report(&name, &v.to_string(), &findings)
                );
                if findings
                    .iter()
                    .any(|f| f.level == crate::review::Level::Danger)
                    && trusted.get(&name) != Some(&v.to_string())
                {
                    blocked.push(name);
                }
            }
            if blocked.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "untrusted packages with dangers: {}",
                    blocked.join(", ")
                ))
            }
        }
        "search" => {
            let (root, manifest) = project_root()?;
            let reg = Registry::for_project(&root, &manifest)?;
            if !reg.fetch()? {
                println!("the registry is empty");
                return Ok(());
            }
            let word = rest.first().map(|s| s.to_lowercase()).unwrap_or_default();
            for name in reg.names() {
                let Some(v) = reg.versions(&name).first().copied() else {
                    continue;
                };
                let desc = reg
                    .manifest(&name, v)
                    .ok()
                    .and_then(|m| manifest_value(&m, "description"))
                    .unwrap_or_default();
                if name.contains(&word) || desc.to_lowercase().contains(&word) {
                    println!("{name:<16} {v:<8} {desc}");
                }
            }
            Ok(())
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements() {
        let m = |r: &str, v: &str| Req::parse(r).unwrap().matches(Version::parse(v).unwrap());
        assert!(m("1.2", "1.9.0") && !m("1.2", "2.0.0") && !m("1.2", "1.1.9"));
        assert!(m("^0.2.3", "0.2.9") && !m("^0.2.3", "0.3.0"));
        assert!(m("^0.0.3", "0.0.3") && !m("^0.0.3", "0.0.4"));
        assert!(m("~1.2.3", "1.2.9") && !m("~1.2.3", "1.3.0"));
        assert!(m("=1.0.0", "1.0.0") && !m("=1.0.0", "1.0.1"));
        assert!(m("*", "7.0.0"));
        assert!(Req::parse("../kit").is_none() && Req::parse("=1.0").is_none());
    }

    #[test]
    fn editing_dependencies() {
        let t =
            "entry = \"src/main.vcwl\"\n\n[dependencies]\nkit = \"../kit\"\n\n[other]\nx = \"1\"\n";
        let added = set_dependency(t, "is-odd", Some("^1.0.0"));
        assert_eq!(
            added,
            "entry = \"src/main.vcwl\"\n\n[dependencies]\nkit = \"../kit\"\nis-odd = \"^1.0.0\"\n\n[other]\nx = \"1\"\n"
        );
        assert_eq!(set_dependency(&added, "is-odd", None), t);
        assert_eq!(
            set_dependency("entry = \"a\"\n", "x", Some("1")),
            "entry = \"a\"\n\n[dependencies]\nx = \"1\"\n"
        );
    }
}
