//! JSON decoders and encoders for `fetch`, generated per type as ordinary typed functions.
//!
//! The host parses responses with the browser's `JSON.parse` and exposes the result through
//! integer handles (`__json_kind`, `__json_field`, ...). The generated decoders validate every
//! value against the expected type at run time: a static type is never trusted as a substitute
//! for checking the response. Decoders return `Result<T, String>`; the error is a path such as
//! `.items[2].name: expected a string, found null`.

use crate::ast::BinOp;
use crate::check::{Checker, FnSig};
use crate::diag::{Diag, Result, Span};
use crate::tir::*;

/// JSON value kinds reported by the host (`__json_kind`).
const KIND_ARRAY: i64 = 4;
const KIND_OBJECT: i64 = 5;

/// What a generated function does for its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Decode,
    Encode,
    FetchResult,
}

struct Gen {
    locals: Vec<Local>,
    span: Span,
}

impl Gen {
    fn local(&mut self, name: &str, ty: Type) -> LocalId {
        self.locals.push(Local { name: name.into(), ty });
        self.locals.len() - 1
    }
    fn e(&self, kind: TK, ty: Type) -> TExpr {
        TExpr { kind, ty, span: self.span }
    }
    fn get(&self, id: LocalId) -> TExpr {
        self.e(TK::Local(id), self.locals[id].ty.clone())
    }
    fn int(&self, n: i64) -> TExpr {
        self.e(TK::Int(n), Type::Int)
    }
    fn str(&self, s: &str) -> TExpr {
        self.e(TK::Str(s.into()), Type::Str)
    }
    fn call(&self, f: usize, args: Vec<TExpr>, ret: Type) -> TExpr {
        self.e(TK::Call(f, vec![], args), ret)
    }
    fn intr(&self, name: &'static str, targs: Vec<Type>, args: Vec<TExpr>, ret: Type) -> TExpr {
        self.e(TK::Intrinsic(name, targs, args), ret)
    }
    fn bin(&self, op: BinOp, a: TExpr, b: TExpr) -> TExpr {
        self.e(TK::Bin(op, Box::new(a), Box::new(b)), Type::Bool)
    }
    fn ret(&self, v: TExpr) -> TExpr {
        self.e(TK::Return(Some(Box::new(v))), Type::Never)
    }
    /// `if cond { return v }` as a statement.
    fn guard(&self, cond: TExpr, v: TExpr) -> TStmt {
        TStmt::Expr(self.e(TK::If(Box::new(cond), Box::new(self.ret(v)), None), Type::Unit))
    }
    fn block(&self, stmts: Vec<TStmt>, tail: TExpr) -> TExpr {
        let ty = tail.ty.clone();
        self.e(TK::Block(stmts, Some(Box::new(tail))), ty)
    }
}

