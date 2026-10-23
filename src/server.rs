//! `vcwl serve`: serves the built site and runs its server functions (`server fn`).
//!
//! The compiler puts server functions in their own module (`dist-server/server.wasm`), each
//! exported as `__fn_<name>`. A call is `POST /__vcwl/fn/<name>` with the arguments as a JSON
//! array; the response is the JSON of the result. Each call runs in a fresh instance (no state
//! survives between calls except the store, the SQLite database and the session cookie), calls run
//! in parallel, each with a time and memory limit.
//! The host functions mirror the browser host's (`runtime/vcwl.js`): text travels through the
//! I/O region of linear memory, JSON values through integer handles.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wasmtime::{
    Caller, Config, Engine, InstancePre, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

use crate::project::{self, Project};

/// Start of the I/O region in linear memory (as in the browser host).
const IO_BASE: usize = 65536;
/// Largest accepted request body.
const MAX_BODY: usize = 1 << 20;
/// Epoch ticks (of `TICK`) a call may run before it is stopped.
const CALL_TICKS: u64 = 50;
const TICK: Duration = Duration::from_millis(100);

// ----- JSON -----

#[derive(Debug, PartialEq)]
pub enum Json {
    /// A field or index that does not exist (`json_kind` -1).
    Missing,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Rc<Json>>),
    Obj(Vec<(String, Rc<Json>)>),
}

impl Json {
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = JsonParser {
            s: text.as_bytes(),
            i: 0,
        };
        let v = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    fn kind(&self) -> i32 {
        match self {
            Json::Missing => -1,
            Json::Null => 0,
            Json::Bool(_) => 1,
            Json::Num(_) => 2,
            Json::Str(_) => 3,
            Json::Arr(_) => 4,
            Json::Obj(_) => 5,
        }
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Missing | Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&json_number(*n)),
            Json::Str(s) => quote(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        let ok = self.s[self.i..].starts_with(lit.as_bytes());
        if ok {
            self.i += lit.len();
        }
        ok
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 128 {
            return None;
        }
        self.ws();
        match *self.s.get(self.i)? {
            b'n' => self.eat("null").then_some(Json::Null),
            b't' => self.eat("true").then_some(Json::Bool(true)),
            b'f' => self.eat("false").then_some(Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(Rc::new(self.value(depth + 1)?));
                    self.ws();
                    if self.eat("]") {
                        return Some(Json::Arr(items));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return None;
                    }
                    fields.push((k, Rc::new(self.value(depth + 1)?)));
                    self.ws();
                    if self.eat("}") {
                        return Some(Json::Obj(fields));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                // JSON numbers: no leading `+`, `.` or redundant zeros.
                let digits = t.strip_prefix('-').unwrap_or(t);
                let valid = digits.starts_with(|c: char| c.is_ascii_digit())
                    && !(digits.len() > 1
                        && digits.starts_with('0')
                        && digits.as_bytes()[1].is_ascii_digit());
                if valid {
                    t.parse().ok().map(Json::Num)
                } else {
                    None
                }
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len()
                && !matches!(self.s[self.i], b'"' | b'\\')
                && self.s[self.i] >= 0x20
            {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).ok()?);
            match *self.s.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.i += 1;
                    let c = *self.s.get(self.i)?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xdc00).contains(&code) && self.eat("\\u") {
                                let low = self.hex4()?;
                                code = 0x10000
                                    + ((code - 0xd800) << 10)
                                    + (low.wrapping_sub(0xdc00) & 0x3ff);
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let t = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(t, 16).ok()
    }
}

/// `JSON.stringify` of a string.
pub fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// JavaScript's `String(x)` for numbers: shortest round-trip digits, exponent form outside
/// 1e-7 < |x| < 1e21.
pub fn js_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return "0".into();
    }
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let (k, n) = (digits.len() as i32, exp.parse::<i32>().unwrap() + 1);
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let rest = if k > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        format!(
            "{}{rest}e{}{}",
            &digits[..1],
            if n > 0 { "+" } else { "-" },
            (n - 1).abs()
        )
    };
    if x < 0.0 { format!("-{body}") } else { body }
}

/// `JSON.stringify` of a number.
fn json_number(x: f64) -> String {
    if x.is_finite() {
        js_number(x)
    } else {
        "null".into()
    }
}

/// The browser host's `formatFloat`: integral values keep a `.0` suffix.
fn format_float(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e21 {
        format!("{x:.1}")
    } else {
        js_number(x)
    }
}

// ----- store -----

/// The persistent key-value store of `save`/`load`, kept as one JSON object in a file.
pub struct Kv {
    path: PathBuf,
    /// Key → JSON text.
    map: BTreeMap<String, String>,
}

