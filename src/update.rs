//! Update notices and `vcwl upgrade`. The latest release is looked up at most once a day, by a
//! detached `vcwl __update-check` process, so commands never wait on the network; the notice
//! shows on the next run. `VCWL_NO_UPDATE_CHECK` or `CI` turns it off.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const API: &str = "https://api.github.com/repos/chteau/vcwl/releases/latest";
const DAY: u64 = 24 * 60 * 60;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn cache_file() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("vcwl")
    } else if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(x).join("vcwl")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".cache/vcwl")
    };
    Some(dir.join("latest-version"))
}

/// `(checked at, latest version)` from the cache.
fn cached() -> Option<(u64, String)> {
    let text = std::fs::read_to_string(cache_file()?).ok()?;
    let (t, v) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
    Some((t.parse().ok()?, v.to_string()))
}

fn store(version: &str) {
    if let Some(f) = cache_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, format!("{} {version}", now()));
    }
}

/// `"0.2.0"` → `[0, 2, 0]`; a leading `v` and a pre-release suffix are ignored.
fn parse(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|n| n.parse().unwrap_or(0))
        .collect()
}

fn newer(latest: &str) -> bool {
    !latest.is_empty() && parse(latest) > parse(CURRENT)
}

/// The latest release's version, from GitHub.
fn fetch_latest() -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-m",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            API,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let body = String::from_utf8(out.stdout).ok()?;
    let rest = &body[body.find("\"tag_name\"")? + 10..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].trim_start_matches('v').to_string())
}

/// Called before every command: prints a notice when a newer release is known, and refreshes
/// the cache in the background once a day.
pub fn notify(cmd: &str) {
    if matches!(cmd, "lsp" | "upgrade" | "__update-check")
        || std::env::var_os("VCWL_NO_UPDATE_CHECK").is_some()
        || std::env::var_os("CI").is_some()
        || !std::io::stderr().is_terminal()
    {
        return;
    }
    let cache = cached();
    if let Some((_, latest)) = &cache
        && newer(latest)
    {
        eprintln!(
            "\x1b[33mvcwl {latest} is available (you have {CURRENT}): run `vcwl upgrade`\x1b[0m"
        );
    }
    if cache
        .as_ref()
        .is_none_or(|(t, _)| now().saturating_sub(*t) > DAY)
    {
        // Stamp the check first so a failing lookup is not retried on every command.
        store(cache.map(|c| c.1).as_deref().unwrap_or(""));
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = Command::new(exe);
            cmd.arg("__update-check")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // Its own process group, so closing the terminal does not interrupt the lookup.
            #[cfg(unix)]
            std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
            let _ = cmd.spawn();
        }
    }
}

/// `vcwl __update-check`: the background lookup.
pub fn refresh() -> Result<(), String> {
    if let Some(v) = fetch_latest() {
        store(&v);
    }
    Ok(())
}

/// xorshift64, seeded from the clock: good enough for homework.
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Rng(t ^ (std::process::id() as u64) << 32 | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
}

/// A random problem with an integer answer.
fn puzzle(r: &mut Rng) -> (String, i64) {
    match r.range(0, 6) {
        0 => {
            let (a, x, b) = (r.range(2, 9), r.range(2, 12), r.range(1, 30));
            (format!("Solve for x:  {a}x + {b} = {}", a * x + b), x)
        }
        1 => {
            let (a, b, k) = (r.range(1, 4), r.range(1, 5), r.range(1, 4));
            (
                format!("f(x) = {a}x³ + {b}x².  What is f'({k})?"),
                3 * a * k * k + 2 * b * k,
            )
        }
        2 => {
            let (a, k) = (r.range(1, 5), r.range(2, 6));
            (format!("∫₀^{k} {}x dx = ?", 2 * a), a * k * k)
        }
        3 => {
            let n = r.range(10, 60);
            (format!("1 + 2 + 3 + … + {n} = ?"), n * (n + 1) / 2)
        }
        4 => {
            let p = r.range(1, 12);
            let q = (p + r.range(1, 8)).min(15);
            (
                format!(
                    "What is the larger root of  x² − {}x + {} = 0 ?",
                    p + q,
                    p * q
                ),
                q,
            )
        }
        5 => {
            let (a, b, m) = (r.range(2, 9), r.range(2, 6), r.range(5, 13));
            (format!("{a}^{b} mod {m} = ?"), a.pow(b as u32) % m)
        }
        _ => {
            let (n, k) = (r.range(5, 10), r.range(2, 4));
            let c = (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1));
            (
                format!("How many ways to choose {k} items out of {n}?  C({n}, {k}) = ?"),
                c,
            )
        }
    }
}

