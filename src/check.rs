//! Name resolution and type checking: AST -> typed IR.

use std::collections::HashMap;

use crate::ast::{self, BinOp, ExprKind, PatKind, Stmt, TypeExprKind, UnOp};
use crate::diag::{Diag, Result, Span};
use crate::tir::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sym {
    Adt(usize),
    Fn(usize),
    Comp(usize),
    Style(usize),
}

#[derive(Clone)]
pub struct FnSig {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<Type>,
    pub ret: Type,
    pub file: usize,
    pub item: usize,
    /// `server fn`.
    pub server: bool,
    /// `extern fn` (JavaScript).
    pub ext: bool,
}

/// Where a body is being checked; controls which effects are allowed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ctx {
    Fn,
    Handler,
    Render,
}

pub struct Checker<'a> {
    pub files: &'a [ast::File],
    pub adts: Vec<Adt>,
    pub sigs: Vec<FnSig>,
    pub fns: Vec<Option<TFn>>,
    pub scopes: Vec<HashMap<String, Sym>>,
    pub diags: Vec<Diag>,
    // ----- per-body state -----
    pub file: usize,
    pub locals: Vec<Local>,
    pub local_scopes: Vec<Vec<(String, LocalId, bool)>>,
    pub subst: Vec<Option<Type>>,
    pub ret: Type,
    pub loops: u32,
    pub generics: Vec<String>,
    pub ctx: Ctx,
    pub ui: Option<crate::check_ui::UiEnv>,
    /// Spans of expressions whose type contained inference variables, for error reporting.
    pending: Vec<(Span, Type)>,
    /// Generated JSON functions by role and type; see `json.rs`.
    pub json_fns: HashMap<(crate::json::Role, Type), usize>,
    /// `local_scopes` index of the current handler's own scope (fetch arms cannot see it).
    pub handler_base: usize,
    /// Handler scopes hidden while checking `fetch` arms (for diagnostics).
    pub fetch_hidden: Vec<Vec<(String, LocalId, bool)>>,
    /// Checking the body of a server function.
    pub server: bool,
    /// First `fetch` in the program (it requires the network capability).
    pub fetch_span: Option<Span>,
    /// Closures being checked, innermost last.
    lambdas: Vec<LambdaFrame>,
    /// Interfaces (struct ADTs of closures).
    pub ifaces: Vec<usize>,
    /// `impl Interface for Type`: (type, interface, method function per interface field).
    pub impls: Vec<(usize, usize, Vec<usize>)>,
    /// What `Self` means in the current method.
    self_ty: Option<Type>,
    /// The block about to be checked may contain `await` statements (a handler's body, or what
    /// follows an `await`).
    pub await_top: bool,
}

struct LambdaFrame {
    /// `local_scopes` index of the closure's own scope: names from below it are captured.
    base: usize,
    captures: Vec<(TExpr, LocalId)>,
    /// The rest of a handler after an `await`: it runs in the component, so it reads state
    /// directly and may assign its copies of the handler's variables.
    resumes: bool,
}

/// Identifies a captured value, so a closure captures it once.
fn capture_key(k: &TK) -> Option<(u8, usize)> {
    match k {
        TK::Local(id) => Some((0, *id)),
        TK::CompVar(i) => Some((1, *i)),
        TK::Route => Some((2, 0)),
        _ => None,
    }
}

const INTRINSICS: &[(&str, &[&str], &str)] = &[
    ("__str_len", &["String"], "Int"),
    ("__str_byte", &["String", "Int"], "Int"),
    ("__str_alloc", &["Int"], "String"),
    ("__str_set_byte", &["String", "Int", "Int"], "()"),
    (
        "__str_copy",
        &["String", "Int", "String", "Int", "Int"],
        "()",
    ),
    ("__arr_len", &["[T]"], "Int"),
    ("__arr_alloc", &["Int", "T"], "[T]"),
    ("__arr_set", &["[T]", "Int", "T"], "()"),
    ("__arr_push", &["[T]", "T"], "[T]"),
    ("__arr_copy", &["[T]", "Int", "[T]", "Int", "Int"], "()"),
    ("__float_to_string", &["Float"], "String"),
    ("__float_from_int", &["Int"], "Float"),
    ("__int_from_float", &["Float"], "Int"),
    ("__float_floor", &["Float"], "Float"),
    ("__float_ceil", &["Float"], "Float"),
    ("__float_sqrt", &["Float"], "Float"),
    ("__float_round", &["Float"], "Float"),
    ("__print", &["String"], "()"),
    ("__panic", &["String"], "!"),
    ("__now_ms", &[], "Float"),
    ("__str_from_io", &["Int"], "String"),
    ("__json_kind", &["Int"], "Int"),
    ("__json_bool", &["Int"], "Bool"),
    ("__json_num", &["Int"], "Float"),
    ("__json_str", &["Int"], "String"),
    ("__json_num_text", &["Float"], "String"),
    ("__json_quote", &["String"], "String"),
    ("__json_parse", &["String"], "Int"),
    ("__env", &["String"], "Int"),
    ("__store_get", &["String"], "Int"),
    ("__store_set", &["Int", "String"], "()"),
    ("__store_delete", &["String"], "()"),
    ("__js_call", &["Int", "String"], "Int"),
    ("__json_drop", &["Int"], "()"),
];