impl Kv {
    pub fn open(path: PathBuf) -> Result<Kv, String> {
        let mut map = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            let Some(Json::Obj(fields)) = Json::parse(&text) else {
                return Err(format!("{}: not a JSON object", path.display()));
            };
            for (k, v) in fields {
                let mut s = String::new();
                v.write(&mut s);
                map.insert(k, s);
            }
        }
        Ok(Kv { path, map })
    }

    fn save(&self) -> Result<(), String> {
        let mut s = String::from("{\n");
        for (i, (k, v)) in self.map.iter().enumerate() {
            if i > 0 {
                s.push_str(",\n");
            }
            s.push_str("  ");
            quote(k, &mut s);
            s.push_str(": ");
            s.push_str(v);
        }
        s.push_str("\n}\n");
        let err = |e: std::io::Error| format!("{}: {e}", self.path.display());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        // Write then rename, so a crash never leaves half a file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, s).map_err(err)?;
        std::fs::rename(&tmp, &self.path).map_err(err)
    }
}

// ----- host calls -----

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// HMAC-SHA256 (RFC 2104).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&crate::sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(data);
    let mut outer: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&crate::sha256::digest(&inner));
    crate::sha256::digest(&outer)
}

const SESSION_COOKIE: &str = "vcwl_session";

/// The cookie value of a session: hex(json) + "." + hex(HMAC(secret, json)).
pub fn encode_session(secret: &[u8], session: &BTreeMap<String, String>) -> String {
    let obj = Json::Obj(
        session
            .iter()
            .map(|(k, v)| (k.clone(), Rc::new(Json::Str(v.clone()))))
            .collect(),
    );
    let mut json = String::new();
    obj.write(&mut json);
    format!(
        "{}.{}",
        hex(json.as_bytes()),
        hex(&hmac_sha256(secret, json.as_bytes()))
    )
}

/// The session of a `Cookie` header; missing, malformed or tampered cookies give an empty one.
pub fn decode_session(secret: &[u8], cookie_header: Option<&str>) -> BTreeMap<String, String> {
    let decode = || -> Option<BTreeMap<String, String>> {
        let value = cookie_header?.split(';').find_map(|c| {
            let (k, v) = c.trim().split_once('=')?;
            (k == SESSION_COOKIE).then_some(v)
        })?;
        let (data, mac) = value.split_once('.')?;
        let data = unhex(data)?;
        let mac = unhex(mac)?;
        let expected = hmac_sha256(secret, &data);
        if mac.len() != expected.len()
            || mac.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0
        {
            return None;
        }
        let Json::Obj(fields) = Json::parse(std::str::from_utf8(&data).ok()?)? else {
            return None;
        };
        Some(
            fields
                .into_iter()
                .filter_map(|(k, v)| match &*v {
                    Json::Str(s) => Some((k, s.clone())),
                    _ => None,
                })
                .collect(),
        )
    };
    decode().unwrap_or_default()
}

/// What host calls of one server function call can reach.
pub struct HostCtx {
    pub db: Arc<Mutex<Option<rusqlite::Connection>>>,
    pub db_path: PathBuf,
    pub session: BTreeMap<String, String>,
    pub session_changed: bool,
}

impl HostCtx {
    pub fn new(db: Arc<Mutex<Option<rusqlite::Connection>>>, db_path: PathBuf) -> HostCtx {
        HostCtx {
            db,
            db_path,
            session: BTreeMap::new(),
            session_changed: false,
        }
    }

    fn with_db<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        if db.is_none() {
            if let Some(dir) = self.db_path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let conn = rusqlite::Connection::open(&self.db_path).map_err(|e| e.to_string())?;
            *db = Some(conn);
        }
        f(db.as_ref().expect("opened above")).map_err(|e| e.to_string())
    }
}

fn arg(args: &Json, i: usize) -> Option<&Json> {
    match args {
        Json::Arr(a) => a.get(i).map(|v| &**v),
        _ => None,
    }
}

fn arg_str(args: &Json, i: usize) -> Result<&str, String> {
    match arg(args, i) {
        Some(Json::Str(s)) => Ok(s),
        _ => Err(format!("argument {} must be a string", i + 1)),
    }
}

fn sql_params(args: &Json) -> Result<Vec<String>, String> {
    match arg(args, 1) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Arr(items)) => Ok(items
            .iter()
            .map(|v| match &**v {
                Json::Str(s) => s.clone(),
                other => {
                    let mut s = String::new();
                    other.write(&mut s);
                    s
                }
            })
            .collect()),
        _ => Err("the SQL parameters must be an array".into()),
    }
}

fn sql_value(v: rusqlite::types::ValueRef<'_>) -> Json {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => Json::Null,
        ValueRef::Integer(i) => Json::Num(i as f64),
        ValueRef::Real(f) => Json::Num(f),
        ValueRef::Text(t) => Json::Str(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Json::Str(hex(b)),
    }
}

fn sql_query(ctx: &HostCtx, args: &Json) -> Result<Json, String> {
    let sql = arg_str(args, 0)?;
    let params = sql_params(args)?;
    ctx.with_db(|db| {
        let mut stmt = db.prepare(sql)?;
        let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut fields = Vec::with_capacity(cols.len());
            for (i, c) in cols.iter().enumerate() {
                fields.push((c.clone(), Rc::new(sql_value(row.get_ref(i)?))));
            }
            out.push(Rc::new(Json::Obj(fields)));
        }
        Ok(Json::Arr(out))
    })
}