/// The release archive for this platform, as published by the release workflow.
fn asset() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "vcwl-x86_64-unknown-linux-gnu.tar.gz",
        ("macos", "aarch64") => "vcwl-aarch64-apple-darwin.tar.gz",
        ("macos", "x86_64") => "vcwl-x86_64-apple-darwin.tar.gz",
        ("windows", "x86_64") => "vcwl-x86_64-pc-windows-msvc.zip",
        _ => return None,
    })
}

/// Downloads the latest release and puts it in place of the running executable.
fn install() -> Result<(), String> {
    let asset = asset().ok_or(
        "no prebuilt vcwl for this platform; build it from https://github.com/chteau/vcwl",
    )?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate vcwl: {e}"))?;
    let dir = exe.parent().ok_or("cannot locate vcwl's directory")?;
    let tmp = dir.join(".vcwl-upgrade");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    let archive = tmp.join(asset);
    let url = format!("https://github.com/chteau/vcwl/releases/latest/download/{asset}");
    eprintln!("downloading {url}");
    let run = |cmd: &mut Command| {
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        if ok {
            Ok(())
        } else {
            Err(format!("`{cmd:?}` failed"))
        }
    };
    let r = (|| {
        run(Command::new("curl")
            .args(["-fL", "--progress-bar", "-o"])
            .arg(&archive)
            .arg(&url))?;
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&tmp))?;
        let new = tmp.join(if cfg!(windows) { "vcwl.exe" } else { "vcwl" });
        if cfg!(windows) {
            // A running .exe cannot be replaced, but it can be renamed out of the way.
            let old = exe.with_extension("old.exe");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
            }
        }
        std::fs::rename(&new, &exe).map_err(|e| format!("cannot replace {}: {e}", exe.display()))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// `vcwl upgrade`: installs the latest release, for anyone who can do a little math.
pub fn upgrade(_args: &[String]) -> Result<(), String> {
    eprintln!("checking for a new version...");
    let latest = fetch_latest().ok_or("cannot reach GitHub to check for a new version")?;
    store(&latest);
    if !newer(&latest) {
        println!("vcwl {CURRENT} is the latest version");
        return Ok(());
    }
    println!("vcwl {latest} is out (you have {CURRENT}).");
    println!("Upgrades are free. Answer this first:\n");
    let mut rng = Rng::new();
    let (question, answer) = puzzle(&mut rng);
    println!("    {question}\n");
    let stdin = std::io::stdin();
    for attempt in 1..=3 {
        print!("answer> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("no answer, no upgrade".into());
        }
        match line.trim().parse::<i64>() {
            Ok(n) if n == answer => {
                println!("Correct. Upgrading.");
                install()?;
                println!("upgraded vcwl {CURRENT} → {latest}");
                return Ok(());
            }
            _ if attempt < 3 => println!(
                "Not quite. {} left.",
                if attempt == 1 { "2 tries" } else { "1 try" }
            ),
            _ => {}
        }
    }
    Err(format!(
        "The answer was {answer}. vcwl {CURRENT} keeps working; try `vcwl upgrade` again when you're ready."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(parse("v0.10.0") > parse("0.9.3"));
        assert!(parse("1.0.0-beta") == parse("1.0.0"));
        assert!(!newer(""));
        assert!(!newer(CURRENT));
    }

    #[test]
    fn puzzles_cover_every_kind_with_positive_answers() {
        let mut r = Rng(42);
        let mut kinds = std::collections::HashSet::new();
        for _ in 0..500 {
            let (q, a) = puzzle(&mut r);
            assert!(a >= 0, "{q} = {a}");
            kinds.insert(q.chars().take(4).collect::<String>());
        }
        assert!(kinds.len() >= 7);
    }
}