impl<'a> Checker<'a> {
    pub fn new(files: &'a [ast::File]) -> Checker<'a> {
        Checker {
            files,
            adts: Vec::new(),
            sigs: Vec::new(),
            fns: Vec::new(),
            scopes: vec![HashMap::new(); files.len()],
            diags: Vec::new(),
            file: 0,
            locals: Vec::new(),
            local_scopes: Vec::new(),
            subst: Vec::new(),
            ret: Type::Unit,
            loops: 0,
            generics: Vec::new(),
            ctx: Ctx::Fn,
            ui: None,
            pending: Vec::new(),
            json_fns: HashMap::new(),
            handler_base: 0,
            fetch_span: None,
            fetch_hidden: Vec::new(),
            server: false,
            lambdas: Vec::new(),
            ifaces: Vec::new(),
            impls: Vec::new(),
            self_ty: None,
            await_top: false,
        }
    }

    // ----- declarations -----

    /// Registers all top-level names, resolves type and function signatures.
    pub fn declare(&mut self, imports: &[Vec<usize>]) {
        let mut decls: Vec<(usize, &ast::Item, Sym, &ast::Ident, bool)> = Vec::new();
        let mut comp_count = 0;
        let mut style_count = 0;
        for (fi, f) in self.files.iter().enumerate() {
            for (ii, item) in f.items.iter().enumerate() {
                let (sym, name, exported) = match item {
                    ast::Item::Struct(s) => {
                        self.adts.push(Adt {
                            name: s.name.name.clone(),
                            generics: names(&s.generics),
                            kind: AdtKind::Struct(vec![]),
                        });
                        if s.interface {
                            self.ifaces.push(self.adts.len() - 1);
                        }
                        (Sym::Adt(self.adts.len() - 1), &s.name, s.exported)
                    }
                    ast::Item::Enum(e) => {
                        self.adts.push(Adt {
                            name: e.name.name.clone(),
                            generics: names(&e.generics),
                            kind: AdtKind::Enum(vec![]),
                        });
                        (Sym::Adt(self.adts.len() - 1), &e.name, e.exported)
                    }
                    ast::Item::Fn(f) => {
                        self.sigs.push(FnSig {
                            name: f.name.name.clone(),
                            generics: names(&f.generics),
                            params: vec![],
                            ret: Type::Unit,
                            file: fi,
                            item: ii,
                            server: f.server,
                            ext: f.ext,
                        });
                        self.fns.push(None);
                        (Sym::Fn(self.sigs.len() - 1), &f.name, f.exported)
                    }
                    ast::Item::Component(c) => {
                        comp_count += 1;
                        (Sym::Comp(comp_count - 1), &c.name, c.exported)
                    }
                    ast::Item::Style(s) => {
                        style_count += 1;
                        (Sym::Style(style_count - 1), &s.name, s.exported)
                    }
                    _ => continue,
                };
                if let Some(prev) = self.scopes[fi].insert(name.name.clone(), sym) {
                    let _ = prev;
                    self.diags.push(Diag::ty(
                        name.span,
                        format!("`{}` is defined more than once in this file", name.name),
                    ));
                }
                decls.push((fi, item, sym, name, exported));
            }
        }
        // Make exported names of imported files (and the prelude, file 0) visible.
        for (fi, deps) in imports.iter().enumerate() {
            let mut visible: Vec<usize> = deps.clone();
            if fi != 0 {
                visible.insert(0, 0);
            }
            for dep in visible {
                for &(df, _, sym, name, exported) in &decls {
                    if df != dep || !exported {
                        continue;
                    }
                    match self.scopes[fi].get(&name.name) {
                        Some(&s) if s == sym => {}
                        Some(_) if dep == 0 => {} // local definitions shadow the prelude
                        Some(_) => self.diags.push(Diag::ty(name.span, format!(
                            "`{}` is imported into `{}` but that file already defines or imports a `{}`",
                            name.name, self.files[fi].path, name.name))),
                        None => {
                            self.scopes[fi].insert(name.name.clone(), sym);
                        }
                    }
                }
            }
        }
        // Resolve ADT bodies and function signatures.
        for &(fi, item, sym, _, _) in &decls {
            self.file = fi;
            let r: Result<()> = (|| {
                match (item, sym) {
                    (ast::Item::Struct(s), Sym::Adt(id)) => {
                        self.generics = names(&s.generics);
                        let mut fields: Vec<(String, Type)> = Vec::new();
                        for (n, t) in &s.fields {
                            if fields.iter().any(|(m, _)| *m == n.name) {
                                return Err(Diag::ty(
                                    n.span,
                                    format!("duplicate field `{}`", n.name),
                                ));
                            }
                            fields.push((n.name.clone(), self.resolve_type(t)?));
                        }
                        self.adts[id].kind = AdtKind::Struct(fields);
                    }
                    (ast::Item::Enum(e), Sym::Adt(id)) => {
                        self.generics = names(&e.generics);
                        let mut vs: Vec<(String, Vec<Type>)> = Vec::new();
                        for v in &e.variants {
                            if vs.iter().any(|(m, _)| *m == v.name.name) {
                                return Err(Diag::ty(
                                    v.name.span,
                                    format!("duplicate variant `{}`", v.name.name),
                                ));
                            }
                            let fs = v
                                .fields
                                .iter()
                                .map(|t| self.resolve_type(t))
                                .collect::<Result<Vec<_>>>()?;
                            vs.push((v.name.name.clone(), fs));
                        }
                        if vs.is_empty() {
                            return Err(Diag::ty(
                                e.name.span,
                                "an enum needs at least one variant",
                            ));
                        }
                        self.adts[id].kind = AdtKind::Enum(vs);
                    }
                    (ast::Item::Fn(f), Sym::Fn(id)) => {
                        self.generics = names(&f.generics);
                        self.self_ty = self.method_self(&f.name, &self.generics)?;
                        let params = f
                            .params
                            .iter()
                            .map(|p| self.resolve_type(&p.ty))
                            .collect::<Result<Vec<_>>>()?;
                        let ret = match &f.ret {
                            Some(t) => self.resolve_type(t)?,
                            None => Type::Unit,
                        };
                        if f.name.name.starts_with("__") && fi != 0 {
                            return Err(Diag::ty(
                                f.name.span,
                                "names starting with `__` are reserved for the standard library",
                            ));
                        }
                        self.sigs[id].params = params;
                        self.sigs[id].ret = ret;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(d) = r {
                self.diags.push(d);
            }
        }
        self.self_ty = None;
        self.generics.clear();
        for (fi, f) in self.files.iter().enumerate() {
            for item in &f.items {
                if let ast::Item::Impl(im) = item
                    && let Err(d) = self.declare_impl(fi, im)
                {
                    self.diags.push(d);
                }
            }
        }
    }

    /// `Self` for a method `Type.name` (whose first type parameters are the type's), or `None`.
    fn method_self(&self, name: &ast::Ident, generics: &[String]) -> Result<Option<Type>> {
        let Some((ty, _)) = name.name.split_once('.') else {
            return Ok(None);
        };
        let Some(Sym::Adt(id)) = self.scopes[self.file].get(ty).copied() else {
            return Err(Diag::ty(name.span, format!("unknown type `{ty}`")));
        };
        let n = self.adts[id].generics.len();
        if generics.len() < n {
            return Err(
                Diag::ty(name.span, format!("`{ty}` is generic")).help(format!(
                    "name its type parameters: `impl {ty}<{}>`",
                    self.adts[id].generics.join(", ")
                )),
            );
        }
        Ok(Some(Type::Adt(
            id,
            generics[..n]
                .iter()
                .map(|g| Type::Param(g.clone()))
                .collect(),
        )))
    }

    /// Checks `impl Interface for Type`: every interface method exists with the declared signature.
    fn declare_impl(&mut self, fi: usize, im: &ast::ImplDecl) -> Result<()> {
        let scope = &self.scopes[fi];
        let Some(Sym::Adt(ty)) = scope.get(&im.ty.name).copied() else {
            return Err(Diag::ty(
                im.ty.span,
                format!("unknown type `{}`", im.ty.name),
            ));
        };
        let Some(iname) = &im.iface else {
            return Ok(());
        };
        let iface = match scope.get(&iname.name).copied() {
            Some(Sym::Adt(i)) if self.ifaces.contains(&i) => i,
            _ => {
                return Err(Diag::ty(
                    iname.span,
                    format!("`{}` is not an interface", iname.name),
                ));
            }
        };
        if !self.adts[ty].generics.is_empty() {
            return Err(Diag::ty(
                im.ty.span,
                "generic types cannot implement interfaces",
            ));
        }
        let self_t = Type::Adt(ty, vec![]);
        let mut fns = Vec::new();
        for (m, mt) in self.adts[iface].fields().to_vec() {
            let Type::Fn(ps, r) = mt else { unreachable!() };
            let want: Vec<Type> = std::iter::once(self_t.clone()).chain(ps).collect();
            let found = match self.scopes[fi].get(&format!("{}.{m}", im.ty.name)).copied() {
                Some(Sym::Fn(f)) => Some(f),
                _ => None,
            };
            match found {
                Some(f)
                    if self.sigs[f].params == want
                        && self.sigs[f].ret == *r
                        && self.sigs[f].generics.is_empty() =>
                {
                    fns.push(f)
                }
                _ => {
                    let params: String = want[1..]
                        .iter()
                        .map(|t| format!(", _: {}", self.show(t)))
                        .collect();
                    let ret = if *r == Type::Unit {
                        String::new()
                    } else {
                        format!(" -> {}", self.show(&r))
                    };
                    return Err(Diag::ty(
                        im.span,
                        format!(
                            "`{}` does not implement `{}`: it needs `fn {m}(self{params}){ret}`",
                            im.ty.name, iname.name
                        ),
                    ));
                }
            }
        }
        self.impls.push((ty, iface, fns));
        Ok(())
    }

    pub fn check_fns(&mut self) {
        for id in 0..self.sigs.len() {
            let (fi, ii) = (self.sigs[id].file, self.sigs[id].item);
            let ast::Item::Fn(f) = &self.files[fi].items[ii] else {
                unreachable!()
            };
            match self.check_fn(id, f) {
                Ok(tf) => self.fns[id] = Some(tf),
                Err(d) => self.diags.push(d),
            }
        }
        self.server = false;
        self.self_ty = None;
    }

    fn check_fn(&mut self, id: usize, f: &ast::FnDecl) -> Result<TFn> {
        self.file = self.sigs[id].file;
        self.server = f.server;
        if f.server && !f.generics.is_empty() {
            return Err(Diag::ty(f.name.span, "server functions cannot be generic"));
        }
        self.begin_body(names(&f.generics), self.sigs[id].ret.clone(), Ctx::Fn);
        self.self_ty = self.method_self(&f.name, &self.generics)?;
        let mut params = Vec::new();
        for (p, ty) in f.params.iter().zip(self.sigs[id].params.clone()) {
            if params
                .iter()
                .any(|&l: &LocalId| self.locals[l].name == p.name.name)
            {
                return Err(Diag::ty(
                    p.name.span,
                    format!("duplicate parameter `{}`", p.name.name),
                ));
            }
            params.push(self.bind(&p.name.name, ty, false));
        }
        let ret = self.ret.clone();
        if f.ext {
            // The arguments go to the JavaScript function as JSON; its result comes back the same way.
            let args = params
                .iter()
                .map(|&l| TExpr {
                    kind: TK::Local(l),
                    ty: self.locals[l].ty.clone(),
                    span: f.span,
                })
                .collect();
            let json = self.server_args(args, f.span)?;
            let wrapper = self.extern_call_fn(&ret, f.span)?;
            let name = TExpr {
                kind: TK::Str(f.name.name.clone()),
                ty: Type::Str,
                span: f.span,
            };
            let body = TExpr {
                kind: TK::Call(wrapper, vec![], vec![name, json]),
                ty: ret.clone(),
                span: f.span,
            };
            let body = self.finish_body(body)?;
            return Ok(TFn {
                name: f.name.name.clone(),
                generics: Vec::new(),
                params,
                locals: std::mem::take(&mut self.locals),
                ret,
                body,
                exported: f.exported,
            });
        }
        let mut body = self.check_block(&f.body, Some(&ret))?;
        if ret != Type::Unit {
            if let TK::Block(_, Some(tail)) = &mut body.kind {
                let t = std::mem::replace(
                    &mut **tail,
                    TExpr {
                        kind: TK::Unit,
                        ty: Type::Unit,
                        span: f.span,
                    },
                );
                **tail = self.box_iface(t, &ret);
                body.ty = tail.ty.clone();
            }
            self.coerce(
                &body.ty,
                &ret,
                f.body.tail.as_ref().map(|t| t.span).unwrap_or(f.body.span),
            )
            .map_err(|d| {
                if self.zonk(&body.ty) == Type::Unit {
                    Diag::ty(
                        f.name.span,
                        format!(
                            "function `{}` must return a value of type `{}` on every path",
                            f.name.name,
                            self.show(&ret)
                        ),
                    )
                    .help("end the body with the value to return, or add `return value`")
                } else {
                    d
                }
            })?;
        }
        let body = self.finish_body(body)?;
        Ok(TFn {
            name: f.name.name.clone(),
            generics: names(&f.generics),
            params,
            locals: std::mem::take(&mut self.locals),
            ret,
            body,
            exported: f.exported,
        })
    }

    pub fn begin_body(&mut self, generics: Vec<String>, ret: Type, ctx: Ctx) {
        self.generics = generics;
        self.locals.clear();
        self.local_scopes = vec![Vec::new()];
        self.subst.clear();
        self.pending.clear();
        self.ret = ret;
        self.loops = 0;
        self.ctx = ctx;
    }

    /// Resolves inference variables in a finished body and its locals.
    pub fn finish_body(&mut self, mut body: TExpr) -> Result<TExpr> {
        for (span, t) in std::mem::take(&mut self.pending) {
            let z = self.zonk(&t);
            if has_var(&z) {
                return Err(Diag::ty(
                    span,
                    format!(
                        "cannot infer the type of this expression (`{}`)",
                        self.show(&z)
                    ),
                )
                .help("add a type annotation, e.g. `let items: [Int] = []`"));
            }
        }
        for i in 0..self.locals.len() {
            let t = self.zonk(&self.locals[i].ty);
            self.locals[i].ty = default_vars(&t);
        }
        self.zonk_expr(&mut body);
        Ok(body)
    }

    pub fn zonk_expr(&self, e: &mut TExpr) {
        e.ty = default_vars(&self.zonk(&e.ty));
        let fix = |ts: &mut Vec<Type>| {
            for t in ts.iter_mut() {
                *t = default_vars(&self.zonk(t));
            }
        };
        match &mut e.kind {
            TK::Bin(_, a, b) | TK::Index(a, b) => {
                self.zonk_expr(a);
                self.zonk_expr(b);
            }
            TK::Not(a) | TK::Neg(a) | TK::Field(a, _) => self.zonk_expr(a),
            TK::Call(_, ts, args)
            | TK::Intrinsic(_, ts, args)
            | TK::StructNew(_, ts, args)
            | TK::Variant(_, ts, _, args) => {
                fix(ts);
                args.iter_mut().for_each(|a| self.zonk_expr(a));
            }
            TK::Array(items) => items.iter_mut().for_each(|a| self.zonk_expr(a)),
            TK::If(c, t, e2) => {
                self.zonk_expr(c);
                self.zonk_expr(t);
                if let Some(e2) = e2 {
                    self.zonk_expr(e2);
                }
            }
            TK::Block(stmts, tail) => {
                stmts.iter_mut().for_each(|s| self.zonk_stmt(s));
                if let Some(t) = tail {
                    self.zonk_expr(t);
                }
            }
            TK::Match(s, arms) => {
                self.zonk_expr(s);
                for (p, b) in arms {
                    self.zonk_pat(p);
                    self.zonk_expr(b);
                }
            }
            TK::Return(Some(v)) => self.zonk_expr(v),
            TK::Lambda(l) => {
                l.captures.iter_mut().for_each(|(c, _)| self.zonk_expr(c));
                self.zonk_expr(&mut l.body);
            }
            TK::FnRef(_, ts) => fix(ts),
            TK::CallValue(f, args) => {
                self.zonk_expr(f);
                args.iter_mut().for_each(|a| self.zonk_expr(a));
            }
            _ => {}
        }
    }

    fn zonk_pat(&self, p: &mut TPat) {
        if let TPat::Variant(_, ts, _, ps) = p {
            for t in ts.iter_mut() {
                *t = default_vars(&self.zonk(t));
            }
            ps.iter_mut().for_each(|p| self.zonk_pat(p));
        }
    }

    pub fn zonk_stmt(&self, s: &mut TStmt) {
        match s {
            TStmt::Let(_, e) | TStmt::Assign(_, e) | TStmt::Expr(e) => self.zonk_expr(e),
            TStmt::While(c, b) | TStmt::ForArray(_, c, b) => {
                self.zonk_expr(c);
                self.zonk_expr(b);
            }
            TStmt::ForRange(_, a, b, body) => {
                self.zonk_expr(a);
                self.zonk_expr(b);
                self.zonk_expr(body);
            }
            TStmt::Fetch(f) => {
                f.captures.iter_mut().for_each(|(c, _)| self.zonk_expr(c));
                self.zonk_expr(&mut f.url);
                if let Some(b) = &mut f.body {
                    self.zonk_expr(b);
                }
                self.zonk_expr(&mut f.on_done);
            }
            TStmt::Emit(_, Some(e)) => self.zonk_expr(e),
            TStmt::Break | TStmt::Continue | TStmt::Emit(_, None) => {}
        }
    }

    // ----- types -----

    pub fn resolve_type(&mut self, t: &ast::TypeExpr) -> Result<Type> {
        Ok(match &t.kind {
            TypeExprKind::Unit => Type::Unit,
            TypeExprKind::Array(inner) => Type::Array(Box::new(self.resolve_type(inner)?)),
            TypeExprKind::Optional(inner) => {
                let inner = self.resolve_type(inner)?;
                Type::Adt(self.prelude_adt("Option"), vec![inner])
            }
            TypeExprKind::Fn(ps, r) => Type::Fn(
                ps.iter()
                    .map(|p| self.resolve_type(p))
                    .collect::<Result<_>>()?,
                Box::new(match r {
                    Some(r) => self.resolve_type(r)?,
                    None => Type::Unit,
                }),
            ),
            TypeExprKind::Named(name, args) if name == "Self" && args.is_empty() => {
                return self.self_ty.clone().ok_or_else(|| {
                    Diag::ty(t.span, "`Self` is only available inside `impl` blocks")
                });
            }
            TypeExprKind::Named(name, args) => {
                let prim = match name.as_str() {
                    "Int" => Some(Type::Int),
                    "Float" => Some(Type::Float),
                    "Bool" => Some(Type::Bool),
                    "String" => Some(Type::Str),
                    _ => None,
                };
                if let Some(p) = prim {
                    if !args.is_empty() {
                        return Err(Diag::ty(
                            t.span,
                            format!("`{name}` does not take type arguments"),
                        ));
                    }
                    return Ok(p);
                }
                if self.generics.contains(name) {
                    if !args.is_empty() {
                        return Err(Diag::ty(
                            t.span,
                            "type parameters cannot take type arguments",
                        ));
                    }
                    return Ok(Type::Param(name.clone()));
                }
                match self.scopes[self.file].get(name) {
                    Some(Sym::Adt(id)) => {
                        let id = *id;
                        let want = self.adts[id].generics.len();
                        if args.len() != want {
                            return Err(Diag::ty(
                                t.span,
                                format!(
                                    "`{name}` expects {want} type argument(s), found {}",
                                    args.len()
                                ),
                            ));
                        }
                        let args = args
                            .iter()
                            .map(|a| self.resolve_type(a))
                            .collect::<Result<Vec<_>>>()?;
                        Type::Adt(id, args)
                    }
                    _ => {
                        let mut d = Diag::ty(t.span, format!("unknown type `{name}`"));
                        let cands: Vec<String> = ["Int", "Float", "Bool", "String"]
                            .iter()
                            .map(|s| s.to_string())
                            .chain(
                                self.scopes[self.file]
                                    .iter()
                                    .filter(|(_, s)| matches!(s, Sym::Adt(_)))
                                    .map(|(n, _)| n.clone()),
                            )
                            .collect();
                        if let Some(s) = suggest(name, cands.iter().map(|s| s.as_str())) {
                            d = d.help(format!("did you mean `{s}`?"));
                        }
                        return Err(d);
                    }
                }
            }
        })
    }

    pub fn prelude_adt(&self, name: &str) -> usize {
        match self.scopes[0].get(name) {
            Some(Sym::Adt(id)) => *id,
            _ => panic!("prelude type {name} missing"),
        }
    }

    pub fn prelude_fn(&self, name: &str) -> Option<usize> {
        match self.scopes[0].get(name) {
            Some(Sym::Fn(id)) => Some(*id),
            _ => None,
        }
    }

    pub fn option(&self, t: Type) -> Type {
        Type::Adt(self.prelude_adt("Option"), vec![t])
    }

    pub fn show(&self, t: &Type) -> String {
        match self.zonk(t) {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Str => "String".into(),
            Type::Unit => "()".into(),
            Type::Never => "Never".into(),
            Type::Array(t) => format!("[{}]", self.show(&t)),
            Type::Adt(id, args)
                if self.adts[id].name == "Option" && id == self.prelude_adt("Option") =>
            {
                format!("{}?", self.show(&args[0]))
            }
            Type::Adt(id, args) if args.is_empty() => self.adts[id].name.clone(),
            Type::Adt(id, args) => format!(
                "{}<{}>",
                self.adts[id].name,
                args.iter()
                    .map(|a| self.show(a))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Type::Param(n) => n,
            Type::Fn(ps, r) => {
                let ps: Vec<String> = ps.iter().map(|p| self.show(p)).collect();
                match *r {
                    Type::Unit => format!("fn({})", ps.join(", ")),
                    r => format!("fn({}) -> {}", ps.join(", "), self.show(&r)),
                }
            }
            Type::Var(_) => "_".into(),
        }
    }

    pub fn fresh(&mut self) -> Type {
        self.subst.push(None);
        Type::Var(self.subst.len() as u32 - 1)
    }

    pub fn zonk(&self, t: &Type) -> Type {
        match t {
            Type::Var(v) => match self.subst.get(*v as usize).cloned().flatten() {
                Some(t) => self.zonk(&t),
                None => t.clone(),
            },
            Type::Array(t) => Type::Array(Box::new(self.zonk(t))),
            Type::Adt(id, args) => Type::Adt(*id, args.iter().map(|a| self.zonk(a)).collect()),
            Type::Fn(ps, r) => Type::Fn(
                ps.iter().map(|p| self.zonk(p)).collect(),
                Box::new(self.zonk(r)),
            ),
            _ => t.clone(),
        }
    }

    fn occurs(&self, v: u32, t: &Type) -> bool {
        match self.zonk(t) {
            Type::Var(w) => w == v,
            Type::Array(t) => self.occurs(v, &t),
            Type::Adt(_, args) => args.iter().any(|a| self.occurs(v, a)),
            Type::Fn(ps, r) => ps.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &r),
            _ => false,
        }
    }

    pub fn unify(&mut self, a: &Type, b: &Type) -> bool {
        let (a, b) = (self.zonk(a), self.zonk(b));
        match (&a, &b) {
            (Type::Var(x), Type::Var(y)) if x == y => true,
            (Type::Var(x), t) | (t, Type::Var(x)) => {
                if *t == Type::Never || self.occurs(*x, t) {
                    return *t == Type::Never;
                }
                self.subst[*x as usize] = Some(t.clone());
                true
            }
            (Type::Array(x), Type::Array(y)) => self.unify(x, y),
            (Type::Adt(i, xs), Type::Adt(j, ys)) => {
                i == j && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (Type::Fn(xs, r), Type::Fn(ys, q)) => {
                xs.len() == ys.len()
                    && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y))
                    && self.unify(r, q)
            }
            _ => a == b,
        }
    }

    /// Checks that a value of type `actual` can be used where `expected` is required.
    pub fn coerce(&mut self, actual: &Type, expected: &Type, span: Span) -> Result<()> {
        if self.zonk(actual) == Type::Never || self.unify(actual, expected) {
            return Ok(());
        }
        let (a, e) = (self.show(actual), self.show(expected));
        let mut d = Diag::ty(span, format!("expected `{e}`, found `{a}`"));
        let za = self.zonk(actual);
        let ze = self.zonk(expected);
        d = match (&za, &ze) {
            (Type::Int, Type::Float) => d.help("convert explicitly with `x.to_float()`"),
            (Type::Float, Type::Int) => {
                d.help("convert explicitly with `x.to_int()` (truncates) or `x.round().to_int()`")
            }
            (Type::Int | Type::Float | Type::Bool, Type::Str) => {
                d.help("convert explicitly with `x.to_string()` or use interpolation: \"{x}\"")
            }
            (_, Type::Adt(id, args))
                if *id == self.prelude_adt("Option") && self.zonk(&args[0]) == za =>
            {
                d.help("wrap the value: `Some(value)`")
            }
            (Type::Adt(id, args), _)
                if *id == self.prelude_adt("Option") && self.zonk(&args[0]) == ze =>
            {
                d.help("the value may be absent; handle it with `match` or `.unwrap_or(default)`")
            }
            _ => d,
        };
        Err(d)
    }

    // ----- scopes -----

    pub fn bind(&mut self, name: &str, ty: Type, mutable: bool) -> LocalId {
        self.locals.push(Local {
            name: name.to_string(),
            ty,
        });
        let id = self.locals.len() - 1;
        self.local_scopes
            .last_mut()
            .unwrap()
            .push((name.to_string(), id, mutable));
        id
    }

    fn lookup_local(&self, name: &str) -> Option<(LocalId, bool)> {
        self.lookup_local_at(name).map(|(id, m, _)| (id, m))
    }

    /// The local named `name`, whether it is mutable, and the index of its scope.
    fn lookup_local_at(&self, name: &str) -> Option<(LocalId, bool, usize)> {
        self.local_scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(d, s)| {
                s.iter()
                    .rev()
                    .find(|(n, _, _)| n == name)
                    .map(|&(_, id, m)| (id, m, d))
            })
    }

    /// Starts checking the rest of a handler after an `await`.
    pub fn begin_resume(&mut self) {
        self.lambdas.push(LambdaFrame {
            base: self.local_scopes.len(),
            captures: Vec::new(),
            resumes: true,
        });
    }

    pub fn end_resume(&mut self) -> Vec<(TExpr, LocalId)> {
        self.lambdas.pop().unwrap().captures
    }

    /// Routes a value defined at scope `depth` through the captures of every closure that
    /// encloses the use but not the definition.
    fn capture(&mut self, mut e: TExpr, depth: usize) -> TExpr {
        for k in 0..self.lambdas.len() {
            let key = capture_key(&e.kind);
            if self.lambdas[k].base <= depth
                || self.lambdas[k].resumes && !matches!(e.kind, TK::Local(_))
            {
                continue;
            }
            let found = self.lambdas[k]
                .captures
                .iter()
                .find(|(o, _)| capture_key(&o.kind) == key)
                .map(|c| c.1);
            let inner = found.unwrap_or_else(|| {
                self.locals.push(Local {
                    name: "$captured".into(),
                    ty: e.ty.clone(),
                });
                let l = self.locals.len() - 1;
                self.lambdas[k].captures.push((e.clone(), l));
                l
            });
            e.kind = TK::Local(inner);
        }
        e
    }

    fn visible_value_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .local_scopes
            .iter()
            .flatten()
            .map(|(n, _, _)| n.clone())
            .collect();
        if let Some(ui) = &self.ui {
            v.extend(ui.names());
        }
        v
    }

    // ----- statements and blocks -----

    pub fn check_block(&mut self, b: &ast::Block, expected: Option<&Type>) -> Result<TExpr> {
        self.local_scopes.push(Vec::new());
        let r = self.check_block_inner(b, expected);
        self.local_scopes.pop();
        r
    }

    fn check_block_inner(&mut self, b: &ast::Block, expected: Option<&Type>) -> Result<TExpr> {
        let mut stmts = Vec::new();
        let mut diverges = false;
        let await_top = std::mem::replace(&mut self.await_top, false);
        for (i, s) in b.stmts.iter().enumerate() {
            if await_top && let Some(f) = self.await_split(b, i) {
                stmts.push(self.check_fetch(&f, true)?);
                return Ok(TExpr {
                    kind: TK::Block(stmts, None),
                    ty: Type::Unit,
                    span: b.span,
                });
            }
            let ts = self.check_stmt(s)?;
            diverges |= stmt_diverges(&ts);
            stmts.push(ts);
        }
        let tail = match &b.tail {
            Some(t) => Some(Box::new(self.check_expr(t, expected)?)),
            None => None,
        };
        let ty = match &tail {
            Some(t) => t.ty.clone(),
            None if diverges => Type::Never,
            None => Type::Unit,
        };
        Ok(TExpr {
            kind: TK::Block(stmts, tail),
            ty,
            span: b.span,
        })
    }

    /// When statement `i` of `b` is `[let x =] await f(...)`: a `fetch` whose single arm binds
    /// the result and runs the rest of the block.
    fn await_split(&self, b: &ast::Block, i: usize) -> Option<ast::Fetch> {
        let (f, bind) = match &b.stmts[i] {
            Stmt::Let { value, .. } | Stmt::Expr(value) => match &value.kind {
                ExprKind::Await(f) => (f, matches!(b.stmts[i], Stmt::Let { .. })),
                _ => return None,
            },
            _ => return None,
        };
        let span = f.span;
        let mut rest = Vec::new();
        if let Stmt::Let {
            name,
            mutable,
            ty,
            span: s,
            ..
        } = &b.stmts[i]
        {
            rest.push(Stmt::Let {
                name: name.clone(),
                mutable: *mutable,
                ty: ty.clone(),
                value: ast::Expr {
                    id: 0,
                    kind: ExprKind::Ident("$awaited".into()),
                    span,
                },
                span: *s,
            });
        }
        rest.extend(b.stmts[i + 1..].iter().cloned());
        let body = ast::Block {
            stmts: rest,
            tail: b.tail.clone(),
            span: b.span,
        };
        let pat = if bind {
            PatKind::Bind("$awaited".into())
        } else {
            PatKind::Wild
        };
        let mut f = (**f).clone();
        f.arms = vec![ast::Arm {
            pat: ast::Pattern { kind: pat, span },
            body: ast::Expr {
                id: 0,
                kind: ExprKind::Block(body),
                span: b.span,
            },
        }];
        Some(f)
    }

    pub fn check_stmt(&mut self, s: &Stmt) -> Result<TStmt> {
        Ok(match s {
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
                ..
            } => {
                let ann = match ty {
                    Some(t) => Some(self.resolve_type(t)?),
                    None => None,
                };
                let mut v = self.check_expr(value, ann.as_ref())?;
                if let Some(a) = &ann {
                    v = self.coerce_into(v, a)?;
                }
                let ty = ann.unwrap_or_else(|| v.ty.clone());
                if self.zonk(&ty) == Type::Unit {
                    return Err(Diag::ty(value.span, "cannot bind a value of type `()`"));
                }
                self.pending.push((value.span, ty.clone()));
                let id = self.bind(&name.name, ty, *mutable);
                TStmt::Let(id, v)
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => {
                let (place, ty) = self.check_place(target)?;
                let mut v = match op {
                    None => self.check_expr(value, Some(&ty))?,
                    Some(op) => {
                        let cur = TExpr {
                            kind: place_read(&place),
                            ty: ty.clone(),
                            span: target.span,
                        };
                        let rhs = self.check_expr(value, Some(&ty))?;
                        self.binary(*op, cur, rhs, *span)?
                    }
                };
                if op.is_none() {
                    v = self.coerce_into(v, &ty)?;
                }
                self.coerce(&v.ty, &ty, value.span)?;
                TStmt::Assign(place, v)
            }
            Stmt::Expr(e) => TStmt::Expr(self.check_expr(e, None)?),
            Stmt::While { cond, body, .. } => {
                let c = self.check_expr(cond, Some(&Type::Bool))?;
                self.coerce(&c.ty, &Type::Bool, cond.span)?;
                self.loops += 1;
                let b = self.check_block(body, None);
                self.loops -= 1;
                TStmt::While(c, b?)
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                self.local_scopes.push(Vec::new());
                let r = (|| {
                    if let ExprKind::Range(a, b) = &iter.kind {
                        let a = self.check_expr(a, Some(&Type::Int))?;
                        self.coerce(&a.ty, &Type::Int, a.span)?;
                        let bb = self.check_expr(b, Some(&Type::Int))?;
                        self.coerce(&bb.ty, &Type::Int, bb.span)?;
                        let id = self.bind(&var.name, Type::Int, false);
                        self.loops += 1;
                        let body = self.check_block(body, None);
                        self.loops -= 1;
                        return Ok(TStmt::ForRange(id, a, bb, body?));
                    }
                    let arr = self.check_expr(iter, None)?;
                    let elem = match self.zonk(&arr.ty) {
                        Type::Array(t) => *t,
                        t => {
                            return Err(Diag::ty(
                                iter.span,
                                format!("cannot iterate over `{}`", self.show(&t)),
                            )
                            .help("`for` loops accept arrays and ranges like `0..n`"));
                        }
                    };
                    let id = self.bind(&var.name, elem, false);
                    self.loops += 1;
                    let body = self.check_block(body, None);
                    self.loops -= 1;
                    Ok(TStmt::ForArray(id, arr, body?))
                })();
                self.local_scopes.pop();
                r?
            }
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loops == 0 {
                    return Err(Diag::ty(*span, "`break`/`continue` outside of a loop"));
                }
                if matches!(s, Stmt::Break(_)) {
                    TStmt::Break
                } else {
                    TStmt::Continue
                }
            }
            Stmt::Fetch(f) => self.check_fetch(f, false)?,
            Stmt::Emit { name, value, span } => self.check_emit(name, value.as_ref(), *span)?,
        })
    }

    fn check_place(&mut self, target: &ast::Expr) -> Result<(Place, Type)> {
        match &target.kind {
            ExprKind::Ident(name) => {
                let closed = |depth: usize| self.lambdas.iter().any(|l| !l.resumes && depth < l.base);
                let captured = match self.lookup_local_at(name) {
                    Some((_, _, depth)) => closed(depth),
                    None => self.ui.is_some() && closed(0),
                };
                if captured {
                    return Err(Diag::ty(target.span, format!("a closure cannot assign to `{name}`"))
                        .help("closures capture copies of the values they use; return the new value instead"));
                }
                if let Some((id, mutable)) = self.lookup_local(name) {
                    if !mutable {
                        return Err(Diag::ty(target.span, format!("cannot assign to `{name}`: it is immutable"))
                            .help(format!("declare it with `var {name} = ...` to make it mutable")));
                    }
                    let depth = self.lookup_local_at(name).unwrap().2;
                    let e = TExpr {
                        kind: TK::Local(id),
                        ty: self.locals[id].ty.clone(),
                        span: target.span,
                    };
                    let TK::Local(id) = self.capture(e, depth).kind else { unreachable!() };
                    return Ok((Place::Local(id), self.locals[id].ty.clone()));
                }
                if self.ui.is_some() {
                    return self.ui_place(name, target.span);
                }
                Err(Diag::ty(target.span, format!("unknown variable `{name}`")))
            }
            ExprKind::Field(..) => Err(Diag::ty(target.span, "record fields cannot be assigned")
                .help("records are immutable; build an updated copy with `Name { ..value, field: new }`")),
            ExprKind::Index(..) => Err(Diag::ty(target.span, "array elements cannot be assigned")
                .help("arrays are immutable; use `items = items.set(i, value)`")),
            _ => Err(Diag::ty(target.span, "this expression cannot be assigned to")),
        }
    }

    // ----- expressions -----

    pub fn check_expr(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Result<TExpr> {
        let mk = |kind, ty| TExpr {
            kind,
            ty,
            span: e.span,
        };
        Ok(match &e.kind {
            ExprKind::Int(n) => mk(TK::Int(*n), Type::Int),
            ExprKind::Float(n) => mk(TK::Float(*n), Type::Float),
            ExprKind::Bool(b) => mk(TK::Bool(*b), Type::Bool),
            ExprKind::Unit => mk(TK::Unit, Type::Unit),
            ExprKind::Str(parts) => self.check_interp(parts, e.span)?,
            ExprKind::Dim(n, u) => {
                return Err(Diag::ty(
                    e.span,
                    format!("`{n}{u}` is a style value and can only be used in `style` blocks"),
                ));
            }
            ExprKind::Color(c) => {
                return Err(Diag::ty(
                    e.span,
                    format!("`#{c}` is a color and can only be used in `style` blocks"),
                ));
            }
            ExprKind::Ident(name) => self.check_ident(name, e.span, expected)?,
            ExprKind::Binary(op, a, b) => {
                let hint = match op {
                    BinOp::And | BinOp::Or => Some(Type::Bool),
                    _ => None,
                };
                let ta = self.check_expr(a, hint.as_ref())?;
                let tb = self.check_expr(b, Some(&ta.ty.clone()))?;
                self.binary(*op, ta, tb, e.span)?
            }
            ExprKind::Unary(op, a) => {
                let ta = self.check_expr(a, None)?;
                let t = self.zonk(&ta.ty);
                match (op, &t) {
                    (UnOp::Not, Type::Bool) => mk(TK::Not(Box::new(ta)), Type::Bool),
                    (UnOp::Neg, Type::Int | Type::Float) => mk(TK::Neg(Box::new(ta)), t),
                    _ => {
                        let sym = if *op == UnOp::Not { "!" } else { "-" };
                        return Err(Diag::ty(
                            e.span,
                            format!("cannot apply `{sym}` to `{}`", self.show(&t)),
                        ));
                    }
                }
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
            } => self.check_call(callee, type_args, args, e.span, expected)?,
            ExprKind::Method {
                recv,
                name,
                type_args,
                args,
            } => self.check_method(recv, name, type_args, args, e.span, expected)?,
            ExprKind::Field(recv, name) => {
                if let ExprKind::Ident(q) = &recv.kind
                    && let Some(v) = self.qualified_variant(q, &name.name)
                {
                    return self.check_variant(v.0, v.1, &[], e.span, expected);
                }
                let r = self.check_expr(recv, None)?;
                match self.zonk(&r.ty) {
                    Type::Adt(id, targs) if matches!(self.adts[id].kind, AdtKind::Struct(_)) => {
                        let fields = self.adts[id].fields();
                        let Some(i) = fields.iter().position(|(n, _)| *n == name.name) else {
                            let mut d = Diag::ty(
                                name.span,
                                format!("`{}` has no field `{}`", self.adts[id].name, name.name),
                            );
                            if let Some(s) =
                                suggest(&name.name, fields.iter().map(|(n, _)| n.as_str()))
                            {
                                d = d.help(format!("did you mean `{s}`?"));
                            }
                            return Err(d);
                        };
                        let map: Vec<(String, Type)> =
                            self.adts[id].generics.iter().cloned().zip(targs).collect();
                        let ft = fields[i].1.subst(&map);
                        mk(TK::Field(Box::new(r), i), ft)
                    }
                    t => {
                        let mut d = Diag::ty(
                            name.span,
                            format!("`{}` has no field `{}`", self.show(&t), name.name),
                        );
                        if matches!(t, Type::Adt(id, _) if id == self.prelude_adt("Option")) {
                            d = d.help("the value is optional; unwrap it with `match` first");
                        }
                        return Err(d);
                    }
                }
            }
            ExprKind::Index(a, i) => {
                let ta = self.check_expr(a, None)?;
                let ti = self.check_expr(i, Some(&Type::Int))?;
                self.coerce(&ti.ty, &Type::Int, i.span)?;
                let elem = match self.zonk(&ta.ty) {
                    Type::Array(t) => *t,
                    t => {
                        return Err(Diag::ty(
                            a.span,
                            format!("cannot index into `{}`", self.show(&t)),
                        ));
                    }
                };
                mk(TK::Index(Box::new(ta), Box::new(ti)), elem)
            }
            ExprKind::Try(inner) => self.check_try(inner, e.span)?,
            ExprKind::If { cond, then, els } => {
                let c = self.check_expr(cond, Some(&Type::Bool))?;
                self.coerce(&c.ty, &Type::Bool, cond.span)?;
                match els {
                    None => {
                        let t = self.check_block(then, None)?;
                        mk(TK::If(Box::new(c), Box::new(t), None), Type::Unit)
                    }
                    Some(els) => {
                        let t = self.check_block(then, expected)?;
                        let hint = if self.zonk(&t.ty) == Type::Never {
                            expected.cloned()
                        } else {
                            Some(t.ty.clone())
                        };
                        let f = self.check_expr(els, hint.as_ref())?;
                        let ty = self.join(&t, &f, els.span)?;
                        mk(TK::If(Box::new(c), Box::new(t), Some(Box::new(f))), ty)
                    }
                }
            }
            ExprKind::Block(b) => self.check_block(b, expected)?,
            ExprKind::Match { scrut, arms } => {
                let s = self.check_expr(scrut, None)?;
                let sty = self.zonk(&s.ty);
                let mut tarms = Vec::new();
                let mut ty: Option<Type> = None;
                for arm in arms {
                    self.local_scopes.push(Vec::new());
                    let r = (|| {
                        let p = self.check_pat(&arm.pat, &sty)?;
                        let hint = ty.clone().or_else(|| expected.cloned());
                        let b = self.check_expr(&arm.body, hint.as_ref())?;
                        Ok::<_, Diag>((p, b))
                    })();
                    self.local_scopes.pop();
                    let (p, b) = r?;
                    match &ty {
                        None if self.zonk(&b.ty) != Type::Never => ty = Some(b.ty.clone()),
                        Some(t) => {
                            let t = t.clone();
                            self.coerce(&b.ty, &t, arm.body.span).map_err(|d| {
                                Diag::ty(
                                    d.span,
                                    format!("match arms have incompatible types: {}", d.msg),
                                )
                            })?;
                        }
                        None => {}
                    }
                    tarms.push((p, b));
                }
                self.check_exhaustive(&sty, &tarms, e.span)?;
                mk(TK::Match(Box::new(s), tarms), ty.unwrap_or(Type::Never))
            }
            ExprKind::StructLit { name, fields, base } => {
                self.check_struct_lit(name, fields, base.as_deref(), e.span, expected)?
            }
            ExprKind::Array(items) => {
                let elem = match expected.map(|t| self.zonk(t)) {
                    Some(Type::Array(t)) => *t,
                    _ => self.fresh(),
                };
                let mut out = Vec::new();
                for it in items {
                    let t = self.check_expr(it, Some(&elem))?;
                    let t = self.coerce_into(t, &elem).map_err(|d| {
                        Diag::ty(
                            d.span,
                            format!("array elements must all have the same type: {}", d.msg),
                        )
                    })?;
                    out.push(t);
                }
                let ty = Type::Array(Box::new(elem));
                self.pending.push((e.span, ty.clone()));
                mk(TK::Array(out), ty)
            }
            ExprKind::Range(..) => {
                return Err(Diag::ty(e.span, "ranges can only be used in `for` loops"));
            }
            ExprKind::Lambda { params, ret, body } => {
                self.check_lambda(params, ret.as_ref(), body, e.span, expected)?
            }
            ExprKind::Await(_) => {
                return Err(Diag::ty(e.span, "`await` can only start a statement in an event handler")
                    .help("write `let result = await get(url) as T` directly in the handler, outside `if`, `match` and loops"));
            }
            ExprKind::Apply(f, args) => {
                let f = self.check_expr(f, None)?;
                self.call_value(f, args, e.span)?
            }
            ExprKind::Return(v) => {
                if self.ctx == Ctx::Render {
                    return Err(Diag::ty(
                        e.span,
                        "`return` is not allowed in view expressions",
                    ));
                }
                let ret = self.ret.clone();
                let tv = match v {
                    Some(v) => {
                        let tv = self.check_expr(v, Some(&ret))?;
                        Some(Box::new(self.coerce_into(tv, &ret)?))
                    }
                    None => {
                        if ret != Type::Unit {
                            return Err(Diag::ty(
                                e.span,
                                format!(
                                    "this function must return a value of type `{}`",
                                    self.show(&ret)
                                ),
                            ));
                        }
                        None
                    }
                };
                mk(TK::Return(tv), Type::Never)
            }
        })
    }

    fn join(&mut self, a: &TExpr, b: &TExpr, span: Span) -> Result<Type> {
        if self.zonk(&a.ty) == Type::Never {
            return Ok(b.ty.clone());
        }
        self.coerce(&b.ty, &a.ty, span).map_err(|d| {
            Diag::ty(
                d.span,
                format!("`if` and `else` have incompatible types: {}", d.msg),
            )
        })?;
        Ok(a.ty.clone())
    }

    fn check_interp(&mut self, parts: &[ast::StrPart], span: Span) -> Result<TExpr> {
        let mut pieces = Vec::new();
        for p in parts {
            match p {
                ast::StrPart::Lit(s) => pieces.push(TExpr {
                    kind: TK::Str(s.clone()),
                    ty: Type::Str,
                    span,
                }),
                ast::StrPart::Expr(e) => {
                    let t = self.check_expr(e, None)?;
                    pieces.push(self.to_string_expr(t)?);
                }
            }
        }
        let mut it = pieces.into_iter();
        let mut acc = it.next().unwrap();
        for p in it {
            let f = self.prelude_fn("str_concat").expect("prelude str_concat");
            acc = TExpr {
                kind: TK::Call(f, vec![], vec![acc, p]),
                ty: Type::Str,
                span,
            };
        }
        Ok(acc)
    }

    pub fn to_string_expr(&mut self, t: TExpr) -> Result<TExpr> {
        let span = t.span;
        let f = match self.zonk(&t.ty) {
            Type::Str => return Ok(t),
            Type::Int => "int_to_string",
            Type::Float => "float_to_string",
            Type::Bool => "bool_to_string",
            ty => {
                return Err(Diag::ty(
                    span,
                    format!("cannot interpolate a value of type `{}`", self.show(&ty)),
                )
                .help(
                    "only String, Int, Float and Bool can be interpolated; convert the value first",
                ));
            }
        };
        let f = self.prelude_fn(f).unwrap();
        Ok(TExpr {
            kind: TK::Call(f, vec![], vec![t]),
            ty: Type::Str,
            span,
        })
    }

    pub fn binary(&mut self, op: BinOp, a: TExpr, b: TExpr, span: Span) -> Result<TExpr> {
        let (ta, tb) = (self.zonk(&a.ty), self.zonk(&b.ty));
        let err = |this: &Self, extra: Option<&str>| {
            let mut d = Diag::ty(
                span,
                format!(
                    "cannot apply `{}` to `{}` and `{}`",
                    op.symbol(),
                    this.show(&ta),
                    this.show(&tb)
                ),
            );
            if let Some(h) = extra {
                d = d.help(h);
            }
            Err(d)
        };
        if ta != tb && ta != Type::Never && tb != Type::Never {
            let help = match (&ta, &tb) {
                (Type::Str, Type::Int | Type::Float | Type::Bool)
                | (Type::Int | Type::Float | Type::Bool, Type::Str) => {
                    Some("convert explicitly with `.to_string()`, or use interpolation: \"{a}{b}\"")
                }
                (Type::Int, Type::Float) | (Type::Float, Type::Int) => {
                    Some("Int and Float do not mix implicitly; use `.to_float()` or `.to_int()`")
                }
                _ => None,
            };
            return err(self, help);
        }
        let t = if ta == Type::Never {
            tb.clone()
        } else {
            ta.clone()
        };
        use BinOp::*;
        let ty = match op {
            Add if t == Type::Str => {
                let f = self.prelude_fn("str_concat").unwrap();
                return Ok(TExpr {
                    kind: TK::Call(f, vec![], vec![a, b]),
                    ty: Type::Str,
                    span,
                });
            }
            Add | Sub | Mul | Div | Rem if matches!(t, Type::Int | Type::Float) => t.clone(),
            Lt | Le | Gt | Ge if matches!(t, Type::Int | Type::Float) => Type::Bool,
            Eq | Ne if t == Type::Str => {
                let f = self.prelude_fn("str_eq").unwrap();
                let call = TExpr {
                    kind: TK::Call(f, vec![], vec![a, b]),
                    ty: Type::Bool,
                    span,
                };
                return Ok(if op == Ne {
                    TExpr {
                        kind: TK::Not(Box::new(call)),
                        ty: Type::Bool,
                        span,
                    }
                } else {
                    call
                });
            }
            Eq | Ne if matches!(t, Type::Int | Type::Float | Type::Bool) => Type::Bool,
            Eq | Ne if matches!(&t, Type::Adt(id, _) if self.adts[*id].variants().iter().all(|v| v.1.is_empty()) && !self.adts[*id].variants().is_empty()) => {
                Type::Bool
            }
            Eq | Ne => {
                return err(
                    self,
                    Some(
                        "`==` works on Int, Float, Bool, String and enums without payloads; use `match` for other values",
                    ),
                );
            }
            And | Or if t == Type::Bool => Type::Bool,
            _ => return err(self, None),
        };
        Ok(TExpr {
            kind: TK::Bin(op, Box::new(a), Box::new(b)),
            ty,
            span,
        })
    }

    fn check_ident(&mut self, name: &str, span: Span, expected: Option<&Type>) -> Result<TExpr> {
        if let Some((id, _, depth)) = self.lookup_local_at(name) {
            let e = TExpr {
                kind: TK::Local(id),
                ty: self.locals[id].ty.clone(),
                span,
            };
            return Ok(self.capture(e, depth));
        }
        if self.ui.is_some()
            && let Some(t) = self.ui_ident(name, span)?
        {
            return Ok(self.capture(t, 0));
        }
        if let Some((adt, v)) = self.unqualified_variant(name, span)? {
            return self.check_variant(adt, v, &[], span, expected);
        }
        if let Some(Sym::Fn(id)) = self.scopes[self.file].get(name).copied()
            && !self.sigs[id].server
            && !name.starts_with("server_")
        {
            let targs: Vec<Type> = (0..self.sigs[id].generics.len())
                .map(|_| self.fresh())
                .collect();
            let map: Vec<(String, Type)> = self.sigs[id]
                .generics
                .iter()
                .cloned()
                .zip(targs.clone())
                .collect();
            let ty = Type::Fn(
                self.sigs[id].params.iter().map(|p| p.subst(&map)).collect(),
                Box::new(self.sigs[id].ret.subst(&map)),
            );
            for t in &targs {
                self.pending.push((span, t.clone()));
            }
            return Ok(TExpr {
                kind: TK::FnRef(id, targs),
                ty,
                span,
            });
        }
        let mut d = Diag::ty(span, format!("unknown name `{name}`"));
        if self.fetch_hidden.iter().flatten().any(|(n, ..)| n == name) {
            d = d.help("`fetch` arms run after the handler has returned, so they cannot use its local values; store the value in component state first");
        } else if matches!(self.scopes[self.file].get(name), Some(Sym::Fn(_))) {
            d = d.help(format!(
                "`{name}` is a function; call it with `{name}(...)`"
            ));
        } else if let Some(s) = suggest(name, self.visible_value_names().iter().map(|s| s.as_str()))
        {
            d = d.help(format!("did you mean `{s}`?"));
        }
        Err(d)
    }

    /// `x?` becomes `match x { Ok(v) => v, Err(e) => return Err(e) }` (or the `Option` analogue).
    fn check_try(&mut self, inner: &ast::Expr, span: Span) -> Result<TExpr> {
        let v = self.check_expr(inner, None)?;
        let vt = self.zonk(&v.ty);
        let ret = self.zonk(&self.ret.clone());
        let (res, opt) = (self.prelude_adt("Result"), self.prelude_adt("Option"));
        let (Type::Adt(a, targs), Type::Adt(r, rargs)) = (&vt, &ret) else {
            return Err(match vt {
                Type::Adt(a, _) if a == res || a == opt => self.try_context_error(&vt, &ret, span),
                _ => Diag::ty(
                    inner.span,
                    format!(
                        "`?` applies to `Result` or optional values, not `{}`",
                        self.show(&vt)
                    ),
                ),
            });
        };
        if a != r || (*a != res && *a != opt) {
            return Err(self.try_context_error(&vt, &ret, span));
        }
        if *a == res && !self.unify(&targs[1], &rargs[1]) {
            return Err(Diag::ty(
                span,
                format!(
                    "`?` cannot return error `{}` from a function returning `{}`",
                    self.show(&targs[1]),
                    self.show(&ret)
                ),
            )
            .help("convert the error with `match` first"));
        }
        let mk = |kind, ty| TExpr { kind, ty, span };
        self.local_scopes.push(Vec::new());
        let ok = self.bind("$ok", targs[0].clone(), false);
        let fail = if *a == res {
            vec![self.bind("$err", targs[1].clone(), false)]
        } else {
            vec![]
        };
        self.local_scopes.pop();
        let fail_args = fail
            .iter()
            .map(|&l| mk(TK::Local(l), self.locals[l].ty.clone()))
            .collect();
        let early = mk(
            TK::Return(Some(Box::new(mk(
                TK::Variant(*a, rargs.clone(), 1, fail_args),
                ret.clone(),
            )))),
            Type::Never,
        );
        let arms = vec![
            (
                TPat::Variant(*a, targs.clone(), 0, vec![TPat::Bind(ok)]),
                mk(TK::Local(ok), targs[0].clone()),
            ),
            (
                TPat::Variant(
                    *a,
                    targs.clone(),
                    1,
                    fail.into_iter().map(TPat::Bind).collect(),
                ),
                early,
            ),
        ];
        Ok(mk(TK::Match(Box::new(v), arms), targs[0].clone()))
    }

    fn try_context_error(&self, vt: &Type, ret: &Type, span: Span) -> Diag {
        let want = if matches!(vt, Type::Adt(a, _) if *a == self.prelude_adt("Result")) {
            "a `Result`"
        } else {
            "an optional"
        };
        Diag::ty(
            span,
            format!(
                "`?` on `{}` needs a function returning {want}, but this one returns `{}`",
                self.show(vt),
                self.show(ret)
            ),
        )
        .help("handle both cases with `match` instead")
    }

    fn qualified_variant(&self, enum_name: &str, variant: &str) -> Option<(usize, usize)> {
        if self.lookup_local(enum_name).is_some() {
            return None;
        }
        match self.scopes[self.file].get(enum_name) {
            Some(Sym::Adt(id)) => self.adts[*id]
                .variants()
                .iter()
                .position(|(n, _)| n == variant)
                .map(|v| (*id, v)),
            _ => None,
        }
    }

    /// Finds an enum variant by bare name among visible enums; ambiguous names must be qualified.
    pub fn unqualified_variant(&self, name: &str, span: Span) -> Result<Option<(usize, usize)>> {
        if !name.starts_with(char::is_uppercase) {
            return Ok(None);
        }
        let mut found: Vec<(usize, usize)> = Vec::new();
        for sym in self.scopes[self.file].values() {
            if let Sym::Adt(id) = sym
                && let Some(v) = self.adts[*id]
                    .variants()
                    .iter()
                    .position(|(n, _)| n == name)
            {
                found.push((*id, v));
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(Some(found[0])),
            _ => Err(
                Diag::ty(span, format!("`{name}` is a variant of several enums")).help(format!(
                    "qualify it, e.g. `{}.{name}`",
                    self.adts[found[0].0].name
                )),
            ),
        }
    }

    fn check_variant(
        &mut self,
        adt: usize,
        v: usize,
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Type>,
    ) -> Result<TExpr> {
        let targs: Vec<Type> = (0..self.adts[adt].generics.len())
            .map(|_| self.fresh())
            .collect();
        let ty = Type::Adt(adt, targs.clone());
        if let Some(exp) = expected {
            let _ = self.unify(&ty, exp);
        }
        let map: Vec<(String, Type)> = self.adts[adt]
            .generics
            .iter()
            .cloned()
            .zip(targs.clone())
            .collect();
        let (vname, ftys) = self.adts[adt].variants()[v].clone();
        if args.len() != ftys.len() {
            let what = if ftys.is_empty() {
                format!("`{vname}` takes no values")
            } else {
                format!(
                    "`{vname}` takes {} value(s), found {}",
                    ftys.len(),
                    args.len()
                )
            };
            return Err(Diag::ty(span, what));
        }
        let mut targs_out = Vec::new();
        for (a, ft) in args.iter().zip(&ftys) {
            if let Some(n) = &a.name {
                return Err(Diag::ty(n.span, "variant values are positional"));
            }
            let ft = ft.subst(&map);
            let ta = self.check_expr(&a.value, Some(&ft))?;
            targs_out.push(self.coerce_into(ta, &ft)?);
        }
        self.pending.push((span, ty.clone()));
        Ok(TExpr {
            kind: TK::Variant(adt, targs, v, targs_out),
            ty,
            span,
        })
    }

    fn check_struct_lit(
        &mut self,
        name: &ast::Ident,
        fields: &[(ast::Ident, ast::Expr)],
        base: Option<&ast::Expr>,
        span: Span,
        expected: Option<&Type>,
    ) -> Result<TExpr> {
        let id = match self.scopes[self.file].get(&name.name) {
            Some(Sym::Adt(id)) if matches!(self.adts[*id].kind, AdtKind::Struct(_)) => *id,
            Some(Sym::Adt(_)) => {
                return Err(Diag::ty(
                    name.span,
                    format!("`{}` is an enum, not a struct", name.name),
                ));
            }
            _ => {
                return Err(Diag::ty(
                    name.span,
                    format!("unknown struct `{}`", name.name),
                ));
            }
        };
        let targs: Vec<Type> = (0..self.adts[id].generics.len())
            .map(|_| self.fresh())
            .collect();
        let ty = Type::Adt(id, targs.clone());
        if let Some(exp) = expected {
            let _ = self.unify(&ty, exp);
        }
        let map: Vec<(String, Type)> = self.adts[id]
            .generics
            .iter()
            .cloned()
            .zip(targs.clone())
            .collect();
        let decl = self.adts[id].fields().to_vec();
        let mut given: Vec<Option<TExpr>> = vec![None; decl.len()];
        for (f, e) in fields {
            let Some(i) = decl.iter().position(|(n, _)| *n == f.name) else {
                let mut d = Diag::ty(f.span, format!("`{}` has no field `{}`", name.name, f.name));
                if let Some(s) = suggest(&f.name, decl.iter().map(|(n, _)| n.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                return Err(d);
            };
            if given[i].is_some() {
                return Err(Diag::ty(f.span, format!("field `{}` is set twice", f.name)));
            }
            let ft = decl[i].1.subst(&map);
            let te = self.check_expr(e, Some(&ft))?;
            given[i] = Some(self.coerce_into(te, &ft)?);
        }
        self.pending.push((span, ty.clone()));
        let Some(base) = base else {
            let missing: Vec<&str> = decl
                .iter()
                .zip(&given)
                .filter(|(_, g)| g.is_none())
                .map(|((n, _), _)| n.as_str())
                .collect();
            if !missing.is_empty() {
                return Err(Diag::ty(
                    span,
                    format!(
                        "missing field(s) in `{}`: {}",
                        name.name,
                        missing.join(", ")
                    ),
                ));
            }
            return Ok(TExpr {
                kind: TK::StructNew(id, targs, given.into_iter().map(Option::unwrap).collect()),
                ty,
                span,
            });
        };
        // `Name { ..base, f: v }` evaluates `base` once into a temporary.
        let tb = self.check_expr(base, Some(&ty))?;
        self.coerce(&tb.ty, &ty, base.span)?;
        let tmp = self.bind("$base", ty.clone(), false);
        let vals = given
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                g.unwrap_or_else(|| {
                    let ft = decl[i].1.subst(&map);
                    TExpr {
                        kind: TK::Field(
                            Box::new(TExpr {
                                kind: TK::Local(tmp),
                                ty: ty.clone(),
                                span,
                            }),
                            i,
                        ),
                        ty: ft,
                        span,
                    }
                })
            })
            .collect();
        let new = TExpr {
            kind: TK::StructNew(id, targs, vals),
            ty: ty.clone(),
            span,
        };
        Ok(TExpr {
            kind: TK::Block(vec![TStmt::Let(tmp, tb)], Some(Box::new(new))),
            ty,
            span,
        })
    }

    fn check_call(
        &mut self,
        callee: &ast::Ident,
        type_args: &[ast::TypeExpr],
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Type>,
    ) -> Result<TExpr> {
        let name = callee.name.as_str();
        if name.starts_with("__") {
            if self.file != 0 {
                return Err(Diag::ty(
                    callee.span,
                    format!("`{name}` is an internal standard-library intrinsic"),
                ));
            }
            if let Some(&(iname, params, ret)) = INTRINSICS.iter().find(|i| i.0 == name) {
                return self.check_intrinsic(iname, params, ret, args, span);
            }
        }
        if let Some(f) = self.value_callee(name, callee.span)? {
            return self.call_value(f, args, span);
        }
        match name {
            "navigate" | "copy" if self.ctx != Ctx::Handler => {
                return Err(Diag::ty(
                    callee.span,
                    format!("`{name}` can only be called from an event handler"),
                ));
            }
            "assert" => {
                if args.len() != 2 {
                    return Err(Diag::ty(
                        span,
                        "`assert` takes a Bool condition and a String message",
                    ));
                }
                let c = self.check_expr(&args[0].value, Some(&Type::Bool))?;
                self.coerce(&c.ty, &Type::Bool, args[0].value.span)?;
                let m = self.check_expr(&args[1].value, Some(&Type::Str))?;
                self.coerce(&m.ty, &Type::Str, args[1].value.span)?;
                let panic = TExpr {
                    kind: TK::Intrinsic("__panic", vec![], vec![m]),
                    ty: Type::Never,
                    span,
                };
                let not = TExpr {
                    kind: TK::Not(Box::new(c)),
                    ty: Type::Bool,
                    span,
                };
                let then = TExpr {
                    kind: TK::Block(vec![TStmt::Expr(panic)], None),
                    ty: Type::Never,
                    span,
                };
                return Ok(TExpr {
                    kind: TK::If(Box::new(not), Box::new(then), None),
                    ty: Type::Unit,
                    span,
                });
            }
            "navigate" | "print" | "copy" => {
                if args.len() != 1 {
                    return Err(Diag::ty(span, format!("`{name}` takes exactly one String")));
                }
                let a = self.check_expr(&args[0].value, Some(&Type::Str))?;
                self.coerce(&a.ty, &Type::Str, args[0].value.span)?;
                let iname = match name {
                    "print" => "__print",
                    "copy" => "__copy",
                    _ => "__navigate",
                };
                return Ok(TExpr {
                    kind: TK::Intrinsic(iname, vec![], vec![a]),
                    ty: Type::Unit,
                    span,
                });
            }
            _ => {}
        }
        let user_fn = matches!(self.scopes[self.file].get(name), Some(Sym::Fn(id)) if self.sigs[*id].file != 0);
        if !user_fn && matches!(name, "env" | "load" | "save" | "remove") {
            return self.check_server_builtin(callee, type_args, args, span);
        }
        if !user_fn && name == "to_json" {
            let [arg] = args else {
                return Err(Diag::ty(span, "`to_json` takes one value"));
            };
            let v = self.check_expr(&arg.value, None)?;
            let ty = self.zonk(&v.ty);
            let enc = self.json_encoder(&ty, arg.value.span)?;
            return Ok(TExpr {
                kind: TK::Call(enc, vec![], vec![v]),
                ty: Type::Str,
                span,
            });
        }
        match self.scopes[self.file].get(name).copied() {
            Some(Sym::Fn(id)) => self.call_fn(
                id,
                type_args,
                args.iter().map(|a| a.value.clone()).collect(),
                args,
                span,
                None,
            ),
            Some(Sym::Comp(_)) => Err(Diag::ty(
                callee.span,
                format!("`{name}` is a component and can only be used inside a view"),
            )),
            Some(Sym::Adt(id)) if matches!(self.adts[id].kind, AdtKind::Struct(_)) => {
                Err(Diag::ty(callee.span, format!("`{name}` is a struct"))
                    .help(format!("construct it with `{name} {{ field: value }}`")))
            }
            _ => {
                if let Some((adt, v)) = self.unqualified_variant(name, callee.span)? {
                    return self.check_variant(adt, v, args, span, expected);
                }
                let cands: Vec<String> = self.scopes[self.file]
                    .iter()
                    .filter(|(_, s)| matches!(s, Sym::Fn(_)))
                    .map(|(n, _)| n.clone())
                    .collect();
                let mut d = Diag::ty(callee.span, format!("unknown function `{name}`"));
                if let Some(s) = suggest(
                    name,
                    cands
                        .iter()
                        .map(|s| s.as_str())
                        .chain(["print", "navigate", "copy", "assert"]),
                ) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                Err(d)
            }
        }
    }

    /// `env(name)`, `load<T>(key)`, `save(key, value)` and `remove(key)`: environment variables
    /// and the persistent store, available in server functions only.
    fn check_server_builtin(
        &mut self,
        callee: &ast::Ident,
        type_args: &[ast::TypeExpr],
        args: &[ast::Arg],
        span: Span,
    ) -> Result<TExpr> {
        let name = callee.name.as_str();
        if !self.server {
            return Err(Diag::ty(
                callee.span,
                format!("`{name}` is only available in server functions (`server fn`)"),
            ));
        }
        let arity = if name == "save" { 2 } else { 1 };
        if args.len() != arity {
            return Err(Diag::ty(
                span,
                format!("`{name}` takes {arity} argument(s)"),
            ));
        }
        let key = self.check_expr(&args[0].value, Some(&Type::Str))?;
        self.coerce(&key.ty, &Type::Str, args[0].value.span)?;
        let call = |f: usize, args: Vec<TExpr>, ty: Type| TExpr {
            kind: TK::Call(f, vec![], args),
            ty,
            span,
        };
        let pf = |this: &Self, n: &str| this.prelude_fn(n).expect("server prelude");
        match name {
            "env" => Ok(call(
                pf(self, "server_env"),
                vec![key],
                self.option(Type::Str),
            )),
            "remove" => Ok(call(pf(self, "server_remove"), vec![key], Type::Unit)),
            "save" => {
                let v = self.check_expr(&args[1].value, None)?;
                let ty = self.zonk(&v.ty);
                let enc = self.json_encoder(&ty, args[1].value.span)?;
                let text = call(enc, vec![v], Type::Str);
                Ok(call(
                    pf(self, "server_save_text"),
                    vec![key, text],
                    Type::Unit,
                ))
            }
            _ => {
                let [t] = type_args else {
                    return Err(Diag::ty(
                        span,
                        "`load` needs the stored type: `load<[Todo]>(\"todos\")`",
                    ));
                };
                let t = self.resolve_type(t)?;
                let f = self.store_load_fn(&t, span)?;
                Ok(call(f, vec![key], self.option(t)))
            }
        }
    }

    fn check_intrinsic(
        &mut self,
        name: &'static str,
        params: &[&str],
        ret: &str,
        args: &[ast::Arg],
        span: Span,
    ) -> Result<TExpr> {
        let t = self.fresh();
        let parse = |s: &str| match s {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::Str,
            "Bool" => Type::Bool,
            "()" => Type::Unit,
            "!" => Type::Never,
            "T" => t.clone(),
            "[T]" => Type::Array(Box::new(t.clone())),
            _ => unreachable!(),
        };
        if args.len() != params.len() {
            return Err(Diag::ty(
                span,
                format!("`{name}` takes {} arguments", params.len()),
            ));
        }
        let mut targs = Vec::new();
        for (a, p) in args.iter().zip(params) {
            let pt = parse(p);
            let ta = self.check_expr(&a.value, Some(&pt))?;
            self.coerce(&ta.ty, &pt, a.value.span)?;
            targs.push(ta);
        }
        let generic = params.iter().any(|p| p.contains('T')) || ret.contains('T');
        Ok(TExpr {
            kind: TK::Intrinsic(name, if generic { vec![t.clone()] } else { vec![] }, targs),
            ty: parse(ret),
            span,
        })
    }

    /// Checks a call to function `id`. `recv` is an already-checked first argument (method syntax).
    fn call_fn(
        &mut self,
        id: usize,
        type_args: &[ast::TypeExpr],
        args: Vec<ast::Expr>,
        named: &[ast::Arg],
        span: Span,
        recv: Option<TExpr>,
    ) -> Result<TExpr> {
        if let Some(n) = named.iter().find_map(|a| a.name.as_ref()) {
            return Err(Diag::ty(n.span, "function arguments are positional"));
        }
        let name = &self.sigs[id].name;
        if self.sigs[id].ext && self.server {
            return Err(Diag::ty(
                span,
                format!(
                    "`{name}` is a JavaScript function: it runs in the browser, not in server functions"
                ),
            ));
        }
        let server_only = self.sigs[id].server
            || self.sigs[id].file == 0 && name.starts_with("server_") && self.file != 0;
        if server_only && !self.server {
            let d = Diag::ty(span, format!("`{name}` runs on the server"));
            return Err(if self.sigs[id].server {
                d.help(format!(
                    "call it from an event handler with `fetch {name}(...) {{ Ok(v) => ... Err(e) => ... }}`"
                ))
            } else {
                d
            });
        }
        let sig_generics = self.sigs[id].generics.clone();
        let targs: Vec<Type> = if type_args.is_empty() {
            (0..sig_generics.len()).map(|_| self.fresh()).collect()
        } else {
            if type_args.len() != sig_generics.len() {
                return Err(Diag::ty(
                    span,
                    format!(
                        "`{}` expects {} type argument(s)",
                        self.sigs[id].name,
                        sig_generics.len()
                    ),
                ));
            }
            type_args
                .iter()
                .map(|t| self.resolve_type(t))
                .collect::<Result<_>>()?
        };
        let map: Vec<(String, Type)> = sig_generics.into_iter().zip(targs.clone()).collect();
        let params: Vec<Type> = self.sigs[id].params.iter().map(|p| p.subst(&map)).collect();
        let ret = self.sigs[id].ret.subst(&map);
        let given = args.len() + recv.is_some() as usize;
        if given != params.len() {
            return Err(Diag::ty(
                span,
                format!(
                    "`{}` takes {} argument(s), but {} were given",
                    self.sigs[id].name,
                    params.len(),
                    given
                ),
            ));
        }
        let mut targs_out = Vec::new();
        let mut pit = params.iter();
        if let Some(r) = recv {
            let p = pit.next().unwrap();
            self.coerce(&r.ty, p, r.span)?;
            targs_out.push(r);
        }
        for (a, p) in args.iter().zip(pit) {
            let ta = self.check_expr(a, Some(p))?;
            targs_out.push(self.coerce_into(ta, p)?);
        }
        for t in &targs {
            self.pending.push((span, t.clone()));
        }
        Ok(TExpr {
            kind: TK::Call(id, targs, targs_out),
            ty: ret,
            span,
        })
    }

    /// Checks `e` against `expected`, wrapping a value whose type implements the expected
    /// interface.
    pub fn coerce_into(&mut self, e: TExpr, expected: &Type) -> Result<TExpr> {
        let e = self.box_iface(e, expected);
        self.coerce(&e.ty, expected, e.span)?;
        Ok(e)
    }

    /// `e` as a value of interface `expected` when its type implements it: a struct of
    /// closures calling the type's methods on `e`. Otherwise `e` unchanged.
    fn box_iface(&mut self, e: TExpr, expected: &Type) -> TExpr {
        let (Type::Adt(ty, _), Type::Adt(iface, _)) = (self.zonk(&e.ty), self.zonk(expected))
        else {
            return e;
        };
        let Some((_, _, fns)) = self
            .impls
            .iter()
            .find(|i| i.0 == ty && i.1 == iface)
            .cloned()
        else {
            return e;
        };
        let span = e.span;
        let ex = |kind, ty| TExpr { kind, ty, span };
        let recv_ty = e.ty.clone();
        let new_local = |this: &mut Self, ty: Type| {
            this.locals.push(Local {
                name: "$self".into(),
                ty,
            });
            this.locals.len() - 1
        };
        let tmp = new_local(self, recv_ty.clone());
        let mut closures = Vec::new();
        for ((_, mt), f) in self.adts[iface].fields().to_vec().into_iter().zip(fns) {
            let Type::Fn(ps, r) = mt.clone() else {
                unreachable!()
            };
            let inner = new_local(self, recv_ty.clone());
            let params: Vec<LocalId> = ps.iter().map(|p| new_local(self, p.clone())).collect();
            let args = std::iter::once(ex(TK::Local(inner), recv_ty.clone()))
                .chain(
                    params
                        .iter()
                        .zip(&ps)
                        .map(|(&l, p)| ex(TK::Local(l), p.clone())),
                )
                .collect();
            let body = ex(TK::Call(f, vec![], args), *r);
            let captures = vec![(ex(TK::Local(tmp), recv_ty.clone()), inner)];
            closures.push(ex(
                TK::Lambda(Box::new(TLambda {
                    params,
                    captures,
                    body,
                })),
                mt,
            ));
        }
        let obj = ex(
            TK::StructNew(iface, vec![], closures),
            Type::Adt(iface, vec![]),
        );
        ex(
            TK::Block(vec![TStmt::Let(tmp, e)], Some(Box::new(obj))),
            Type::Adt(iface, vec![]),
        )
    }

    fn check_lambda(
        &mut self,
        params: &[(ast::Ident, Option<ast::TypeExpr>)],
        ret: Option<&ast::TypeExpr>,
        body: &ast::Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> Result<TExpr> {
        let (exp_ps, exp_r) = match expected.map(|t| self.zonk(t)) {
            Some(Type::Fn(ps, r)) if ps.len() == params.len() => (Some(ps), Some(*r)),
            _ => (None, None),
        };
        let mut ptys = Vec::new();
        for (i, (_, t)) in params.iter().enumerate() {
            ptys.push(match (t, &exp_ps) {
                (Some(t), _) => self.resolve_type(t)?,
                (None, Some(ps)) => ps[i].clone(),
                (None, None) => self.fresh(),
            });
        }
        let ret = match (ret, exp_r) {
            (Some(t), _) => self.resolve_type(t)?,
            (None, Some(r)) => r,
            (None, None) => self.fresh(),
        };
        let saved = (
            std::mem::replace(&mut self.ret, ret.clone()),
            std::mem::replace(&mut self.loops, 0),
            std::mem::replace(&mut self.ctx, Ctx::Fn),
        );
        self.lambdas.push(LambdaFrame {
            base: self.local_scopes.len(),
            captures: Vec::new(),
            resumes: false,
        });
        self.local_scopes.push(Vec::new());
        let r = (|| {
            let mut ids = Vec::new();
            for ((name, _), t) in params.iter().zip(&ptys) {
                if ids
                    .iter()
                    .any(|&l: &LocalId| self.locals[l].name == name.name)
                {
                    return Err(Diag::ty(
                        name.span,
                        format!("duplicate parameter `{}`", name.name),
                    ));
                }
                ids.push(self.bind(&name.name, t.clone(), false));
            }
            let b = self.check_expr(body, Some(&ret))?;
            let b = if self.zonk(&ret) == Type::Unit && self.zonk(&b.ty) != Type::Never {
                TExpr {
                    span: b.span,
                    kind: TK::Block(vec![TStmt::Expr(b)], None),
                    ty: Type::Unit,
                }
            } else {
                self.coerce_into(b, &ret)?
            };
            Ok((ids, b))
        })();
        self.local_scopes.pop();
        let frame = self.lambdas.pop().unwrap();
        (self.ret, self.loops, self.ctx) = saved;
        let (params, body) = r?;
        let ty = Type::Fn(ptys, Box::new(ret));
        self.pending.push((span, ty.clone()));
        Ok(TExpr {
            kind: TK::Lambda(Box::new(TLambda {
                params,
                captures: frame.captures,
                body,
            })),
            ty,
            span,
        })
    }

    /// Calls the function value `f`.
    fn call_value(&mut self, f: TExpr, args: &[ast::Arg], span: Span) -> Result<TExpr> {
        let Type::Fn(ps, r) = self.zonk(&f.ty) else {
            return Err(Diag::ty(
                f.span,
                format!("`{}` is not a function", self.show(&f.ty)),
            ));
        };
        if let Some(n) = args.iter().find_map(|a| a.name.as_ref()) {
            return Err(Diag::ty(n.span, "function arguments are positional"));
        }
        if args.len() != ps.len() {
            return Err(Diag::ty(
                span,
                format!(
                    "this function takes {} argument(s), but {} were given",
                    ps.len(),
                    args.len()
                ),
            ));
        }
        let mut out = Vec::new();
        for (a, p) in args.iter().zip(&ps) {
            let ta = self.check_expr(&a.value, Some(p))?;
            out.push(self.coerce_into(ta, p)?);
        }
        Ok(TExpr {
            kind: TK::CallValue(Box::new(f), out),
            ty: *r,
            span,
        })
    }

    /// A local, or a component value not shadowed by a function, that holds a function.
    fn value_callee(&mut self, name: &str, span: Span) -> Result<Option<TExpr>> {
        let is_fn = |this: &Self, t: &Type| matches!(this.zonk(t), Type::Fn(..));
        if let Some((id, _)) = self.lookup_local(name) {
            return Ok(if is_fn(self, &self.locals[id].ty) {
                Some(self.check_ident(name, span, None)?)
            } else {
                None
            });
        }
        if self.ui.is_some()
            && !matches!(self.scopes[self.file].get(name), Some(Sym::Fn(_)))
            && let Some(t) = self.ui_ident(name, span)?
            && is_fn(self, &t.ty)
        {
            return Ok(Some(self.capture(t, 0)));
        }
        Ok(None)
    }

    fn check_method(
        &mut self,
        recv: &ast::Expr,
        name: &ast::Ident,
        type_args: &[ast::TypeExpr],
        args: &[ast::Arg],
        span: Span,
        expected: Option<&Type>,
    ) -> Result<TExpr> {
        if let ExprKind::Ident(q) = &recv.kind
            && let Some((adt, v)) = self.qualified_variant(q, &name.name)
        {
            return self.check_variant(adt, v, args, span, expected);
        }
        let args_e: Vec<ast::Expr> = args.iter().map(|a| a.value.clone()).collect();
        // `Type.function(args)`: a method without `self`.
        if let ExprKind::Ident(q) = &recv.kind
            && self.lookup_local(q).is_none()
            && let Some(Sym::Fn(id)) = self.scopes[self.file]
                .get(&format!("{q}.{}", name.name))
                .copied()
        {
            return self.call_fn(id, type_args, args_e, args, span, None);
        }
        let r = self.check_expr(recv, None)?;
        let rt = self.zonk(&r.ty);
        if let Type::Adt(id, targs) = &rt
            && matches!(self.adts[*id].kind, AdtKind::Struct(_))
        {
            let method = format!("{}.{}", self.adts[*id].name, name.name);
            if let Some(Sym::Fn(f)) = self.scopes[self.file].get(&method).copied() {
                return self.call_fn(f, type_args, args_e, args, span, Some(r));
            }
            let map: Vec<(String, Type)> = self.adts[*id]
                .generics
                .iter()
                .cloned()
                .zip(targs.clone())
                .collect();
            if let Some(i) = self.adts[*id]
                .fields()
                .iter()
                .position(|(n, _)| *n == name.name)
                && let ft @ Type::Fn(..) = self.adts[*id].fields()[i].1.subst(&map)
            {
                let field = TExpr {
                    kind: TK::Field(Box::new(r), i),
                    ty: ft,
                    span: name.span,
                };
                return self.call_value(field, args, span);
            }
        } else if let Type::Adt(id, _) = &rt
            && let Some(Sym::Fn(f)) = self.scopes[self.file]
                .get(&format!("{}.{}", self.adts[*id].name, name.name))
                .copied()
        {
            return self.call_fn(f, type_args, args_e, args, span, Some(r));
        }
        let prefix = match &rt {
            Type::Int => Some("int"),
            Type::Float => Some("float"),
            Type::Bool => Some("bool"),
            Type::Str => Some("str"),
            Type::Array(_) => Some("arr"),
            Type::Adt(id, _) if *id == self.prelude_adt("Option") => Some("opt"),
            Type::Adt(id, _) if *id == self.prelude_adt("Result") => Some("res"),
            Type::Adt(id, _) if *id == self.prelude_adt("HttpError") => Some("http"),
            _ => None,
        };
        if let Some(p) = prefix
            && let Some(id) = self.prelude_fn(&format!("{p}_{}", name.name))
        {
            return self.call_fn(id, type_args, args_e, args, span, Some(r));
        }
        if let Some(Sym::Fn(id)) = self.scopes[self.file].get(&name.name).copied() {
            return self.call_fn(id, type_args, args_e, args, span, Some(r));
        }
        let mut d = Diag::ty(
            name.span,
            format!("no method `{}` on `{}`", name.name, self.show(&rt)),
        );
        if let Some(p) = prefix {
            let pre = format!("{p}_");
            let methods: Vec<String> = self.scopes[0]
                .keys()
                .filter_map(|k| k.strip_prefix(&pre).map(str::to_string))
                .collect();
            if let Some(s) = suggest(&name.name, methods.iter().map(|s| s.as_str())) {
                d = d.help(format!("did you mean `{s}`?"));
            }
        }
        Err(d)
    }

    // ----- patterns -----

    pub fn check_pat(&mut self, p: &ast::Pattern, ty: &Type) -> Result<TPat> {
        let ty = self.zonk(ty);
        let mismatch = |this: &Self, what: &str| {
            Err(Diag::ty(
                p.span,
                format!(
                    "{what} pattern cannot match a value of type `{}`",
                    this.show(&ty)
                ),
            ))
        };
        Ok(match &p.kind {
            PatKind::Wild => TPat::Wild,
            PatKind::Bind(n) => TPat::Bind(self.bind(n, ty.clone(), false)),
            PatKind::Int(n) => {
                if ty != Type::Int {
                    return mismatch(self, "an integer");
                }
                TPat::Int(*n)
            }
            PatKind::Str(s) => {
                if ty != Type::Str {
                    return mismatch(self, "a string");
                }
                TPat::Str(s.clone())
            }
            PatKind::Bool(b) => {
                if ty != Type::Bool {
                    return mismatch(self, "a boolean");
                }
                TPat::Bool(*b)
            }
            PatKind::Variant { qual, name, args } => {
                let Type::Adt(adt, targs) = &ty else {
                    return mismatch(self, &format!("variant `{}`", name.name));
                };
                if let Some(q) = qual
                    && q.name != self.adts[*adt].name
                {
                    return Err(Diag::ty(
                        q.span,
                        format!(
                            "expected a variant of `{}`, found `{}`",
                            self.show(&ty),
                            q.name
                        ),
                    ));
                }
                let Some(v) = self.adts[*adt]
                    .variants()
                    .iter()
                    .position(|(n, _)| *n == name.name)
                else {
                    let mut d = Diag::ty(
                        name.span,
                        format!("`{}` is not a variant of `{}`", name.name, self.show(&ty)),
                    );
                    if let Some(s) = suggest(
                        &name.name,
                        self.adts[*adt].variants().iter().map(|(n, _)| n.as_str()),
                    ) {
                        d = d.help(format!("did you mean `{s}`?"));
                    }
                    return Err(d);
                };
                let map: Vec<(String, Type)> = self.adts[*adt]
                    .generics
                    .iter()
                    .cloned()
                    .zip(targs.clone())
                    .collect();
                let ftys = self.adts[*adt].variants()[v].1.clone();
                if ftys.len() != args.len() {
                    return Err(Diag::ty(
                        p.span,
                        format!(
                            "`{}` has {} field(s), but the pattern has {}",
                            name.name,
                            ftys.len(),
                            args.len()
                        ),
                    ));
                }
                let sub = args
                    .iter()
                    .zip(&ftys)
                    .map(|(a, ft)| self.check_pat(a, &ft.subst(&map)))
                    .collect::<Result<Vec<_>>>()?;
                TPat::Variant(*adt, targs.clone(), v, sub)
            }
        })
    }

    pub fn check_exhaustive(&self, ty: &Type, arms: &[(TPat, TExpr)], span: Span) -> Result<()> {
        let rows: Vec<Vec<&TPat>> = arms.iter().map(|(p, _)| vec![p]).collect();
        if let Some(w) = self.witness(&rows, std::slice::from_ref(ty)) {
            return Err(Diag::ty(
                span,
                format!("non-exhaustive match: `{}` is not covered", w[0]),
            )
            .help("add an arm for it, or a `_ => ...` arm"));
        }
        Ok(())
    }

    /// Returns a value pattern (one string per column) not matched by any row, if one exists.
    fn witness(&self, rows: &[Vec<&TPat>], tys: &[Type]) -> Option<Vec<String>> {
        if tys.is_empty() {
            return if rows.is_empty() { Some(vec![]) } else { None };
        }
        // Constructors of the first column's type: (label, field types) when finite.
        let ctors: Option<Vec<(String, Vec<Type>)>> = match &tys[0] {
            Type::Bool => Some(vec![("true".into(), vec![]), ("false".into(), vec![])]),
            Type::Adt(id, targs) if !self.adts[*id].variants().is_empty() => {
                let map: Vec<(String, Type)> = self.adts[*id]
                    .generics
                    .iter()
                    .cloned()
                    .zip(targs.clone())
                    .collect();
                Some(
                    self.adts[*id]
                        .variants()
                        .iter()
                        .map(|(n, f)| (n.clone(), f.iter().map(|t| t.subst(&map)).collect()))
                        .collect(),
                )
            }
            _ => None,
        };
        let head_matches = |p: &TPat, ci: usize, label: &str| match p {
            TPat::Bool(b) => b.to_string() == label,
            TPat::Variant(_, _, v, _) => *v == ci,
            _ => false,
        };
        if let Some(ctors) = &ctors {
            let used = |ci: usize, label: &str| rows.iter().any(|r| head_matches(r[0], ci, label));
            let complete = ctors.iter().enumerate().all(|(ci, (l, _))| used(ci, l));
            if complete {
                for (ci, (label, ftys)) in ctors.iter().enumerate() {
                    let wild = TPat::Wild;
                    let spec: Vec<Vec<&TPat>> = rows
                        .iter()
                        .filter_map(|r| {
                            let mut out: Vec<&TPat> = match r[0] {
                                TPat::Wild | TPat::Bind(_) => ftys.iter().map(|_| &wild).collect(),
                                p if head_matches(p, ci, label) => match p {
                                    TPat::Variant(_, _, _, sub) => sub.iter().collect(),
                                    _ => vec![],
                                },
                                _ => return None,
                            };
                            out.extend_from_slice(&r[1..]);
                            Some(out)
                        })
                        .collect();
                    let tys2: Vec<Type> = ftys
                        .iter()
                        .cloned()
                        .chain(tys[1..].iter().cloned())
                        .collect();
                    if let Some(w) = self.witness(&spec, &tys2) {
                        let (fields, rest) = w.split_at(ftys.len());
                        let head = if fields.is_empty() {
                            label.clone()
                        } else {
                            format!("{label}({})", fields.join(", "))
                        };
                        return Some(std::iter::once(head).chain(rest.iter().cloned()).collect());
                    }
                }
                return None;
            }
        }
        let default: Vec<Vec<&TPat>> = rows
            .iter()
            .filter(|r| matches!(r[0], TPat::Wild | TPat::Bind(_)))
            .map(|r| r[1..].to_vec())
            .collect();
        let w = self.witness(&default, &tys[1..])?;
        let head = match &ctors {
            Some(ctors) => {
                let (ci, (label, ftys)) = ctors
                    .iter()
                    .enumerate()
                    .find(|(ci, (l, _))| !rows.iter().any(|r| head_matches(r[0], *ci, l)))
                    .unwrap();
                let _ = ci;
                if ftys.is_empty() {
                    label.clone()
                } else {
                    format!("{label}({})", vec!["_"; ftys.len()].join(", "))
                }
            }
            None => "_".into(),
        };
        Some(std::iter::once(head).chain(w).collect())
    }
}

fn place_read(p: &Place) -> TK {
    match p {
        Place::Local(id) => TK::Local(*id),
        Place::CompVar(i) => TK::CompVar(*i),
    }
}

fn stmt_diverges(s: &TStmt) -> bool {
    match s {
        TStmt::Let(_, e) | TStmt::Expr(e) | TStmt::Assign(_, e) => e.ty == Type::Never,
        _ => false,
    }
}

fn has_var(t: &Type) -> bool {
    match t {
        Type::Var(_) => true,
        Type::Fn(ps, r) => ps.iter().any(has_var) || has_var(r),
        Type::Array(t) => has_var(t),
        Type::Adt(_, a) => a.iter().any(has_var),
        _ => false,
    }
}

/// Unconstrained variables (e.g. the element type of an array that is never used) default to `()`.
fn default_vars(t: &Type) -> Type {
    match t {
        Type::Var(_) => Type::Unit,
        Type::Fn(ps, r) => Type::Fn(
            ps.iter().map(default_vars).collect(),
            Box::new(default_vars(r)),
        ),
        Type::Array(t) => Type::Array(Box::new(default_vars(t))),
        Type::Adt(id, a) => Type::Adt(*id, a.iter().map(default_vars).collect()),
        _ => t.clone(),
    }
}

pub fn names(ids: &[ast::Ident]) -> Vec<String> {
    ids.iter().map(|i| i.name.clone()).collect()
}

/// Closest candidate by edit distance, if reasonably close.
pub fn suggest<'b>(name: &str, cands: impl Iterator<Item = &'b str>) -> Option<String> {
    fn dist(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut cur = vec![i + 1];
            for (j, cb) in b.iter().enumerate() {
                cur.push(
                    (prev[j] + (ca != *cb) as usize)
                        .min(prev[j + 1] + 1)
                        .min(cur[j] + 1),
                );
            }
            prev = cur;
        }
        prev[b.len()]
    }
    cands
        .filter(|c| *c != name)
        .map(|c| (dist(name, c), c))
        .filter(|(d, c)| *d <= 2.max(c.len() / 3) && *d < name.len().max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}