fn sql_exec(ctx: &HostCtx, args: &Json) -> Result<Json, String> {
    let sql = arg_str(args, 0)?;
    let params = sql_params(args)?;
    ctx.with_db(|db| {
        let n = db.execute(sql, rusqlite::params_from_iter(params.iter()))?;
        Ok(Json::Num(n as f64))
    })
}

/// An HTTP client in a language that is not Rust: we do not pull in an async runtime, a TLS
/// stack and four hundred transitive crates to fetch one URL. We ask curl, like adults.
fn http(args: &Json) -> Result<Json, String> {
    let method = arg_str(args, 0)?;
    let url = arg_str(args, 1)?;
    let body = match arg(args, 2) {
        None | Some(Json::Null) => "",
        Some(Json::Str(s)) => s.as_str(),
        _ => return Err("argument 3 must be a string".into()),
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("not an http(s) URL: {url}"));
    }
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(format!("bad HTTP method: {method}"));
    }
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-X",
        method,
        "--max-time",
        "10",
        "-w",
        "\n%{http_code}",
    ]);
    if !body.is_empty() {
        cmd.args(["--data-binary", "@-"]);
    }
    cmd.arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').ok_or("curl: no status code")?;
    let status: f64 = code
        .trim()
        .parse()
        .map_err(|_| format!("curl: bad status code {code:?}"))?;
    Ok(Json::Obj(vec![
        ("status".into(), Rc::new(Json::Num(status))),
        ("body".into(), Rc::new(Json::Str(body.to_string()))),
    ]))
}

/// Runs host call `name`; the result is `{"ok": value}` or `{"err": message}`.
pub fn host_call(ctx: &mut HostCtx, name: &str, args: &Json) -> Json {
    let result = match name {
        "sql.query" => sql_query(ctx, args),
        "sql.exec" => sql_exec(ctx, args),
        "http" => http(args),
        "session.get" => arg_str(args, 0).map(|k| match ctx.session.get(k) {
            Some(v) => Json::Str(v.clone()),
            None => Json::Null,
        }),
        "session.set" => arg_str(args, 0).and_then(|k| {
            let k = k.to_string();
            match arg(args, 1) {
                Some(Json::Str(v)) => {
                    if ctx.session.get(&k) != Some(v) {
                        ctx.session.insert(k, v.clone());
                        ctx.session_changed = true;
                    }
                    Ok(Json::Null)
                }
                None | Some(Json::Null) => {
                    ctx.session_changed |= ctx.session.remove(&k).is_some();
                    Ok(Json::Null)
                }
                _ => Err("argument 2 must be a string or null".into()),
            }
        }),
        "time.now" => Ok(Json::Num(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_millis() as f64),
        )),
        _ => Err(format!("unknown host call `{name}`")),
    };
    match result {
        Ok(v) => Json::Obj(vec![("ok".into(), Rc::new(v))]),
        Err(e) => Json::Obj(vec![("err".into(), Rc::new(Json::Str(e)))]),
    }
}

// ----- rate limits -----

/// `vcwl.limits.toml`: calls per minute per client, overall and per function.
#[derive(Debug, Default, PartialEq)]
pub struct Limits {
    pub calls_per_minute: Option<u32>,
    pub functions: BTreeMap<String, u32>,
}

impl Limits {
    pub fn parse(text: &str) -> Result<Limits, String> {
        let mut limits = Limits::default();
        let mut section = String::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = s.trim().to_string();
                continue;
            }
            let bad = || format!("vcwl.limits.toml:{}: expected `name = N`", n + 1);
            let (k, v) = line.split_once('=').ok_or_else(bad)?;
            let k = k.trim().trim_matches('"');
            let v: u32 = v.trim().parse().map_err(|_| bad())?;
            match section.as_str() {
                "" if k == "calls_per_minute" => limits.calls_per_minute = Some(v),
                "functions" => {
                    limits.functions.insert(k.to_string(), v);
                }
                _ => {
                    return Err(format!(
                        "vcwl.limits.toml:{}: unknown setting `{k}`",
                        n + 1
                    ));
                }
            }
        }
        Ok(limits)
    }
}