impl Checker<'_> {
    fn result_ty(&self, ok: Type, err: Type) -> Type {
        Type::Adt(self.prelude_adt("Result"), vec![ok, err])
    }

    /// `Ok(v)` (variant 0) or `Err(v)` (variant 1) of `Result<ok, err>`.
    fn result_variant(&self, g: &Gen, ok: &Type, err: &Type, vi: usize, v: TExpr) -> TExpr {
        let targs = vec![ok.clone(), err.clone()];
        g.e(TK::Variant(self.prelude_adt("Result"), targs.clone(), vi, vec![v]), self.result_ty(ok.clone(), err.clone()))
    }

    fn pf(&self, name: &str) -> usize {
        self.prelude_fn(name).unwrap_or_else(|| panic!("prelude function {name} missing"))
    }

    fn concat(&self, g: &Gen, parts: Vec<TExpr>) -> TExpr {
        let f = self.pf("str_concat");
        parts.into_iter().reduce(|a, b| g.call(f, vec![a, b], Type::Str)).unwrap_or_else(|| g.str(""))
    }

    /// `match dec { Ok(v) => v, Err(e) => return Err(prefix + e) }` inside a function
    /// returning `Result<outer, String>`.
    fn try_decode(&self, g: &mut Gen, dec: TExpr, val: &Type, outer: &Type, prefix: TExpr) -> TExpr {
        let result = self.prelude_adt("Result");
        let targs = vec![val.clone(), Type::Str];
        let (v, e) = (g.local("v", val.clone()), g.local("e", Type::Str));
        let msg = self.concat(g, vec![prefix, g.get(e)]);
        let err = self.result_variant(g, outer, &Type::Str, 1, msg);
        let arms = vec![
            (TPat::Variant(result, targs.clone(), 0, vec![TPat::Bind(v)]), g.get(v)),
            (TPat::Variant(result, targs, 1, vec![TPat::Bind(e)]), g.ret(err)),
        ];
        g.e(TK::Match(Box::new(dec), arms), val.clone())
    }

    /// Registers a generated function; its body is filled in by the caller.
    fn new_json_fn(&mut self, key: (Role, Type), name: String, params: Vec<Type>, ret: Type) -> usize {
        self.sigs.push(FnSig { name, generics: vec![], params, ret, file: 0, item: usize::MAX });
        self.fns.push(None);
        let id = self.sigs.len() - 1;
        self.json_fns.insert(key, id);
        id
    }

    fn finish_json_fn(&mut self, id: usize, g: Gen, params: Vec<LocalId>, body: TExpr) {
        let s = &self.sigs[id];
        self.fns[id] = Some(TFn { name: s.name.clone(), generics: vec![], params, locals: g.locals, ret: s.ret.clone(), body, exported: false });
    }

    fn unsupported(&self, t: &Type, span: Span, what: &str) -> Diag {
        Diag::ty(span, format!("cannot {what} JSON for type `{}`", self.show(t)))
            .help("supported: Int, Float, Bool, String, (), arrays, optionals, structs of these, and enums without payloads")
    }

    /// `(h: Int) -> Result<t, String>`: decodes the JSON value behind handle `h`.
    pub fn json_decoder(&mut self, t: &Type, span: Span) -> Result<usize> {
        let scalar = match t {
            Type::Int => Some("json_int"),
            Type::Float => Some("json_float"),
            Type::Bool => Some("json_bool"),
            Type::Str => Some("json_string"),
            _ => None,
        };
        if let Some(f) = scalar {
            return Ok(self.pf(f));
        }
        if let Some(&id) = self.json_fns.get(&(Role::Decode, t.clone())) {
            return Ok(id);
        }
        let ret = self.result_ty(t.clone(), Type::Str);
        let id = self.new_json_fn((Role::Decode, t.clone()), format!("decode<{}>", self.show(t)), vec![Type::Int], ret);
        let mut g = Gen { locals: Vec::new(), span };
        let h = g.local("h", Type::Int);
        let kind = |g: &Gen| g.intr("__json_kind", vec![], vec![g.get(h)], Type::Int);
        let expected = |this: &Self, g: &Gen, what: &str| this.call_expected(g, what, h);
        let body = match t {
            Type::Unit => self.result_variant(&g, t, &Type::Str, 0, g.e(TK::Unit, Type::Unit)),
            Type::Array(el) => {
                let dec = self.json_decoder(el, span)?;
                let el = (**el).clone();
                let elem_res = self.result_ty(el.clone(), Type::Str);
                let n = g.local("n", Type::Int);
                let out = g.local("out", t.clone());
                let i = g.local("i", Type::Int);
                let item = |this: &Self, g: &mut Gen, at: TExpr| {
                    let handle = g.intr("__json_index", vec![], vec![g.get(h), at.clone()], Type::Int);
                    let call = g.call(dec, vec![handle], elem_res.clone());
                    let prefix = this.concat(g, vec![g.str("["), g.call(this.pf("int_to_string"), vec![at], Type::Str), g.str("]")]);
                    this.try_decode(g, call, &el, t, prefix)
                };
                let not_array = g.bin(BinOp::Ne, kind(&g), g.int(KIND_ARRAY));
                let err = expected(self, &g, "an array");
                let empty = self.result_variant(&g, t, &Type::Str, 0, g.e(TK::Array(vec![]), t.clone()));
                let zero = g.int(0);
                let first = item(self, &mut g, zero);
                let at = g.get(i);
                let next = item(self, &mut g, at);
                let lt = g.bin(BinOp::Lt, g.get(i), g.get(n));
                let set = g.intr("__arr_set", vec![el.clone()], vec![g.get(out), g.get(i), next], Type::Unit);
                let inc = g.e(TK::Bin(BinOp::Add, Box::new(g.get(i)), Box::new(g.int(1))), Type::Int);
                let lp = g.e(TK::Block(vec![TStmt::Expr(set), TStmt::Assign(Place::Local(i), inc)], None), Type::Unit);
                let stmts = vec![
                    g.guard(not_array, self.result_variant(&g, t, &Type::Str, 1, err)),
                    TStmt::Let(n, g.intr("__json_len", vec![], vec![g.get(h)], Type::Int)),
                    g.guard(g.bin(BinOp::Eq, g.get(n), g.int(0)), empty),
                    TStmt::Let(out, g.intr("__arr_alloc", vec![el.clone()], vec![g.get(n), first], t.clone())),
                    TStmt::Let(i, g.int(1)),
                    TStmt::While(lt, lp),
                ];
                let ok = self.result_variant(&g, t, &Type::Str, 0, g.get(out));
                g.block(stmts, ok)
            }
            Type::Adt(adt, targs) if *adt == self.prelude_adt("Option") => {
                let inner = targs[0].clone();
                let dec = self.json_decoder(&inner, span)?;
                let opt = self.prelude_adt("Option");
                let none = g.e(TK::Variant(opt, targs.clone(), 1, vec![]), t.clone());
                let absent = g.bin(BinOp::Le, kind(&g), g.int(0));
                let v = g.local("v", inner.clone());
                let call = g.call(dec, vec![g.get(h)], self.result_ty(inner.clone(), Type::Str));
                let prefix = g.str("");
                let value = self.try_decode(&mut g, call, &inner, t, prefix);
                let stmts = vec![g.guard(absent, self.result_variant(&g, t, &Type::Str, 0, none)), TStmt::Let(v, value)];
                let some = g.e(TK::Variant(opt, targs.clone(), 0, vec![g.get(v)]), t.clone());
                let ok = self.result_variant(&g, t, &Type::Str, 0, some);
                g.block(stmts, ok)
            }
            Type::Adt(adt, targs) => {
                let a = self.adts[*adt].clone();
                let map: Vec<(String, Type)> = a.generics.iter().cloned().zip(targs.iter().cloned()).collect();
                match &a.kind {
                    AdtKind::Struct(fields) => {
                        let not_object = g.bin(BinOp::Ne, kind(&g), g.int(KIND_OBJECT));
                        let err = expected(self, &g, "an object");
                        let mut stmts = vec![g.guard(not_object, self.result_variant(&g, t, &Type::Str, 1, err))];
                        let mut vals = Vec::new();
                        for (name, fty) in fields {
                            let fty = fty.subst(&map);
                            let dec = self.json_decoder(&fty, span)?;
                            let handle = g.intr("__json_field", vec![], vec![g.get(h), g.str(name)], Type::Int);
                            let call = g.call(dec, vec![handle], self.result_ty(fty.clone(), Type::Str));
                            let v = g.local(name, fty.clone());
                            let prefix = g.str(&format!(".{name}"));
                            let value = self.try_decode(&mut g, call, &fty, t, prefix);
                            stmts.push(TStmt::Let(v, value));
                            vals.push(g.get(v));
                        }
                        let ok = self.result_variant(&g, t, &Type::Str, 0, g.e(TK::StructNew(*adt, targs.clone(), vals), t.clone()));
                        g.block(stmts, ok)
                    }
                    AdtKind::Enum(variants) if !variants.is_empty() && variants.iter().all(|v| v.1.is_empty()) => {
                        let s = g.local("s", Type::Str);
                        let call = g.call(self.pf("json_string"), vec![g.get(h)], self.result_ty(Type::Str, Type::Str));
                        let prefix = g.str("");
                        let value = self.try_decode(&mut g, call, &Type::Str, t, prefix);
                        let mut stmts = vec![TStmt::Let(s, value)];
                        for (vi, (name, _)) in variants.iter().enumerate() {
                            let is = g.call(self.pf("str_eq"), vec![g.get(s), g.str(name)], Type::Bool);
                            let v = g.e(TK::Variant(*adt, targs.clone(), vi, vec![]), t.clone());
                            stmts.push(g.guard(is, self.result_variant(&g, t, &Type::Str, 0, v)));
                        }
                        let names: Vec<String> = variants.iter().map(|v| format!("\"{}\"", v.0)).collect();
                        let quoted = g.call(self.pf("json_quote"), vec![g.get(s)], Type::Str);
                        let msg = self.concat(&g, vec![g.str(&format!(": expected one of {}, found ", names.join(", "))), quoted]);
                        g.block(stmts, self.result_variant(&g, t, &Type::Str, 1, msg))
                    }
                    _ => return Err(self.unsupported(t, span, "decode")),
                }
            }
            _ => return Err(self.unsupported(t, span, "decode")),
        };
        self.finish_json_fn(id, g, vec![h], body);
        Ok(id)
    }

    fn call_expected(&self, g: &Gen, what: &str, h: LocalId) -> TExpr {
        g.call(self.pf("json_expected"), vec![g.str(what), g.get(h)], Type::Str)
    }

    /// `(v: t) -> String`: the JSON text of `v`.
    pub fn json_encoder(&mut self, t: &Type, span: Span) -> Result<usize> {
        let scalar = match t {
            Type::Int => Some("int_to_string"),
            Type::Float => Some("json_float_text"),
            Type::Bool => Some("bool_to_string"),
            Type::Str => Some("json_quote"),
            _ => None,
        };
        if let Some(f) = scalar {
            return Ok(self.pf(f));
        }
        if let Some(&id) = self.json_fns.get(&(Role::Encode, t.clone())) {
            return Ok(id);
        }
        let id = self.new_json_fn((Role::Encode, t.clone()), format!("encode<{}>", self.show(t)), vec![t.clone()], Type::Str);
        let mut g = Gen { locals: Vec::new(), span };
        let v = g.local("v", t.clone());
        let body = match t {
            Type::Unit => g.str("null"),
            Type::Array(el) => {
                let enc = self.json_encoder(el, span)?;
                let el = (**el).clone();
                let strs = Type::Array(Box::new(Type::Str));
                let (n, parts, i) = (g.local("n", Type::Int), g.local("parts", strs.clone()), g.local("i", Type::Int));
                let item = g.e(TK::Index(Box::new(g.get(v)), Box::new(g.get(i))), el.clone());
                let set = g.intr("__arr_set", vec![Type::Str], vec![g.get(parts), g.get(i), g.call(enc, vec![item], Type::Str)], Type::Unit);
                let inc = g.e(TK::Bin(BinOp::Add, Box::new(g.get(i)), Box::new(g.int(1))), Type::Int);
                let lp = g.e(TK::Block(vec![TStmt::Expr(set), TStmt::Assign(Place::Local(i), inc)], None), Type::Unit);
                let stmts = vec![
                    TStmt::Let(n, g.intr("__arr_len", vec![el.clone()], vec![g.get(v)], Type::Int)),
                    TStmt::Let(parts, g.intr("__arr_alloc", vec![Type::Str], vec![g.get(n), g.str("")], strs)),
                    TStmt::Let(i, g.int(0)),
                    TStmt::While(g.bin(BinOp::Lt, g.get(i), g.get(n)), lp),
                ];
                let joined = g.call(self.pf("arr_join"), vec![g.get(parts), g.str(",")], Type::Str);
                let tail = self.concat(&g, vec![g.str("["), joined, g.str("]")]);
                g.block(stmts, tail)
            }
            Type::Adt(adt, targs) if *adt == self.prelude_adt("Option") => {
                let inner = targs[0].clone();
                let enc = self.json_encoder(&inner, span)?;
                let x = g.local("x", inner.clone());
                let arms = vec![
                    (TPat::Variant(*adt, targs.clone(), 0, vec![TPat::Bind(x)]), g.call(enc, vec![g.get(x)], Type::Str)),
                    (TPat::Variant(*adt, targs.clone(), 1, vec![]), g.str("null")),
                ];
                g.e(TK::Match(Box::new(g.get(v)), arms), Type::Str)
            }
            Type::Adt(adt, targs) => {
                let a = self.adts[*adt].clone();
                let map: Vec<(String, Type)> = a.generics.iter().cloned().zip(targs.iter().cloned()).collect();
                match &a.kind {
                    AdtKind::Struct(fields) => {
                        let mut parts = Vec::new();
                        for (fi, (name, fty)) in fields.iter().enumerate() {
                            let fty = fty.subst(&map);
                            let enc = self.json_encoder(&fty, span)?;
                            let sep = if fi == 0 { "{" } else { "," };
                            parts.push(g.str(&format!("{sep}\"{name}\":")));
                            let field = g.e(TK::Field(Box::new(g.get(v)), fi), fty);
                            parts.push(g.call(enc, vec![field], Type::Str));
                        }
                        parts.push(g.str(if fields.is_empty() { "{}" } else { "}" }));
                        self.concat(&g, parts)
                    }
                    AdtKind::Enum(variants) if variants.iter().all(|v| v.1.is_empty()) => {
                        let arms = variants.iter().enumerate()
                            .map(|(vi, (name, _))| (TPat::Variant(*adt, targs.clone(), vi, vec![]), g.str(&format!("\"{name}\""))))
                            .collect();
                        g.e(TK::Match(Box::new(g.get(v)), arms), Type::Str)
                    }
                    _ => return Err(self.unsupported(t, span, "encode")),
                }
            }
            _ => return Err(self.unsupported(t, span, "encode")),
        };
        self.finish_json_fn(id, g, vec![v], body);
        Ok(id)
    }

    /// `(kind: Int, status: Int, len: Int) -> Result<t, HttpError>`: the outcome of a request
    /// as reported by the host; on success handle 0 is the parsed response body.
    pub fn fetch_result_fn(&mut self, t: &Type, span: Span) -> Result<usize> {
        let dec = self.json_decoder(t, span)?;
        let http = Type::Adt(self.prelude_adt("HttpError"), vec![]);
        let ret = self.result_ty(t.clone(), http.clone());
        if let Some(&id) = self.json_fns.get(&(Role::FetchResult, t.clone())) {
            return Ok(id);
        }
        let id = self.new_json_fn((Role::FetchResult, t.clone()), format!("fetch_result<{}>", self.show(t)), vec![Type::Int; 3], ret.clone());
        let mut g = Gen { locals: Vec::new(), span };
        let (kind, status, len) = (g.local("kind", Type::Int), g.local("status", Type::Int), g.local("len", Type::Int));
        let failed = g.bin(BinOp::Ne, g.get(kind), g.int(0));
        let err = g.call(self.pf("http_error"), vec![g.get(kind), g.get(status), g.get(len)], http.clone());
        let result = self.prelude_adt("Result");
        let targs = vec![t.clone(), Type::Str];
        let (v, e) = (g.local("v", t.clone()), g.local("e", Type::Str));
        let decode_err = g.e(TK::Variant(self.prelude_adt("HttpError"), vec![], 3, vec![self.concat(&g, vec![g.str("$"), g.get(e)])]), http.clone());
        let arms = vec![
            (TPat::Variant(result, targs.clone(), 0, vec![TPat::Bind(v)]), self.result_variant(&g, t, &http, 0, g.get(v))),
            (TPat::Variant(result, targs, 1, vec![TPat::Bind(e)]), self.result_variant(&g, t, &http, 1, decode_err)),
        ];
        let decoded = g.e(TK::Match(Box::new(g.call(dec, vec![g.int(0)], self.result_ty(t.clone(), Type::Str))), arms), ret);
        let body = g.block(vec![g.guard(failed, self.result_variant(&g, t, &http, 1, err))], decoded);
        self.finish_json_fn(id, g, vec![kind, status, len], body);
        Ok(id)
    }
}
