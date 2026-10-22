//! `vcwl serve`: serves the built site and runs its server functions (`server fn`).
//!
//! The compiler puts server functions in their own module (`dist-server/server.wasm`), each
//! exported as `__fn_<name>`. A call is `POST /__vcwl/fn/<name>` with the arguments as a JSON
//! array; the response is the JSON of the result. Each call runs in a fresh instance (no state
//! survives between calls except the store), one call at a time, with a time and memory limit.
//! The host functions mirror the browser host's (`runtime/vcwl.js`): text travels through the
//! I/O region of linear memory, JSON values through integer handles.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

// ----- runtime -----

struct State {
    json: Vec<Rc<Json>>,
    response: Option<Result<String, String>>,
    kv: Arc<Mutex<Kv>>,
    changed: bool,
    limits: StoreLimits,
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
        Ok(Functions {
            engine,
            pre,
            names,
            kv: Arc::new(Mutex::new(Kv::open(store)?)),
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Calls server function `name` with the JSON array `args`.
    pub fn call(&self, name: &str, args: &str) -> Reply {
        if !self.names.iter().any(|n| n == name) {
            return Reply::NotFound;
        }
        let Some(args) = Json::parse(args) else {
            return Reply::BadRequest("the body is not valid JSON".into());
        };
        // One call at a time: calls see each other's writes in order, like a single thread.
        static CALLS: Mutex<()> = Mutex::new(());
        let _turn = CALLS.lock().unwrap_or_else(|e| e.into_inner());
        let state = State {
            json: vec![Rc::new(args)],
            response: None,
            kv: self.kv.clone(),
            changed: false,
            limits: StoreLimitsBuilder::new().memory_size(256 << 20).build(),
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
        match (run, store.data_mut().response.take()) {
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
        }
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
    let start = std::time::Instant::now();
    let reply = fns.call(name, &String::from_utf8_lossy(&req.body));
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    Some(match reply {
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
    })
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
        Ok(wasm) => Functions::load(&wasm, store_path(p)).map(Some),
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
            let res = function_call(&req, fns.as_ref().as_ref()).unwrap_or_else(|| {
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
}