fn random_bytes() -> [u8; 32] {
    let mut buf = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok()
    {
        return buf;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seed = format!("{nanos}:{}:{:p}", std::process::id(), &buf);
    crate::sha256::digest(seed.as_bytes())
}

/// The session signing key: `VCWL_SECRET`, else a random key kept in the store.
fn session_secret(kv: &mut Kv) -> Vec<u8> {
    if let Ok(s) = std::env::var("VCWL_SECRET")
        && !s.is_empty()
    {
        return s.into_bytes();
    }
    const KEY: &str = "__vcwl_secret";
    if let Some(Json::Str(s)) = kv.map.get(KEY).and_then(|t| Json::parse(t))
        && let Some(b) = unhex(&s)
    {
        return b;
    }
    let secret = random_bytes();
    let mut text = String::new();
    quote(&hex(&secret), &mut text);
    kv.map.insert(KEY.into(), text);
    if let Err(e) = kv.save() {
        eprintln!("[server] {e}");
    }
    secret.to_vec()
}

// ----- runtime -----

struct State {
    json: Vec<Rc<Json>>,
    response: Option<Result<String, String>>,
    kv: Arc<Mutex<Kv>>,
    changed: bool,
    limits: StoreLimits,
    host: HostCtx,
}

/// Outcome of a server function call.
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// 200 with the JSON result.
    Ok(String),
    /// 400: the arguments do not match the function's parameters.
    BadRequest(String),
    /// 404: no such server function.
    NotFound,
    /// 500: the function panicked, trapped or ran too long (the details are logged).
    Failed(String),
}

pub struct Functions {
    engine: Engine,
    pre: InstancePre<State>,
    names: Vec<String>,
    kv: Arc<Mutex<Kv>>,
    db: Arc<Mutex<Option<rusqlite::Connection>>>,
    db_path: PathBuf,
    secret: Vec<u8>,
    limits: Limits,
    /// (client, function or "") → (window start, calls in the window).
    rates: Mutex<HashMap<(String, String), (Instant, u32)>>,
}

