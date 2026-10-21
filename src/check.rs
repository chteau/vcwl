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

pub struct FnSig {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<Type>,
    pub ret: Type,
    pub file: usize,
    pub item: usize,
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
}

const INTRINSICS: &[(&str, &[&str], &str)] = &[
    ("__str_len", &["String"], "Int"),
    ("__str_byte", &["String", "Int"], "Int"),
    ("__str_alloc", &["Int"], "String"),
    ("__str_set_byte", &["String", "Int", "Int"], "()"),
    ("__str_copy", &["String", "Int", "String", "Int", "Int"], "()"),
    ("__arr_len", &["[T]"], "Int"),
    ("__arr_alloc", &["Int", "T"], "[T]"),
    ("__arr_set", &["[T]", "Int", "T"], "()"),
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
                        self.adts.push(Adt { name: s.name.name.clone(), generics: names(&s.generics), kind: AdtKind::Struct(vec![]) });
                        (Sym::Adt(self.adts.len() - 1), &s.name, s.exported)
                    }
                    ast::Item::Enum(e) => {
                        self.adts.push(Adt { name: e.name.name.clone(), generics: names(&e.generics), kind: AdtKind::Enum(vec![]) });
                        (Sym::Adt(self.adts.len() - 1), &e.name, e.exported)
                    }
                    ast::Item::Fn(f) => {
                        self.sigs.push(FnSig { name: f.name.name.clone(), generics: names(&f.generics), params: vec![], ret: Type::Unit, file: fi, item: ii });
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
                    self.diags.push(Diag::ty(name.span, format!("`{}` is defined more than once in this file", name.name)));
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
                                return Err(Diag::ty(n.span, format!("duplicate field `{}`", n.name)));
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
                                return Err(Diag::ty(v.name.span, format!("duplicate variant `{}`", v.name.name)));
                            }
                            let fs = v.fields.iter().map(|t| self.resolve_type(t)).collect::<Result<Vec<_>>>()?;
                            vs.push((v.name.name.clone(), fs));
                        }
                        if vs.is_empty() {
                            return Err(Diag::ty(e.name.span, "an enum needs at least one variant"));
                        }
                        self.adts[id].kind = AdtKind::Enum(vs);
                    }
                    (ast::Item::Fn(f), Sym::Fn(id)) => {
                        self.generics = names(&f.generics);
                        let params = f.params.iter().map(|p| self.resolve_type(&p.ty)).collect::<Result<Vec<_>>>()?;
                        let ret = match &f.ret {
                            Some(t) => self.resolve_type(t)?,
                            None => Type::Unit,
                        };
                        if f.name.name.starts_with("__") && fi != 0 {
                            return Err(Diag::ty(f.name.span, "names starting with `__` are reserved for the standard library"));
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
        self.generics.clear();
    }

    pub fn check_fns(&mut self) {
        for id in 0..self.sigs.len() {
            let (fi, ii) = (self.sigs[id].file, self.sigs[id].item);
            let ast::Item::Fn(f) = &self.files[fi].items[ii] else { unreachable!() };
            match self.check_fn(id, f) {
                Ok(tf) => self.fns[id] = Some(tf),
                Err(d) => self.diags.push(d),
            }
        }
    }

    fn check_fn(&mut self, id: usize, f: &ast::FnDecl) -> Result<TFn> {
        self.file = self.sigs[id].file;
        self.begin_body(names(&f.generics), self.sigs[id].ret.clone(), Ctx::Fn);
        let mut params = Vec::new();
        for (p, ty) in f.params.iter().zip(self.sigs[id].params.clone()) {
            if params.iter().any(|&l: &LocalId| self.locals[l].name == p.name.name) {
                return Err(Diag::ty(p.name.span, format!("duplicate parameter `{}`", p.name.name)));
            }
            params.push(self.bind(&p.name.name, ty, false));
        }
        let ret = self.ret.clone();
        let body = self.check_block(&f.body, Some(&ret))?;
        if ret != Type::Unit {
            self.coerce(&body.ty, &ret, f.body.tail.as_ref().map(|t| t.span).unwrap_or(f.body.span))
                .map_err(|d| if self.zonk(&body.ty) == Type::Unit {
                    Diag::ty(f.name.span, format!("function `{}` must return a value of type `{}` on every path", f.name.name, self.show(&ret)))
                        .help("end the body with the value to return, or add `return value`")
                } else { d })?;
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
                return Err(Diag::ty(span, format!("cannot infer the type of this expression (`{}`)", self.show(&z)))
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
            TK::Call(_, ts, args) | TK::Intrinsic(_, ts, args) | TK::StructNew(_, ts, args) | TK::Variant(_, ts, _, args) => {
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
                        return Err(Diag::ty(t.span, format!("`{name}` does not take type arguments")));
                    }
                    return Ok(p);
                }
                if self.generics.contains(name) {
                    if !args.is_empty() {
                        return Err(Diag::ty(t.span, "type parameters cannot take type arguments"));
                    }
                    return Ok(Type::Param(name.clone()));
                }
                match self.scopes[self.file].get(name) {
                    Some(Sym::Adt(id)) => {
                        let id = *id;
                        let want = self.adts[id].generics.len();
                        if args.len() != want {
                            return Err(Diag::ty(t.span, format!("`{name}` expects {want} type argument(s), found {}", args.len())));
                        }
                        let args = args.iter().map(|a| self.resolve_type(a)).collect::<Result<Vec<_>>>()?;
                        Type::Adt(id, args)
                    }
                    _ => {
                        let mut d = Diag::ty(t.span, format!("unknown type `{name}`"));
                        let cands: Vec<String> = ["Int", "Float", "Bool", "String"].iter().map(|s| s.to_string())
                            .chain(self.scopes[self.file].iter().filter(|(_, s)| matches!(s, Sym::Adt(_))).map(|(n, _)| n.clone()))
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
            Type::Adt(id, args) if self.adts[id].name == "Option" && id == self.prelude_adt("Option") => format!("{}?", self.show(&args[0])),
            Type::Adt(id, args) if args.is_empty() => self.adts[id].name.clone(),
            Type::Adt(id, args) => format!("{}<{}>", self.adts[id].name, args.iter().map(|a| self.show(a)).collect::<Vec<_>>().join(", ")),
            Type::Param(n) => n,
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
            _ => t.clone(),
        }
    }

    fn occurs(&self, v: u32, t: &Type) -> bool {
        match self.zonk(t) {
            Type::Var(w) => w == v,
            Type::Array(t) => self.occurs(v, &t),
            Type::Adt(_, args) => args.iter().any(|a| self.occurs(v, a)),
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
            (Type::Adt(i, xs), Type::Adt(j, ys)) => i == j && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y)),
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
            (Type::Float, Type::Int) => d.help("convert explicitly with `x.to_int()` (truncates) or `x.round().to_int()`"),
            (Type::Int | Type::Float | Type::Bool, Type::Str) => d.help("convert explicitly with `x.to_string()` or use interpolation: \"{x}\""),
            (_, Type::Adt(id, args)) if *id == self.prelude_adt("Option") && self.zonk(&args[0]) == za => d.help("wrap the value: `Some(value)`"),
            (Type::Adt(id, args), _) if *id == self.prelude_adt("Option") && self.zonk(&args[0]) == ze => {
                d.help("the value may be absent; handle it with `match` or `.unwrap_or(default)`")
            }
            _ => d,
        };
        Err(d)
    }

    // ----- scopes -----

    pub fn bind(&mut self, name: &str, ty: Type, mutable: bool) -> LocalId {
        self.locals.push(Local { name: name.to_string(), ty });
        let id = self.locals.len() - 1;
        self.local_scopes.last_mut().unwrap().push((name.to_string(), id, mutable));
        id
    }

    fn lookup_local(&self, name: &str) -> Option<(LocalId, bool)> {
        self.local_scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(n, _, _)| n == name).map(|&(_, id, m)| (id, m))
    }

    fn visible_value_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.local_scopes.iter().flatten().map(|(n, _, _)| n.clone()).collect();
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
        for s in &b.stmts {
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
        Ok(TExpr { kind: TK::Block(stmts, tail), ty, span: b.span })
    }

    pub fn check_stmt(&mut self, s: &Stmt) -> Result<TStmt> {
        Ok(match s {
            Stmt::Let { name, mutable, ty, value, .. } => {
                let ann = match ty {
                    Some(t) => Some(self.resolve_type(t)?),
                    None => None,
                };
                let v = self.check_expr(value, ann.as_ref())?;
                if let Some(a) = &ann {
                    self.coerce(&v.ty, a, value.span)?;
                }
                let ty = ann.unwrap_or_else(|| v.ty.clone());
                if self.zonk(&ty) == Type::Unit {
                    return Err(Diag::ty(value.span, "cannot bind a value of type `()`"));
                }
                self.pending.push((value.span, ty.clone()));
                let id = self.bind(&name.name, ty, *mutable);
                TStmt::Let(id, v)
            }
            Stmt::Assign { target, op, value, span } => {
                let (place, ty) = self.check_place(target)?;
                let v = match op {
                    None => self.check_expr(value, Some(&ty))?,
                    Some(op) => {
                        let cur = TExpr { kind: place_read(&place), ty: ty.clone(), span: target.span };
                        let rhs = self.check_expr(value, Some(&ty))?;
                        self.binary(*op, cur, rhs, *span)?
                    }
                };
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
            Stmt::For { var, iter, body, .. } => {
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
                            return Err(Diag::ty(iter.span, format!("cannot iterate over `{}`", self.show(&t)))
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
                if matches!(s, Stmt::Break(_)) { TStmt::Break } else { TStmt::Continue }
            }
            Stmt::Fetch(f) => self.check_fetch(f)?,
            Stmt::Emit { name, value, span } => self.check_emit(name, value.as_ref(), *span)?,
        })
    }

    fn check_place(&mut self, target: &ast::Expr) -> Result<(Place, Type)> {
        match &target.kind {
            ExprKind::Ident(name) => {
                if let Some((id, mutable)) = self.lookup_local(name) {
                    if !mutable {
                        return Err(Diag::ty(target.span, format!("cannot assign to `{name}`: it is immutable"))
                            .help(format!("declare it with `var {name} = ...` to make it mutable")));
                    }
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
        let mk = |kind, ty| TExpr { kind, ty, span: e.span };
        Ok(match &e.kind {
            ExprKind::Int(n) => mk(TK::Int(*n), Type::Int),
            ExprKind::Float(n) => mk(TK::Float(*n), Type::Float),
            ExprKind::Bool(b) => mk(TK::Bool(*b), Type::Bool),
            ExprKind::Unit => mk(TK::Unit, Type::Unit),
            ExprKind::Str(parts) => self.check_interp(parts, e.span)?,
            ExprKind::Dim(n, u) => {
                return Err(Diag::ty(e.span, format!("`{n}{u}` is a style value and can only be used in `style` blocks")));
            }
            ExprKind::Color(c) => return Err(Diag::ty(e.span, format!("`#{c}` is a color and can only be used in `style` blocks"))),
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
                        return Err(Diag::ty(e.span, format!("cannot apply `{sym}` to `{}`", self.show(&t))));
                    }
                }
            }
            ExprKind::Call { callee, type_args, args } => self.check_call(callee, type_args, args, e.span, expected)?,
            ExprKind::Method { recv, name, type_args, args } => self.check_method(recv, name, type_args, args, e.span, expected)?,
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
                            let mut d = Diag::ty(name.span, format!("`{}` has no field `{}`", self.adts[id].name, name.name));
                            if let Some(s) = suggest(&name.name, fields.iter().map(|(n, _)| n.as_str())) {
                                d = d.help(format!("did you mean `{s}`?"));
                            }
                            return Err(d);
                        };
                        let map: Vec<(String, Type)> = self.adts[id].generics.iter().cloned().zip(targs).collect();
                        let ft = fields[i].1.subst(&map);
                        mk(TK::Field(Box::new(r), i), ft)
                    }
                    t => {
                        let mut d = Diag::ty(name.span, format!("`{}` has no field `{}`", self.show(&t), name.name));
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
                    t => return Err(Diag::ty(a.span, format!("cannot index into `{}`", self.show(&t)))),
                };
                mk(TK::Index(Box::new(ta), Box::new(ti)), elem)
            }
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
                        let hint = if self.zonk(&t.ty) == Type::Never { expected.cloned() } else { Some(t.ty.clone()) };
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
                            self.coerce(&b.ty, &t, arm.body.span)
                                .map_err(|d| Diag::ty(d.span, format!("match arms have incompatible types: {}", d.msg)))?;
                        }
                        None => {}
                    }
                    tarms.push((p, b));
                }
                self.check_exhaustive(&sty, &tarms, e.span)?;
                mk(TK::Match(Box::new(s), tarms), ty.unwrap_or(Type::Never))
            }
            ExprKind::StructLit { name, fields, base } => self.check_struct_lit(name, fields, base.as_deref(), e.span, expected)?,
            ExprKind::Array(items) => {
                let elem = match expected.map(|t| self.zonk(t)) {
                    Some(Type::Array(t)) => *t,
                    _ => self.fresh(),
                };
                let mut out = Vec::new();
                for it in items {
                    let t = self.check_expr(it, Some(&elem))?;
                    self.coerce(&t.ty, &elem, it.span)
                        .map_err(|d| Diag::ty(d.span, format!("array elements must all have the same type: {}", d.msg)))?;
                    out.push(t);
                }
                let ty = Type::Array(Box::new(elem));
                self.pending.push((e.span, ty.clone()));
                mk(TK::Array(out), ty)
            }
            ExprKind::Range(..) => return Err(Diag::ty(e.span, "ranges can only be used in `for` loops")),
            ExprKind::Return(v) => {
                if self.ctx == Ctx::Render {
                    return Err(Diag::ty(e.span, "`return` is not allowed in view expressions"));
                }
                let ret = self.ret.clone();
                let tv = match v {
                    Some(v) => {
                        let tv = self.check_expr(v, Some(&ret))?;
                        self.coerce(&tv.ty, &ret, v.span)?;
                        Some(Box::new(tv))
                    }
                    None => {
                        if ret != Type::Unit {
                            return Err(Diag::ty(e.span, format!("this function must return a value of type `{}`", self.show(&ret))));
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
        self.coerce(&b.ty, &a.ty, span)
            .map_err(|d| Diag::ty(d.span, format!("`if` and `else` have incompatible types: {}", d.msg)))?;
        Ok(a.ty.clone())
    }

    fn check_interp(&mut self, parts: &[ast::StrPart], span: Span) -> Result<TExpr> {
        let mut pieces = Vec::new();
        for p in parts {
            match p {
                ast::StrPart::Lit(s) => pieces.push(TExpr { kind: TK::Str(s.clone()), ty: Type::Str, span }),
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
            acc = TExpr { kind: TK::Call(f, vec![], vec![acc, p]), ty: Type::Str, span };
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
                return Err(Diag::ty(span, format!("cannot interpolate a value of type `{}`", self.show(&ty)))
                    .help("only String, Int, Float and Bool can be interpolated; convert the value first"));
            }
        };
        let f = self.prelude_fn(f).unwrap();
        Ok(TExpr { kind: TK::Call(f, vec![], vec![t]), ty: Type::Str, span })
    }

    pub fn binary(&mut self, op: BinOp, a: TExpr, b: TExpr, span: Span) -> Result<TExpr> {
        let (ta, tb) = (self.zonk(&a.ty), self.zonk(&b.ty));
        let err = |this: &Self, extra: Option<&str>| {
            let mut d = Diag::ty(span, format!("cannot apply `{}` to `{}` and `{}`", op.symbol(), this.show(&ta), this.show(&tb)));
            if let Some(h) = extra {
                d = d.help(h);
            }
            Err(d)
        };
        if ta != tb && ta != Type::Never && tb != Type::Never {
            let help = match (&ta, &tb) {
                (Type::Str, Type::Int | Type::Float | Type::Bool) | (Type::Int | Type::Float | Type::Bool, Type::Str) => {
                    Some("convert explicitly with `.to_string()`, or use interpolation: \"{a}{b}\"")
                }
                (Type::Int, Type::Float) | (Type::Float, Type::Int) => Some("Int and Float do not mix implicitly; use `.to_float()` or `.to_int()`"),
                _ => None,
            };
            return err(self, help);
        }
        let t = if ta == Type::Never { tb.clone() } else { ta.clone() };
        use BinOp::*;
        let ty = match op {
            Add if t == Type::Str => {
                let f = self.prelude_fn("str_concat").unwrap();
                return Ok(TExpr { kind: TK::Call(f, vec![], vec![a, b]), ty: Type::Str, span });
            }
            Add | Sub | Mul | Div | Rem if matches!(t, Type::Int | Type::Float) => t.clone(),
            Lt | Le | Gt | Ge if matches!(t, Type::Int | Type::Float) => Type::Bool,
            Eq | Ne if t == Type::Str => {
                let f = self.prelude_fn("str_eq").unwrap();
                let call = TExpr { kind: TK::Call(f, vec![], vec![a, b]), ty: Type::Bool, span };
                return Ok(if op == Ne { TExpr { kind: TK::Not(Box::new(call)), ty: Type::Bool, span } } else { call });
            }
            Eq | Ne if matches!(t, Type::Int | Type::Float | Type::Bool) => Type::Bool,
            Eq | Ne if matches!(&t, Type::Adt(id, _) if self.adts[*id].variants().iter().all(|v| v.1.is_empty()) && !self.adts[*id].variants().is_empty()) => Type::Bool,
            Eq | Ne => {
                return err(self, Some("`==` works on Int, Float, Bool, String and enums without payloads; use `match` for other values"));
            }
            And | Or if t == Type::Bool => Type::Bool,
            _ => return err(self, None),
        };
        Ok(TExpr { kind: TK::Bin(op, Box::new(a), Box::new(b)), ty, span })
    }

    fn check_ident(&mut self, name: &str, span: Span, expected: Option<&Type>) -> Result<TExpr> {
        if let Some((id, _)) = self.lookup_local(name) {
            return Ok(TExpr { kind: TK::Local(id), ty: self.locals[id].ty.clone(), span });
        }
        if self.ui.is_some()
            && let Some(t) = self.ui_ident(name, span)?
        {
            return Ok(t);
        }
        if let Some((adt, v)) = self.unqualified_variant(name, span)? {
            return self.check_variant(adt, v, &[], span, expected);
        }
        let mut d = Diag::ty(span, format!("unknown name `{name}`"));
        if matches!(self.scopes[self.file].get(name), Some(Sym::Fn(_))) {
            d = d.help(format!("`{name}` is a function; call it with `{name}(...)`"));
        } else if let Some(s) = suggest(name, self.visible_value_names().iter().map(|s| s.as_str())) {
            d = d.help(format!("did you mean `{s}`?"));
        }
        Err(d)
    }

    fn qualified_variant(&self, enum_name: &str, variant: &str) -> Option<(usize, usize)> {
        if self.lookup_local(enum_name).is_some() {
            return None;
        }
        match self.scopes[self.file].get(enum_name) {
            Some(Sym::Adt(id)) => self.adts[*id].variants().iter().position(|(n, _)| n == variant).map(|v| (*id, v)),
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
                && let Some(v) = self.adts[*id].variants().iter().position(|(n, _)| n == name)
            {
                found.push((*id, v));
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(Some(found[0])),
            _ => Err(Diag::ty(span, format!("`{name}` is a variant of several enums"))
                .help(format!("qualify it, e.g. `{}.{name}`", self.adts[found[0].0].name))),
        }
    }

    fn check_variant(&mut self, adt: usize, v: usize, args: &[ast::Arg], span: Span, expected: Option<&Type>) -> Result<TExpr> {
        let targs: Vec<Type> = (0..self.adts[adt].generics.len()).map(|_| self.fresh()).collect();
        let ty = Type::Adt(adt, targs.clone());
        if let Some(exp) = expected {
            let _ = self.unify(&ty, exp);
        }
        let map: Vec<(String, Type)> = self.adts[adt].generics.iter().cloned().zip(targs.clone()).collect();
        let (vname, ftys) = self.adts[adt].variants()[v].clone();
        if args.len() != ftys.len() {
            let what = if ftys.is_empty() { format!("`{vname}` takes no values") } else { format!("`{vname}` takes {} value(s), found {}", ftys.len(), args.len()) };
            return Err(Diag::ty(span, what));
        }
        let mut targs_out = Vec::new();
        for (a, ft) in args.iter().zip(&ftys) {
            if let Some(n) = &a.name {
                return Err(Diag::ty(n.span, "variant values are positional"));
            }
            let ft = ft.subst(&map);
            let ta = self.check_expr(&a.value, Some(&ft))?;
            self.coerce(&ta.ty, &ft, a.value.span)?;
            targs_out.push(ta);
        }
        self.pending.push((span, ty.clone()));
        Ok(TExpr { kind: TK::Variant(adt, targs, v, targs_out), ty, span })
    }

    fn check_struct_lit(&mut self, name: &ast::Ident, fields: &[(ast::Ident, ast::Expr)], base: Option<&ast::Expr>, span: Span, expected: Option<&Type>) -> Result<TExpr> {
        let id = match self.scopes[self.file].get(&name.name) {
            Some(Sym::Adt(id)) if matches!(self.adts[*id].kind, AdtKind::Struct(_)) => *id,
            Some(Sym::Adt(_)) => return Err(Diag::ty(name.span, format!("`{}` is an enum, not a struct", name.name))),
            _ => return Err(Diag::ty(name.span, format!("unknown struct `{}`", name.name))),
        };
        let targs: Vec<Type> = (0..self.adts[id].generics.len()).map(|_| self.fresh()).collect();
        let ty = Type::Adt(id, targs.clone());
        if let Some(exp) = expected {
            let _ = self.unify(&ty, exp);
        }
        let map: Vec<(String, Type)> = self.adts[id].generics.iter().cloned().zip(targs.clone()).collect();
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
            self.coerce(&te.ty, &ft, e.span)?;
            given[i] = Some(te);
        }
        self.pending.push((span, ty.clone()));
        let Some(base) = base else {
            let missing: Vec<&str> = decl.iter().zip(&given).filter(|(_, g)| g.is_none()).map(|((n, _), _)| n.as_str()).collect();
            if !missing.is_empty() {
                return Err(Diag::ty(span, format!("missing field(s) in `{}`: {}", name.name, missing.join(", "))));
            }
            return Ok(TExpr { kind: TK::StructNew(id, targs, given.into_iter().map(Option::unwrap).collect()), ty, span });
        };
        // `Name { ..base, f: v }` evaluates `base` once into a temporary.
        let tb = self.check_expr(base, Some(&ty))?;
        self.coerce(&tb.ty, &ty, base.span)?;
        let tmp = self.bind("$base", ty.clone(), false);
        let vals = given.into_iter().enumerate().map(|(i, g)| g.unwrap_or_else(|| {
            let ft = decl[i].1.subst(&map);
            TExpr { kind: TK::Field(Box::new(TExpr { kind: TK::Local(tmp), ty: ty.clone(), span }), i), ty: ft, span }
        })).collect();
        let new = TExpr { kind: TK::StructNew(id, targs, vals), ty: ty.clone(), span };
        Ok(TExpr { kind: TK::Block(vec![TStmt::Let(tmp, tb)], Some(Box::new(new))), ty, span })
    }

    fn check_call(&mut self, callee: &ast::Ident, type_args: &[ast::TypeExpr], args: &[ast::Arg], span: Span, expected: Option<&Type>) -> Result<TExpr> {
        let name = callee.name.as_str();
        if name.starts_with("__") {
            if self.file != 0 {
                return Err(Diag::ty(callee.span, format!("`{name}` is an internal standard-library intrinsic")));
            }
            if let Some(&(iname, params, ret)) = INTRINSICS.iter().find(|i| i.0 == name) {
                return self.check_intrinsic(iname, params, ret, args, span);
            }
        }
        match name {
            "navigate" if self.ctx != Ctx::Handler => {
                return Err(Diag::ty(callee.span, "`navigate` can only be called from an event handler"));
            }
            "assert" => {
                if args.len() != 2 {
                    return Err(Diag::ty(span, "`assert` takes a Bool condition and a String message"));
                }
                let c = self.check_expr(&args[0].value, Some(&Type::Bool))?;
                self.coerce(&c.ty, &Type::Bool, args[0].value.span)?;
                let m = self.check_expr(&args[1].value, Some(&Type::Str))?;
                self.coerce(&m.ty, &Type::Str, args[1].value.span)?;
                let panic = TExpr { kind: TK::Intrinsic("__panic", vec![], vec![m]), ty: Type::Never, span };
                let not = TExpr { kind: TK::Not(Box::new(c)), ty: Type::Bool, span };
                let then = TExpr { kind: TK::Block(vec![TStmt::Expr(panic)], None), ty: Type::Never, span };
                return Ok(TExpr { kind: TK::If(Box::new(not), Box::new(then), None), ty: Type::Unit, span });
            }
            "navigate" | "print" => {
                if args.len() != 1 {
                    return Err(Diag::ty(span, format!("`{name}` takes exactly one String")));
                }
                let a = self.check_expr(&args[0].value, Some(&Type::Str))?;
                self.coerce(&a.ty, &Type::Str, args[0].value.span)?;
                let iname = if name == "print" { "__print" } else { "__navigate" };
                return Ok(TExpr { kind: TK::Intrinsic(iname, vec![], vec![a]), ty: Type::Unit, span });
            }
            _ => {}
        }
        match self.scopes[self.file].get(name).copied() {
            Some(Sym::Fn(id)) => self.call_fn(id, type_args, args.iter().map(|a| a.value.clone()).collect(), args, span, None),
            Some(Sym::Comp(_)) => Err(Diag::ty(callee.span, format!("`{name}` is a component and can only be used inside a view"))),
            Some(Sym::Adt(id)) if matches!(self.adts[id].kind, AdtKind::Struct(_)) => Err(Diag::ty(callee.span, format!("`{name}` is a struct"))
                .help(format!("construct it with `{name} {{ field: value }}`"))),
            _ => {
                if let Some((adt, v)) = self.unqualified_variant(name, callee.span)? {
                    return self.check_variant(adt, v, args, span, expected);
                }
                let cands: Vec<String> = self.scopes[self.file].iter().filter(|(_, s)| matches!(s, Sym::Fn(_))).map(|(n, _)| n.clone()).collect();
                let mut d = Diag::ty(callee.span, format!("unknown function `{name}`"));
                if let Some(s) = suggest(name, cands.iter().map(|s| s.as_str()).chain(["print", "navigate", "assert"])) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                Err(d)
            }
        }
    }

    fn check_intrinsic(&mut self, name: &'static str, params: &[&str], ret: &str, args: &[ast::Arg], span: Span) -> Result<TExpr> {
        let t = self.fresh();
        let parse = |s: &str| match s {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::Str,
            "()" => Type::Unit,
            "!" => Type::Never,
            "T" => t.clone(),
            "[T]" => Type::Array(Box::new(t.clone())),
            _ => unreachable!(),
        };
        if args.len() != params.len() {
            return Err(Diag::ty(span, format!("`{name}` takes {} arguments", params.len())));
        }
        let mut targs = Vec::new();
        for (a, p) in args.iter().zip(params) {
            let pt = parse(p);
            let ta = self.check_expr(&a.value, Some(&pt))?;
            self.coerce(&ta.ty, &pt, a.value.span)?;
            targs.push(ta);
        }
        let generic = params.iter().any(|p| p.contains('T')) || ret.contains('T');
        Ok(TExpr { kind: TK::Intrinsic(name, if generic { vec![t.clone()] } else { vec![] }, targs), ty: parse(ret), span })
    }

    /// Checks a call to function `id`. `recv` is an already-checked first argument (method syntax).
    fn call_fn(&mut self, id: usize, type_args: &[ast::TypeExpr], args: Vec<ast::Expr>, named: &[ast::Arg], span: Span, recv: Option<TExpr>) -> Result<TExpr> {
        if let Some(n) = named.iter().find_map(|a| a.name.as_ref()) {
            return Err(Diag::ty(n.span, "function arguments are positional"));
        }
        let sig_generics = self.sigs[id].generics.clone();
        let targs: Vec<Type> = if type_args.is_empty() {
            (0..sig_generics.len()).map(|_| self.fresh()).collect()
        } else {
            if type_args.len() != sig_generics.len() {
                return Err(Diag::ty(span, format!("`{}` expects {} type argument(s)", self.sigs[id].name, sig_generics.len())));
            }
            type_args.iter().map(|t| self.resolve_type(t)).collect::<Result<_>>()?
        };
        let map: Vec<(String, Type)> = sig_generics.into_iter().zip(targs.clone()).collect();
        let params: Vec<Type> = self.sigs[id].params.iter().map(|p| p.subst(&map)).collect();
        let ret = self.sigs[id].ret.subst(&map);
        let given = args.len() + recv.is_some() as usize;
        if given != params.len() {
            return Err(Diag::ty(span, format!("`{}` takes {} argument(s), but {} were given", self.sigs[id].name, params.len(), given)));
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
            self.coerce(&ta.ty, p, a.span)?;
            targs_out.push(ta);
        }
        for t in &targs {
            self.pending.push((span, t.clone()));
        }
        Ok(TExpr { kind: TK::Call(id, targs, targs_out), ty: ret, span })
    }

    fn check_method(&mut self, recv: &ast::Expr, name: &ast::Ident, type_args: &[ast::TypeExpr], args: &[ast::Arg], span: Span, expected: Option<&Type>) -> Result<TExpr> {
        if let ExprKind::Ident(q) = &recv.kind
            && let Some((adt, v)) = self.qualified_variant(q, &name.name)
        {
            return self.check_variant(adt, v, args, span, expected);
        }
        let r = self.check_expr(recv, None)?;
        let rt = self.zonk(&r.ty);
        let prefix = match &rt {
            Type::Int => Some("int"),
            Type::Float => Some("float"),
            Type::Bool => Some("bool"),
            Type::Str => Some("str"),
            Type::Array(_) => Some("arr"),
            Type::Adt(id, _) if *id == self.prelude_adt("Option") => Some("opt"),
            Type::Adt(id, _) if *id == self.prelude_adt("Result") => Some("res"),
            _ => None,
        };
        let args_e: Vec<ast::Expr> = args.iter().map(|a| a.value.clone()).collect();
        if let Some(p) = prefix
            && let Some(id) = self.prelude_fn(&format!("{p}_{}", name.name))
        {
            return self.call_fn(id, type_args, args_e, args, span, Some(r));
        }
        if let Some(Sym::Fn(id)) = self.scopes[self.file].get(&name.name).copied() {
            return self.call_fn(id, type_args, args_e, args, span, Some(r));
        }
        let mut d = Diag::ty(name.span, format!("no method `{}` on `{}`", name.name, self.show(&rt)));
        if let Some(p) = prefix {
            let pre = format!("{p}_");
            let methods: Vec<String> = self.scopes[0].keys().filter_map(|k| k.strip_prefix(&pre).map(str::to_string)).collect();
            if let Some(s) = suggest(&name.name, methods.iter().map(|s| s.as_str())) {
                d = d.help(format!("did you mean `{s}`?"));
            }
        }
        Err(d)
    }

    // ----- patterns -----

    pub fn check_pat(&mut self, p: &ast::Pattern, ty: &Type) -> Result<TPat> {
        let ty = self.zonk(ty);
        let mismatch = |this: &Self, what: &str| Err(Diag::ty(p.span, format!("{what} pattern cannot match a value of type `{}`", this.show(&ty))));
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
                let Type::Adt(adt, targs) = &ty else { return mismatch(self, &format!("variant `{}`", name.name)) };
                if let Some(q) = qual
                    && q.name != self.adts[*adt].name
                {
                    return Err(Diag::ty(q.span, format!("expected a variant of `{}`, found `{}`", self.show(&ty), q.name)));
                }
                let Some(v) = self.adts[*adt].variants().iter().position(|(n, _)| *n == name.name) else {
                    let mut d = Diag::ty(name.span, format!("`{}` is not a variant of `{}`", name.name, self.show(&ty)));
                    if let Some(s) = suggest(&name.name, self.adts[*adt].variants().iter().map(|(n, _)| n.as_str())) {
                        d = d.help(format!("did you mean `{s}`?"));
                    }
                    return Err(d);
                };
                let map: Vec<(String, Type)> = self.adts[*adt].generics.iter().cloned().zip(targs.clone()).collect();
                let ftys = self.adts[*adt].variants()[v].1.clone();
                if ftys.len() != args.len() {
                    return Err(Diag::ty(p.span, format!("`{}` has {} field(s), but the pattern has {}", name.name, ftys.len(), args.len())));
                }
                let sub = args.iter().zip(&ftys).map(|(a, ft)| self.check_pat(a, &ft.subst(&map))).collect::<Result<Vec<_>>>()?;
                TPat::Variant(*adt, targs.clone(), v, sub)
            }
        })
    }

    pub fn check_exhaustive(&self, ty: &Type, arms: &[(TPat, TExpr)], span: Span) -> Result<()> {
        let rows: Vec<Vec<&TPat>> = arms.iter().map(|(p, _)| vec![p]).collect();
        if let Some(w) = self.witness(&rows, std::slice::from_ref(ty)) {
            return Err(Diag::ty(span, format!("non-exhaustive match: `{}` is not covered", w[0]))
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
                let map: Vec<(String, Type)> = self.adts[*id].generics.iter().cloned().zip(targs.clone()).collect();
                Some(self.adts[*id].variants().iter().map(|(n, f)| (n.clone(), f.iter().map(|t| t.subst(&map)).collect())).collect())
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
                    let spec: Vec<Vec<&TPat>> = rows.iter().filter_map(|r| {
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
                    }).collect();
                    let tys2: Vec<Type> = ftys.iter().cloned().chain(tys[1..].iter().cloned()).collect();
                    if let Some(w) = self.witness(&spec, &tys2) {
                        let (fields, rest) = w.split_at(ftys.len());
                        let head = if fields.is_empty() { label.clone() } else { format!("{label}({})", fields.join(", ")) };
                        return Some(std::iter::once(head).chain(rest.iter().cloned()).collect());
                    }
                }
                return None;
            }
        }
        let default: Vec<Vec<&TPat>> = rows.iter().filter(|r| matches!(r[0], TPat::Wild | TPat::Bind(_))).map(|r| r[1..].to_vec()).collect();
        let w = self.witness(&default, &tys[1..])?;
        let head = match &ctors {
            Some(ctors) => {
                let (ci, (label, ftys)) = ctors.iter().enumerate().find(|(ci, (l, _))| !rows.iter().any(|r| head_matches(r[0], *ci, l))).unwrap();
                let _ = ci;
                if ftys.is_empty() { label.clone() } else { format!("{label}({})", vec!["_"; ftys.len()].join(", ")) }
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
        Type::Array(t) => has_var(t),
        Type::Adt(_, a) => a.iter().any(has_var),
        _ => false,
    }
}

/// Unconstrained variables (e.g. the element type of an array that is never used) default to `()`.
fn default_vars(t: &Type) -> Type {
    match t {
        Type::Var(_) => Type::Unit,
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
                cur.push((prev[j] + (ca != *cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    cands.filter(|c| *c != name).map(|c| (dist(name, c), c)).filter(|(d, c)| *d <= 2.max(c.len() / 3) && *d < name.len().max(2))
        .min_by_key(|(d, _)| *d).map(|(_, c)| c.to_string())
}