fn io_read(caller: &mut Caller<'_, State>, len: i32) -> wasmtime::Result<String> {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("no memory export"))?;
    let bytes = mem
        .data(&caller)
        .get(IO_BASE..IO_BASE + len.max(0) as usize)
        .ok_or_else(|| wasmtime::Error::msg("I/O region out of bounds"))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn io_write(caller: &mut Caller<'_, State>, s: &str) -> wasmtime::Result<i32> {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("no memory export"))?;
    let need = IO_BASE + s.len();
    let have = mem.data_size(&caller);
    if need > have {
        mem.grow(&mut *caller, (need - have).div_ceil(65536) as u64)?;
    }
    mem.data_mut(&mut *caller)[IO_BASE..need].copy_from_slice(s.as_bytes());
    Ok(s.len() as i32)
}

fn handle(caller: &Caller<'_, State>, h: i32) -> Rc<Json> {
    caller
        .data()
        .json
        .get(h as usize)
        .cloned()
        .unwrap_or_else(|| Rc::new(Json::Missing))
}

fn push(caller: &mut Caller<'_, State>, v: Rc<Json>) -> i32 {
    let json = &mut caller.data_mut().json;
    json.push(v);
    json.len() as i32 - 1
}

fn linker(engine: &Engine) -> wasmtime::Result<Linker<State>> {
    let mut l = Linker::new(engine);
    l.func_wrap("vcwl", "print", |mut c: Caller<'_, State>, len: i32| {
        println!("[server] {}", io_read(&mut c, len)?);
        Ok(())
    })?;
    l.func_wrap(
        "vcwl",
        "panic",
        |mut c: Caller<'_, State>, len: i32| -> wasmtime::Result<()> {
            Err(wasmtime::Error::msg(format!(
                "panic: {}",
                io_read(&mut c, len)?
            )))
        },
    )?;
    l.func_wrap("vcwl", "fmt_f64", |mut c: Caller<'_, State>, x: f64| {
        io_write(&mut c, &format_float(x))
    })?;
    l.func_wrap("vcwl", "now", || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    })?;
    l.func_wrap("vcwl", "json_kind", |c: Caller<'_, State>, h: i32| {
        handle(&c, h).kind()
    })?;
    l.func_wrap("vcwl", "json_bool", |c: Caller<'_, State>, h: i32| {
        (*handle(&c, h) == Json::Bool(true)) as i32
    })?;
    l.func_wrap(
        "vcwl",
        "json_num",
        |c: Caller<'_, State>, h: i32| match *handle(&c, h) {
            Json::Num(n) => n,
            _ => f64::NAN,
        },
    )?;
    l.func_wrap("vcwl", "json_str", |mut c: Caller<'_, State>, h: i32| {
        let v = handle(&c, h);
        io_write(&mut c, if let Json::Str(s) = &*v { s } else { "" })
    })?;
    l.func_wrap(
        "vcwl",
        "json_len",
        |c: Caller<'_, State>, h: i32| match &*handle(&c, h) {
            Json::Arr(a) => a.len() as i32,
            _ => 0,
        },
    )?;
    l.func_wrap(
        "vcwl",
        "json_index",
        |mut c: Caller<'_, State>, h: i32, i: i32| {
            let v = match &*handle(&c, h) {
                Json::Arr(a) => a.get(i as usize).cloned(),
                _ => None,
            };
            push(&mut c, v.unwrap_or_else(|| Rc::new(Json::Missing)))
        },
    )?;
    l.func_wrap(
        "vcwl",
        "json_field",
        |mut c: Caller<'_, State>, h: i32, len: i32| {
            let key = io_read(&mut c, len)?;
            let v = match &*handle(&c, h) {
                Json::Obj(f) => f.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()),
                _ => None,
            };
            Ok(push(&mut c, v.unwrap_or_else(|| Rc::new(Json::Missing))))
        },
    )?;
    l.func_wrap(
        "vcwl",
        "json_num_text",
        |mut c: Caller<'_, State>, x: f64| io_write(&mut c, &json_number(x)),
    )?;
    l.func_wrap(
        "vcwl",
        "json_quote",
        |mut c: Caller<'_, State>, len: i32| {
            let mut s = String::new();
            quote(&io_read(&mut c, len)?, &mut s);
            io_write(&mut c, &s)
        },
    )?;
    l.func_wrap(
        "vcwl",
        "json_parse",
        |mut c: Caller<'_, State>, len: i32| {
            Ok(match Json::parse(&io_read(&mut c, len)?) {
                Some(v) => push(&mut c, Rc::new(v)),
                None => -1,
            })
        },
    )?;
    l.func_wrap(
        "vcwl",
        "env",
        |mut c: Caller<'_, State>, len: i32| match std::env::var(io_read(&mut c, len)?) {
            Ok(v) => io_write(&mut c, &v),
            Err(_) => Ok(-1),
        },
    )?;
    l.func_wrap("vcwl", "store_get", |mut c: Caller<'_, State>, len: i32| {
        let key = io_read(&mut c, len)?;
        let v = c.data().kv.lock().unwrap().map.get(&key).cloned();
        match v {
            Some(v) => io_write(&mut c, &v),
            None => Ok(-1),
        }
    })?;
    l.func_wrap(
        "vcwl",
        "store_set",
        |mut c: Caller<'_, State>, key_len: i32, len: i32| {
            let text = io_read(&mut c, len)?;
            let split = (key_len.max(0) as usize).min(text.len());
            let (key, value) = text.split_at(split);
            let (key, value) = (key.to_string(), value.to_string());
            c.data().kv.lock().unwrap().map.insert(key, value);
            c.data_mut().changed = true;
            Ok(())
        },
    )?;
    l.func_wrap(
        "vcwl",
        "store_delete",
        |mut c: Caller<'_, State>, len: i32| {
            let key = io_read(&mut c, len)?;
            let removed = c.data().kv.lock().unwrap().map.remove(&key).is_some();
            c.data_mut().changed |= removed;
            Ok(())
        },
    )?;
    l.func_wrap(
        "vcwl",
        "host",
        |mut c: Caller<'_, State>, name_len: i32, len: i32| {
            let text = io_read(&mut c, len)?;
            let split = (name_len.max(0) as usize).min(text.len());
            let (name, args) = if text.is_char_boundary(split) {
                text.split_at(split)
            } else {
                (text.as_str(), "")
            };
            let result = match Json::parse(args) {
                Some(a) => host_call(&mut c.data_mut().host, name, &a),
                None => Json::Obj(vec![(
                    "err".into(),
                    Rc::new(Json::Str("host call arguments are not valid JSON".into())),
                )]),
            };
            Ok(push(&mut c, Rc::new(result)))
        },
    )?;
    l.func_wrap("vcwl", "respond", |mut c: Caller<'_, State>, len: i32| {
        let v = io_read(&mut c, len)?;
        c.data_mut().response = Some(Ok(v));
        Ok(())
    })?;
    l.func_wrap("vcwl", "reject", |mut c: Caller<'_, State>, len: i32| {
        let v = io_read(&mut c, len)?;
        c.data_mut().response = Some(Err(v));
        Ok(())
    })?;
    Ok(l)
}

impl Functions {
    /// Loads a server module; `store` is the file behind `save`/`load`.
    pub fn load(wasm: &[u8], store: PathBuf) -> Result<Functions, String> {
        let mut config = Config::new();
        config
            .wasm_gc(true)
            .wasm_function_references(true)
            .epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| e.to_string())?;
        let module = Module::new(&engine, wasm).map_err(|e| format!("server.wasm: {e}"))?;
        let names = module
            .exports()
            .filter_map(|e| e.name().strip_prefix("__fn_").map(str::to_string))
            .collect();
        let pre = linker(&engine)
            .and_then(|l| l.instantiate_pre(&module))
            .map_err(|e| format!("server.wasm: {e}"))?;
        // Calls that outlive their deadline are interrupted by the epoch ticking here.
        let ticker = engine.weak();
        std::thread::spawn(move || {
            while let Some(e) = ticker.upgrade() {
                e.increment_epoch();
                drop(e);
                std::thread::sleep(TICK);
            }
        });
        let mut db_path = store.clone().into_os_string();
        db_path.push(".sqlite");
        let mut kv = Kv::open(store)?;
        let secret = session_secret(&mut kv);
        Ok(Functions {
            engine,
            pre,
            names,
            kv: Arc::new(Mutex::new(kv)),
            db: Arc::new(Mutex::new(None)),
            db_path: PathBuf::from(db_path),
            secret,
            limits: Limits::default(),
            rates: Mutex::new(HashMap::new()),
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    /// Counts a call of `name` by `client`; false when it is over a limit.
    pub fn allow(&self, client: &str, name: &str) -> bool {
        let checks = [
            (String::new(), self.limits.calls_per_minute),
            (name.to_string(), self.limits.functions.get(name).copied()),
        ];
        let now = Instant::now();
        let mut rates = self.rates.lock().unwrap_or_else(|e| e.into_inner());
        rates.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        let over = checks.iter().any(|(key, limit)| {
            limit.is_some_and(|max| {
                rates
                    .get(&(client.to_string(), key.clone()))
                    .is_some_and(|(_, n)| *n >= max)
            })
        });
        if over {
            return false;
        }
        for (key, limit) in checks {
            if limit.is_some() {
                rates
                    .entry((client.to_string(), key))
                    .or_insert((now, 0))
                    .1 += 1;
            }
        }
        true
    }

    /// Calls server function `name` with the JSON array `args`, without a session.
    pub fn call(&self, name: &str, args: &str) -> Reply {
        self.call_with_cookie(name, args, None).0
    }

    /// Calls server function `name` with the session of the `Cookie` header; also returns the
    /// `Set-Cookie` value when the session changed.
    pub fn call_with_cookie(
        &self,
        name: &str,
        args: &str,
        cookie: Option<&str>,
    ) -> (Reply, Option<String>) {
        if !self.names.iter().any(|n| n == name) {
            return (Reply::NotFound, None);
        }
        let Some(args) = Json::parse(args) else {
            return (
                Reply::BadRequest("the body is not valid JSON".into()),
                None,
            );
        };
        let mut host = HostCtx::new(self.db.clone(), self.db_path.clone());
        host.session = decode_session(&self.secret, cookie);
        let state = State {
            json: vec![Rc::new(args)],
            response: None,
            kv: self.kv.clone(),
            changed: false,
            limits: StoreLimitsBuilder::new().memory_size(256 << 20).build(),
            host,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_epoch_deadline(CALL_TICKS);
        let run = self.pre.instantiate(&mut store).and_then(|inst| {
            let f = inst.get_typed_func::<(), ()>(&mut store, &format!("__fn_{name}"))?;
            f.call(&mut store, ())
        });
        if store.data().changed
            && let Err(e) = self.kv.lock().unwrap().save()
        {
            eprintln!("[server] {e}");
        }
        let host = &store.data().host;
        let set_cookie = host.session_changed.then(|| {
            format!(
                "{SESSION_COOKIE}={}; HttpOnly; SameSite=Lax; Path=/",
                encode_session(&self.secret, &host.session)
            )
        });
        let reply = match (run, store.data_mut().response.take()) {
            (Ok(()), Some(Ok(v))) => Reply::Ok(v),
            (Ok(()), Some(Err(msg))) => Reply::BadRequest(msg),
            (Ok(()), None) => Reply::Failed(format!("{name}: no response")),
            (Err(e), _) => {
                let msg = match e.downcast_ref::<wasmtime::Trap>() {
                    Some(wasmtime::Trap::Interrupt) => "ran longer than its time limit".to_string(),
                    _ => format!("{e:#}"),
                };
                Reply::Failed(format!("{name}: {msg}"))
            }
        };
        (reply, set_cookie)
    }
}

// ----- HTTP -----

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Reads one HTTP/1.1 request. A body larger than `MAX_BODY` is an error.
pub fn read_request(conn: &TcpStream) -> std::io::Result<Request> {
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? <= 2 {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err(std::io::Error::other("too many headers"));
        }
    }
    let mut req = Request {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    let len: usize = req
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err(std::io::Error::other("body too large"));
    }
    req.body.resize(len, 0);
    reader.read_exact(&mut req.body)?;
    Ok(req)
}

pub struct Response {
    pub code: &'static str,
    pub ctype: &'static str,
    pub headers: Vec<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(code: &'static str, ctype: &'static str, body: impl Into<Vec<u8>>) -> Response {
        Response {
            code,
            ctype,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn write(&self, conn: &TcpStream, head: bool) -> std::io::Result<()> {
        let mut w = conn;
        let extra: String = self.headers.iter().map(|h| format!("{h}\r\n")).collect();
        write!(
            w,
            "HTTP/1.1 {}\r\n{extra}content-type: {}\r\ncontent-length: {}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\nconnection: close\r\n\r\n",
            self.code,
            self.ctype,
            self.body.len()
        )?;
        if !head {
            w.write_all(&self.body)?;
        }
        w.flush()
    }
}

/// `POST /__vcwl/fn/<name>`, or `None` if the request is not a server function call.
pub fn function_call(req: &Request, fns: Option<&Functions>) -> Option<Response> {
    function_call_from(req, fns, None)
}

/// [`function_call`] for a client at `peer`, which rate limits are counted against.
pub fn function_call_from(
    req: &Request,
    fns: Option<&Functions>,
    peer: Option<std::net::IpAddr>,
) -> Option<Response> {
    let name = req.path.strip_prefix("/__vcwl/fn/")?;
    let text = |code, msg: &str| Response::new(code, "text/plain; charset=utf-8", msg.to_string());
    if req.method != "POST" {
        return Some(text(
            "405 Method Not Allowed",
            "server functions are called with POST",
        ));
    }
    // A JSON content type cannot be sent cross-site without a CORS preflight, which is never
    // answered; a foreign Origin is refused outright.
    if !req
        .header("content-type")
        .is_some_and(|t| t.starts_with("application/json"))
    {
        return Some(text(
            "415 Unsupported Media Type",
            "expected application/json",
        ));
    }
    if let (Some(origin), Some(host)) = (req.header("origin"), req.header("host"))
        && origin.split_once("://").map(|(_, h)| h) != Some(host)
    {
        return Some(text("403 Forbidden", "cross-origin calls are not allowed"));
    }
    let Some(fns) = fns else {
        return Some(text("404 Not Found", "this app has no server functions"));
    };
    let client = peer.map_or_else(|| "local".to_string(), |ip| ip.to_string());
    if !fns.allow(&client, name) {
        println!("[server] {name}: rate limited {client}");
        return Some(text(
            "429 Too Many Requests",
            "rate limited. VCWL is not Rust: it cannot be blazingly fast for you indefinitely",
        ));
    }
    let start = Instant::now();
    let (reply, set_cookie) = fns.call_with_cookie(
        name,
        &String::from_utf8_lossy(&req.body),
        req.header("cookie"),
    );
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut res = match reply {
        Reply::Ok(v) => {
            println!("[server] {name} ok in {ms:.1} ms");
            Response::new("200 OK", "application/json", v)
        }
        Reply::BadRequest(m) => {
            println!("[server] {name}: bad request: {m}");
            text("400 Bad Request", &m)
        }
        Reply::NotFound => text("404 Not Found", "no such server function"),
        Reply::Failed(m) => {
            eprintln!("[server] {m}");
            text("500 Internal Server Error", "the server function failed")
        }
    };
    if let Some(c) = set_cookie {
        res.headers.push(format!("set-cookie: {c}"));
    }
    Some(res)
}

/// Where `save`/`load` keep their data (`store` in vcwl.toml).
pub fn store_path(p: &Project) -> PathBuf {
    p.root.join(
        project::manifest_value(&p.manifest, "store").unwrap_or_else(|| "data/store.json".into()),
    )
}

/// Loads the server module of a build, if it has one.
pub fn load_functions(p: &Project) -> Result<Option<Functions>, String> {
    let file = project::server_dir(&p.out).join("server.wasm");
    match std::fs::read(&file) {
        Ok(wasm) => {
            let mut fns = Functions::load(&wasm, store_path(p))?;
            if let Ok(text) = std::fs::read_to_string(p.root.join("vcwl.limits.toml")) {
                fns.set_limits(Limits::parse(&text)?);
            }
            Ok(Some(fns))
        }
        Err(_) => Ok(None),
    }
}

/// `vcwl serve [--port N] [--host ADDR]`: a release build served with its server functions.
pub fn run(args: &[String]) -> Result<(), String> {
    let opt = |name: &str| args.iter().position(|a| a == name).map(|i| args.get(i + 1));
    let port = match opt("--port") {
        Some(p) => p
            .and_then(|p| p.parse::<u16>().ok())
            .ok_or("usage: vcwl serve [--port N] [--host ADDR]")?,
        None => 8080,
    };
    let host = match opt("--host") {
        Some(h) => h
            .ok_or("usage: vcwl serve [--port N] [--host ADDR]")?
            .clone(),
        None => "127.0.0.1".into(),
    };
    let p = Project::find(Path::new("."))?;
    project::build(&p.entry, &p.out, false)?;
    let fns = Arc::new(load_functions(&p)?);
    let listener = TcpListener::bind((host.as_str(), port))
        .map_err(|e| format!("cannot listen on {host}:{port}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    match fns.as_ref() {
        Some(f) => println!("server functions: {}", f.names().join(", ")),
        None => println!("no server functions"),
    }
    println!(
        "serving {} at http://{addr}/ (Ctrl-C to stop)",
        p.out.display()
    );
    let _ = std::io::stdout().flush();
    for conn in listener.incoming().flatten() {
        let (out, fns) = (p.out.clone(), fns.clone());
        std::thread::spawn(move || {
            let Ok(req) = read_request(&conn) else {
                let _ = Response::new("400 Bad Request", "text/plain", "bad request")
                    .write(&conn, false);
                return;
            };
            let peer = conn.peer_addr().ok().map(|a| a.ip());
            let res = function_call_from(&req, fns.as_ref().as_ref(), peer).unwrap_or_else(|| {
                match req.method.as_str() {
                    "GET" | "HEAD" => crate::devserver::static_file(&out, &req.path),
                    _ => {
                        Response::new("405 Method Not Allowed", "text/plain", "method not allowed")
                    }
                }
            });
            let _ = res.write(&conn, req.method == "HEAD");
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_json() {
        let v = Json::parse(r#" {"a": [1, -2.5e3, true, null], "b": "x\"é😀\n"} "#).unwrap();
        let mut s = String::new();
        v.write(&mut s);
        assert_eq!(s, "{\"a\":[1,-2500,true,null],\"b\":\"x\\\"é😀\\n\"}");
        for bad in [
            "",
            "[1,]",
            "{\"a\" 1}",
            "01",
            "+1",
            ".5",
            "\"\u{1}\"",
            "[1] x",
            "tru",
        ] {
            assert_eq!(Json::parse(bad), None, "{bad:?}");
        }
        assert!(Json::parse(&"[".repeat(200)).is_none(), "nesting limit");
    }

    #[test]
    fn numbers_match_javascript() {
        for (x, js) in [
            (0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (1.5e-10, "1.5e-10"),
            (2.5e25, "2.5e+25"),
        ] {
            assert_eq!(js_number(x), js, "{x}");
        }
        assert_eq!(format_float(2.0), "2.0");
        assert_eq!(json_number(f64::NAN), "null");
    }

    fn text_of(v: &Json) -> String {
        let mut s = String::new();
        v.write(&mut s);
        s
    }

    #[test]
    fn hmac_rfc4231_case_2() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn session_cookie_roundtrip_and_tamper() {
        let secret = b"secret";
        let mut s = BTreeMap::new();
        s.insert("user".to_string(), "ada".to_string());
        let v = encode_session(secret, &s);
        let header = format!("a=b; {SESSION_COOKIE}={v}");
        assert_eq!(decode_session(secret, Some(&header)), s);
        assert!(decode_session(b"other", Some(&header)).is_empty());
        let (data, mac) = v.split_once('.').unwrap();
        let mut forged = BTreeMap::new();
        forged.insert("user".to_string(), "root".to_string());
        let forged_data = encode_session(secret, &forged);
        let forged_data = forged_data.split_once('.').unwrap().0;
        assert_ne!(forged_data, data);
        let tampered = format!("{SESSION_COOKIE}={forged_data}.{mac}");
        assert!(decode_session(secret, Some(&tampered)).is_empty());
        assert!(decode_session(secret, None).is_empty());
    }

    #[test]
    fn sql_query_and_exec() {
        let dir = std::env::temp_dir().join(format!("vcwl-sql-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ctx = HostCtx::new(Arc::new(Mutex::new(None)), dir.join("store.json.sqlite"));
        let exec = |ctx: &mut HostCtx, args: &str| {
            text_of(&host_call(ctx, "sql.exec", &Json::parse(args).unwrap()))
        };
        assert_eq!(exec(&mut ctx, r#"["create table t (a, b)", []]"#), r#"{"ok":0}"#);
        assert_eq!(
            exec(&mut ctx, r#"["insert into t values (?, ?)", ["1", "x"]]"#),
            r#"{"ok":1}"#
        );
        let rows = host_call(
            &mut ctx,
            "sql.query",
            &Json::parse(r#"["select a, b, 2 as n, 1.5 as f, null as z, x'ab01' as y from t where b = ?", ["x"]]"#)
                .unwrap(),
        );
        assert_eq!(
            text_of(&rows),
            r#"{"ok":[{"a":"1","b":"x","n":2,"f":1.5,"z":null,"y":"ab01"}]}"#
        );
        assert!(exec(&mut ctx, r#"["nonsense", []]"#).starts_with(r#"{"err":"#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_limits() {
        let l = Limits::parse("calls_per_minute = 60\n\n[functions]\nlogin = 5 # tight\n").unwrap();
        assert_eq!(l.calls_per_minute, Some(60));
        assert_eq!(l.functions.get("login"), Some(&5));
        assert!(Limits::parse("calls_per_minute = lots").is_err());
        assert!(Limits::parse("[other]\nx = 1").is_err());
    }

    #[test]
    fn unknown_host_call_and_session() {
        let mut ctx = HostCtx::new(Arc::new(Mutex::new(None)), PathBuf::from("unused.sqlite"));
        assert_eq!(
            text_of(&host_call(&mut ctx, "nope", &Json::Arr(vec![]))),
            r#"{"err":"unknown host call `nope`"}"#
        );
        let set = Json::parse(r#"["k", "v"]"#).unwrap();
        assert_eq!(text_of(&host_call(&mut ctx, "session.set", &set)), r#"{"ok":null}"#);
        assert!(ctx.session_changed);
        let get = Json::parse(r#"["k"]"#).unwrap();
        assert_eq!(text_of(&host_call(&mut ctx, "session.get", &get)), r#"{"ok":"v"}"#);
    }
}
