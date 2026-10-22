//! Lowering of components to Wasm functions that build and update the DOM.
//!
//! Every component and every view fragment (an `if`/`match`/`routes` branch or a `for`
//! item) becomes a *scope*: a GC struct plus five generated functions:
//!
//! - `mount(s, parent, anchor)` creates its DOM nodes and inserts them before `anchor`;
//! - `update(s, mask)` re-evaluates only bindings whose dependencies intersect `mask`;
//! - `destroy(s, detach)` releases event-handler slots (and removes nodes if `detach`);
//! - `move(s, parent, anchor)` re-inserts its root nodes (keyed list reordering);
//! - `first(s)` returns its first DOM node id, or -1 when it has none.
//!
//! Dependencies are computed statically: bit `i` of a mask stands for component value `i`
//! (property, state or derived value), `BIT_BOUND` for values bound by enclosing fragments
//! and `BIT_ROUTE` for the current route. DOM changes are encoded as commands in linear
//! memory and applied by the host in one batch per event.

use std::collections::HashMap;

use crate::check_ui::{
    AttrKind, CompSig, Payload, Special, SwitchKind, TComp, TElem, TVarKind, TView,
};
use crate::diag::{SourceMap, Span};
use crate::ir::*;
use crate::lower::{FnCx, IO_BASE, Lower};
use crate::tir::{LocalId, TExpr, TFetch, TK, TPat, TStmt, Type};

pub const BIT_BOUND: i64 = 1 << 62;
pub const BIT_ROUTE: i64 = i64::MIN;
const CMD_CAP: i32 = 65536;
const MAX_CMD_STR: i32 = 60000;

/// Command opcodes understood by the host (`runtime/vcwl.js`).
pub mod opc {
    pub const CREATE: i32 = 1;
    pub const TEXT: i32 = 2;
    pub const COMMENT: i32 = 3;
    pub const INSERT: i32 = 4;
    pub const REMOVE: i32 = 5;
    pub const SET_TEXT: i32 = 6;
    pub const SET_ATTR: i32 = 7;
    pub const REMOVE_ATTR: i32 = 8;
    pub const SET_PROP: i32 = 9;
    pub const SET_CHECKED: i32 = 10;
    pub const LISTEN: i32 = 11;
    pub const CLASS: i32 = 12;
    pub const TOGGLE_CLASS: i32 = 13;
    pub const TITLE: i32 = 14;
    pub const NAVIGATE: i32 = 15;
    pub const WARN: i32 = 16;
    pub const CLEAR: i32 = 17;
    pub const FETCH: i32 = 18;
}

/// Handler-slot kinds at or above this value are pending `fetch` requests.
const FETCH_KIND_BASE: i32 = 1 << 16;

#[derive(Clone, Copy, Debug)]
pub struct ScopeRef {
    pub scope: usize,
    pub local: u32,
}

#[derive(Clone, Copy, Default)]
pub struct ScopeFns {
    pub mount: u32,
    pub update: u32,
    pub destroy: u32,
    pub mv: u32,
    pub first: u32,
}

/// Fields of a component scope beyond its values: the `dead` flag and, per event, the
/// listener's function reference, scope and function type.
#[derive(Clone, Default)]
pub struct CompFields {
    pub dead: u32,
    pub events: Vec<(u32, u32, u32)>,
}

pub struct Scope {
    pub comp: usize,
    pub comp_fields: CompFields,
    pub parent: Option<usize>,
    pub ty: u32,
    pub fields: Vec<(Storage, bool)>,
    pub bound: HashMap<LocalId, u32>,
    pub key_field: Option<u32>,
    pub fns: ScopeFns,
}

#[derive(Clone, Copy, Default)]
struct Globals {
    cmd: u32,
    next_node: u32,
    slot_scope: u32,
    slot_kind: u32,
    slot_n: u32,
    free: u32,
    free_n: u32,
    route: u32,
    app: u32,
}

pub struct UiState {
    pub comps: Vec<TComp>,
    pub sigs: Vec<CompSig>,
    pub scopes: Vec<Scope>,
    pub comp_scope: Vec<usize>,
    /// Handler kinds: scope type, function and payload; the index is the kind id.
    handler_kinds: Vec<(usize, u32, Payload)>,
    /// Handler function per (component, handler index).
    handler_fns: HashMap<(usize, usize), u32>,
    /// Continuations of `fetch` statements: (scope, function).
    fetch_kinds: Vec<(usize, u32)>,
    defaults: HashMap<(usize, usize), u32>,
    g: Globals,
    any_arr: u32,
    i32_arr: u32,
    pub sm: SourceMap,
    pub dev: bool,
    pub title: String,
}

enum Root {
    Node(u32),
    Switch {
        anchor: u32,
        idx: u32,
        branches: Vec<(usize, u32)>,
    },
    List {
        anchor: u32,
        frags: u32,
        count: u32,
        scope: usize,
    },
    Child {
        field: u32,
        comp: usize,
    },
}

#[derive(Clone, Copy)]
enum Pos {
    Root,
    Elem(u32),
}

/// Function bodies being built for one scope.
struct Gen {
    si: usize,
    mount: FnCx,
    update: FnCx,
    destroy: FnCx,
    mb: Vec<IExpr>,
    ub: Vec<IExpr>,
    db: Vec<IExpr>,
    roots: Vec<Root>,
}

enum S {
    Static(String),
    Dyn(IExpr),
}

fn nullable(v: VT) -> VT {
    match v {
        VT::Ref(t) => VT::RefNull(t),
        v => v,
    }
}

impl UiState {
    pub fn new(
        comps: Vec<TComp>,
        sigs: Vec<CompSig>,
        sm: SourceMap,
        dev: bool,
        title: String,
    ) -> UiState {
        UiState {
            comps,
            sigs,
            scopes: Vec::new(),
            comp_scope: Vec::new(),
            handler_kinds: Vec::new(),
            handler_fns: HashMap::new(),
            fetch_kinds: Vec::new(),
            defaults: HashMap::new(),
            g: Globals::default(),
            any_arr: 0,
            i32_arr: 0,
            sm,
            dev,
            title,
        }
    }
}

impl Lower<'_> {
    fn u(&mut self) -> &mut UiState {
        self.ui.as_mut().expect("ui state")
    }

    fn placeholder_fn(&mut self) -> u32 {
        self.m.funcs.push(Func {
            name: String::new(),
            params: vec![],
            result: None,
            locals: vec![],
            body: vec![],
            export: None,
        });
        self.m.funcs.len() as u32 - 1
    }

    fn global(&mut self, ty: VT, init: IExpr) -> u32 {
        self.m.globals.push(Global {
            ty,
            init,
            export: None,
        });
        self.m.globals.len() as u32 - 1
    }

    /// Generates every component, the runtime helpers and the exported entry points.
    pub fn lower_components(&mut self, root_comp: usize) {
        let any_arr = self.m.array_type(Storage::Val(VT::Any));
        let i32_arr = self.m.array_type(Storage::Val(VT::I32));
        let route = self.global(VT::RefNull(self.str_ty), IExpr::Null(self.str_ty));
        let g = Globals {
            cmd: self.global(VT::I32, IExpr::I32(0)),
            next_node: self.global(VT::I32, IExpr::I32(1)),
            slot_scope: self.global(VT::RefNull(any_arr), IExpr::Null(any_arr)),
            slot_kind: self.global(VT::RefNull(i32_arr), IExpr::Null(i32_arr)),
            slot_n: self.global(VT::I32, IExpr::I32(0)),
            free: self.global(VT::RefNull(i32_arr), IExpr::Null(i32_arr)),
            free_n: self.global(VT::I32, IExpr::I32(0)),
            route,
            app: 0,
        };
        {
            let u = self.u();
            u.g = g;
            u.any_arr = any_arr;
            u.i32_arr = i32_arr;
        }
        let n = self.u().comps.len();
        for ci in 0..n {
            let si = self.new_scope(ci, None);
            self.u().comp_scope.push(si);
        }
        // Component layout: dirty mask, values (fields 1..=n), dead flag, event listeners.
        for ci in 0..n {
            let si = self.u().comp_scope[ci];
            let comp = self.u().comps[ci].clone();
            for v in &comp.vars {
                let vt = self.vt(&v.ty).unwrap();
                self.add_field(si, vt);
            }
            let dead = self.add_field(si, VT::I32);
            let mut events = Vec::new();
            for (_, ety) in self.u().sigs[ci].events.clone() {
                let mut params = vec![VT::Any];
                if let Some(vt) = ety.as_ref().and_then(|t| self.vt(t)) {
                    params.push(vt);
                }
                let sig = self.m.add_type(TypeDef::Func {
                    params,
                    result: None,
                });
                let f = self.add_field(si, VT::RefNull(sig));
                let s = self.add_field(si, VT::Any);
                events.push((f, s, sig));
            }
            self.u().scopes[si].comp_fields = CompFields { dead, events };
        }
        for ci in 0..n {
            let comp = self.u().comps[ci].clone();
            let si = self.u().comp_scope[ci];
            self.gen_scope(si, &comp, &comp.body);
        }
        let rs = self.u().comp_scope[root_comp];
        let app_ty = self.u().scopes[rs].ty;
        let app = self.global(VT::RefNull(app_ty), IExpr::Null(app_ty));
        self.u().g.app = app;
        self.gen_entry_points(root_comp);
        let scopes: Vec<(u32, Vec<(Storage, bool)>)> = self
            .u()
            .scopes
            .iter()
            .map(|s| (s.ty, s.fields.clone()))
            .collect();
        for (ty, fields) in scopes {
            self.m.types[ty as usize] = TypeDef::Struct {
                fields,
                sup: None,
                is_final: true,
            };
        }
    }

    fn new_scope(&mut self, comp: usize, parent: Option<usize>) -> usize {
        let ty = self.m.add_type(TypeDef::Struct {
            fields: vec![],
            sup: None,
            is_final: true,
        });
        let fns = ScopeFns {
            mount: self.placeholder_fn(),
            update: self.placeholder_fn(),
            destroy: self.placeholder_fn(),
            mv: self.placeholder_fn(),
            first: self.placeholder_fn(),
        };
        let first_field = match parent {
            Some(p) => (Storage::Val(VT::RefNull(self.u().scopes[p].ty)), true),
            None => (Storage::Val(VT::I64), true),
        };
        let u = self.u();
        u.scopes.push(Scope {
            comp,
            comp_fields: CompFields::default(),
            parent,
            ty,
            fields: vec![first_field],
            bound: HashMap::new(),
            key_field: None,
            fns,
        });
        u.scopes.len() - 1
    }

    fn add_field(&mut self, si: usize, vt: VT) -> u32 {
        let f = &mut self.u().scopes[si].fields;
        f.push((Storage::Val(nullable(vt)), true));
        f.len() as u32 - 1
    }

    fn scope_ty(&mut self, si: usize) -> u32 {
        self.u().scopes[si].ty
    }

    // ----- reading component values -----

    /// Walks from the function's scope up to scope `target`, returning a reference to it.
    fn scope_path(&mut self, cx: &FnCx, target: usize) -> IExpr {
        let sr = cx
            .scope
            .expect("component value read outside a component scope");
        let mut x = get(sr.local);
        let mut cur = sr.scope;
        while cur != target {
            let (ty, parent) = {
                let s = &self.u().scopes[cur];
                (s.ty, s.parent.expect("scope chain does not reach target"))
            };
            x = IExpr::StructGet(ty, 0, b(x));
            cur = parent;
        }
        x
    }

    fn comp_root_of(&mut self, scope: usize) -> usize {
        let mut cur = scope;
        while let Some(p) = self.u().scopes[cur].parent {
            cur = p;
        }
        cur
    }

    fn read_field(&mut self, si: usize, field: u32, obj: IExpr, vt: Option<VT>) -> IExpr {
        let ty = self.scope_ty(si);
        let g = IExpr::StructGet(ty, field, b(obj));
        match vt {
            Some(VT::Ref(_)) => un(UnOp::RefAsNonNull, g),
            Some(_) => g,
            None => IExpr::Drop(b(g)),
        }
    }

    /// Field index of component value `i` in the component struct (field 0 is the dirty mask).
    fn var_field(i: usize) -> u32 {
        i as u32 + 1
    }

    pub fn ui_bound(&mut self, cx: &FnCx, id: LocalId) -> bool {
        if cx.unbound.contains(&id) || cx.scope.is_none() || self.ui.is_none() {
            return false;
        }
        let mut cur = Some(cx.scope.unwrap().scope);
        while let Some(s) = cur {
            if self.u().scopes[s].bound.contains_key(&id) {
                return true;
            }
            cur = self.u().scopes[s].parent;
        }
        false
    }

    pub fn ui_read(&mut self, cx: &mut FnCx, e: &TExpr) -> IExpr {
        let vt = self.vt(&e.ty);
        match &e.kind {
            TK::Route => {
                let g = self.u().g.route;
                un(UnOp::RefAsNonNull, IExpr::GlobalGet(g))
            }
            TK::CompVar(i) => {
                let root = self.comp_root_of(cx.scope.unwrap().scope);
                let obj = self.scope_path(cx, root);
                self.read_field(root, Self::var_field(*i), obj, vt)
            }
            TK::Local(id) => {
                let mut cur = cx.scope.unwrap().scope;
                loop {
                    if let Some(&f) = self.u().scopes[cur].bound.get(id) {
                        let obj = self.scope_path(cx, cur);
                        return self.read_field(cur, f, obj, vt);
                    }
                    cur = self.u().scopes[cur].parent.unwrap();
                }
            }
            _ => unreachable!(),
        }
    }

    fn values_differ(&mut self, t: &Type, a: IExpr, b2: IExpr) -> IExpr {
        match t {
            Type::Int => bin(Op::I64Ne, a, b2),
            Type::Float => bin(Op::F64Ne, a, b2),
            Type::Bool => bin(Op::I32Ne, a, b2),
            Type::Str => {
                let id = self.fns.iter().position(|f| f.name == "str_eq").unwrap();
                let f = self.fn_inst(id, &[]);
                un(UnOp::I32Eqz, IExpr::Call(FuncRef::Local(f), vec![a, b2]))
            }
            _ => un(UnOp::I32Eqz, IExpr::RefEq(b(a), b(b2))),
        }
    }

    /// `state = value` inside a handler: store and mark dirty only when the value changed.
    pub fn ui_assign(&mut self, cx: &mut FnCx, var: usize, e: &TExpr) -> IExpr {
        let root = self.comp_root_of(cx.scope.unwrap().scope);
        let ty = self.ty(cx, &e.ty);
        let vt = self.vt(&ty).unwrap();
        let comp_ty = self.scope_ty(root);
        let c = cx.temp(VT::Ref(comp_ty));
        let v = cx.temp(vt);
        let path = self.scope_path(cx, root);
        let value = self.expr(cx, e);
        let old = self.read_field(root, Self::var_field(var), get(c), Some(vt));
        let differ = self.values_differ(&ty, get(v), old);
        IExpr::Seq(vec![
            set(c, un(UnOp::RefAsNonNull, path)),
            set(v, value),
            IExpr::If(
                None,
                b(differ),
                vec![
                    IExpr::StructSet(comp_ty, Self::var_field(var), b(get(c)), b(get(v))),
                    IExpr::StructSet(
                        comp_ty,
                        0,
                        b(get(c)),
                        b(bin(
                            Op::I64Or,
                            IExpr::StructGet(comp_ty, 0, b(get(c))),
                            IExpr::I64(1i64 << var),
                        )),
                    ),
                ],
                vec![],
            ),
        ])
    }

    pub fn ui_intrinsic(&mut self, name: &str, _targs: &[Type], _args: Vec<IExpr>) -> IExpr {
        unreachable!("unknown intrinsic {name}")
    }

    /// Starts a request: the continuation and the handler's scope go into a handler slot,
    /// and a FETCH command asks the host to perform it. The host answers with `fetch_done`.
    pub fn lower_fetch(&mut self, cx: &mut FnCx, f: &TFetch) -> IExpr {
        let sr = cx.scope.expect("fetch outside a handler");
        let cont = self.fetch_fn(sr.scope, f);
        let kind = {
            let u = self.u();
            u.fetch_kinds.push((sr.scope, cont));
            FETCH_KIND_BASE + u.fetch_kinds.len() as i32 - 1
        };
        let alloc = self.helper_alloc_slot();
        let slot = cx.temp(VT::I32);
        let method = ["GET", "POST", "PUT", "PATCH", "DELETE"]
            .iter()
            .position(|m| *m == f.method)
            .unwrap() as i32;
        let url = self.expr(cx, &f.url);
        let body = match &f.body {
            Some(e) => S::Dyn(self.expr(cx, e)),
            None => S::Static(String::new()),
        };
        let has_body = IExpr::I32(f.body.is_some() as i32);
        let cmd = self.op(
            cx,
            opc::FETCH,
            vec![get(slot), IExpr::I32(method), has_body],
            vec![S::Dyn(url), body],
        );
        IExpr::Seq(vec![
            set(
                slot,
                IExpr::Call(alloc, vec![get(sr.local), IExpr::I32(kind)]),
            ),
            cmd,
        ])
    }

    /// `(scope, kind: i64, status: i64, len: i64)`: runs a fetch's arms, then updates the
    /// component like a handler. Responses for destroyed components are discarded.
    fn fetch_fn(&mut self, si: usize, f: &TFetch) -> u32 {
        let idx = self.placeholder_fn();
        let comp = self.u().scopes[si].comp;
        let c = self.u().comps[comp].clone();
        let params = vec![VT::Ref(self.scope_ty(si)), VT::I64, VT::I64, VT::I64];
        let mut cx = FnCx::new(params.clone(), c.locals.clone(), vec![]);
        cx.scope = Some(ScopeRef {
            scope: si,
            local: 0,
        });
        for (i, p) in f.params.iter().enumerate() {
            cx.map.insert(*p, i as u32 + 1);
        }
        let root = self.comp_root_of(si);
        let (rty, dead) = (self.scope_ty(root), self.u().scopes[root].comp_fields.dead);
        let path = self.scope_path(&cx, root);
        let mut body = vec![self.stmt_expr(&mut cx, &f.on_done)];
        body.extend(self.handler_epilogue(&mut cx, si));
        // The dead check stays outside `wrap_returns`, which turns returns into jumps to the epilogue.
        let mut body = wrap_returns(body);
        body.insert(
            0,
            IExpr::If(
                None,
                b(IExpr::StructGet(rty, dead, b(un(UnOp::RefAsNonNull, path)))),
                vec![IExpr::Return(None)],
                vec![],
            ),
        );
        self.m.funcs[idx as usize] = Func {
            name: format!("{}_fetch{idx}", c.name),
            params,
            result: None,
            locals: cx.body_locals(),
            body,
            export: None,
        };
        idx
    }

    // ----- dependencies -----

    fn deps(&mut self, cx: &FnCx, e: &TExpr) -> i64 {
        let mut m = 0;
        self.deps_into(cx, e, &mut m);
        m
    }

    fn deps_into(&mut self, cx: &FnCx, e: &TExpr, m: &mut i64) {
        match &e.kind {
            TK::CompVar(i) => *m |= 1 << i,
            TK::Route => *m |= BIT_ROUTE,
            TK::Local(id) if self.ui_bound(cx, *id) => *m |= BIT_BOUND,
            TK::Bin(_, a, b2) | TK::Index(a, b2) => {
                self.deps_into(cx, a, m);
                self.deps_into(cx, b2, m);
            }
            TK::Not(a) | TK::Neg(a) | TK::Field(a, _) => self.deps_into(cx, a, m),
            TK::Call(_, _, xs)
            | TK::Intrinsic(_, _, xs)
            | TK::StructNew(_, _, xs)
            | TK::Variant(_, _, _, xs)
            | TK::Array(xs) => xs.iter().for_each(|x| self.deps_into(cx, x, m)),
            TK::If(c, t, f) => {
                self.deps_into(cx, c, m);
                self.deps_into(cx, t, m);
                if let Some(f) = f {
                    self.deps_into(cx, f, m);
                }
            }
            TK::Block(stmts, tail) => {
                for s in stmts {
                    match s {
                        TStmt::Let(_, x) | TStmt::Expr(x) | TStmt::Assign(_, x) => {
                            self.deps_into(cx, x, m)
                        }
                        TStmt::While(a, b2) | TStmt::ForArray(_, a, b2) => {
                            self.deps_into(cx, a, m);
                            self.deps_into(cx, b2, m);
                        }
                        TStmt::ForRange(_, a, b2, c) => {
                            self.deps_into(cx, a, m);
                            self.deps_into(cx, b2, m);
                            self.deps_into(cx, c, m);
                        }
                        _ => {}
                    }
                }
                if let Some(t) = tail {
                    self.deps_into(cx, t, m);
                }
            }
            TK::Match(s, arms) => {
                self.deps_into(cx, s, m);
                arms.iter().for_each(|(_, x)| self.deps_into(cx, x, m));
            }
            TK::Return(Some(x)) => self.deps_into(cx, x, m),
            _ => {}
        }
    }

    /// Union of the dependencies of every binding in a view subtree (used to skip updates).
    fn views_deps(&mut self, cx: &FnCx, views: &[TView]) -> i64 {
        let mut m = 0;
        for v in views {
            m |= match v {
                TView::Element(e) => {
                    let mut d = 0;
                    for (_, x) in &e.attrs {
                        d |= self.deps(cx, x);
                    }
                    for (_, x) in &e.class_ifs {
                        d |= self.deps(cx, x);
                    }
                    if let Special::Checkbox(x) | Special::Select(x) = &e.special {
                        d |= self.deps(cx, x);
                    }
                    d | self.views_deps(cx, &e.children)
                }
                TView::Text(x) => self.deps(cx, x),
                TView::Switch { kind, branches } => {
                    let d = match kind {
                        SwitchKind::If(x) | SwitchKind::Match(x, _) => self.deps(cx, x),
                        SwitchKind::Routes(_) => BIT_ROUTE,
                    };
                    // Branch contents may use values bound by this switch (match arms).
                    branches
                        .iter()
                        .fold(d, |acc, br| acc | self.views_deps(cx, br))
                        | if matches!(kind, SwitchKind::Match(..)) {
                            BIT_BOUND
                        } else {
                            0
                        }
                }
                TView::For {
                    iter, key, body, ..
                } => {
                    let mut d = self.deps(cx, iter) | BIT_BOUND;
                    if let Some(k) = key {
                        d |= self.deps(cx, k);
                    }
                    d | self.views_deps(cx, body)
                }
                TView::Component { props, .. } => props
                    .iter()
                    .fold(BIT_ROUTE, |acc, p| acc | self.deps(cx, p)),
            };
        }
        m
    }

    // ----- command buffer -----

    fn helper_reserve(&mut self) -> FuncRef {
        let cmd = self.u().g.cmd;
        let flush = self.m.import("flush", vec![VT::I32], None);
        self.helper("cmd_reserve", |_| Func {
            name: "cmd_reserve".into(),
            params: vec![VT::I32],
            result: None,
            locals: vec![],
            body: vec![IExpr::If(
                None,
                b(bin(
                    Op::I32GtS,
                    bin(Op::I32Add, IExpr::GlobalGet(cmd), get(0)),
                    IExpr::I32(CMD_CAP),
                )),
                vec![
                    IExpr::Call(flush, vec![IExpr::GlobalGet(cmd)]),
                    IExpr::GlobalSet(cmd, b(IExpr::I32(0))),
                ],
                vec![],
            )],
            export: None,
        })
    }

    fn helper_put_i32(&mut self) -> FuncRef {
        let cmd = self.u().g.cmd;
        self.helper("cmd_i32", |_| Func {
            name: "cmd_i32".into(),
            params: vec![VT::I32],
            result: None,
            locals: vec![],
            body: vec![
                IExpr::Store32(b(IExpr::GlobalGet(cmd)), b(get(0))),
                IExpr::GlobalSet(
                    cmd,
                    b(bin(Op::I32Add, IExpr::GlobalGet(cmd), IExpr::I32(4))),
                ),
            ],
            export: None,
        })
    }

    /// `(s) -> i32`: encoded size of a string operand (length word + padded bytes).
    fn helper_str_size(&mut self) -> FuncRef {
        let panic = self.panic_call("text too long for a single DOM update (limit 60000 bytes)");
        self.helper("cmd_str_size", |_| Func {
            name: "cmd_str_size".into(),
            params: vec![VT::Ref(0)],
            result: Some(VT::I32),
            locals: vec![VT::I32],
            body: vec![
                set(1, IExpr::ArrayLen(b(get(0)))),
                IExpr::If(
                    None,
                    b(bin(Op::I32GtS, get(1), IExpr::I32(MAX_CMD_STR))),
                    vec![panic],
                    vec![],
                ),
                bin(
                    Op::I32Add,
                    IExpr::I32(4),
                    bin(
                        Op::I32And,
                        bin(Op::I32Add, get(1), IExpr::I32(3)),
                        IExpr::I32(-4),
                    ),
                ),
            ],
            export: None,
        })
    }

    fn helper_put_str(&mut self) -> FuncRef {
        let cmd = self.u().g.cmd;
        let s = self.str_ty;
        self.helper("cmd_str", |_| {
            let (str_, n, i) = (0, 1, 2);
            Func {
                name: "cmd_str".into(),
                params: vec![VT::Ref(s)],
                result: None,
                locals: vec![VT::I32, VT::I32],
                body: vec![
                    set(n, IExpr::ArrayLen(b(get(str_)))),
                    IExpr::Store32(b(IExpr::GlobalGet(cmd)), b(get(n))),
                    IExpr::Block(
                        1,
                        None,
                        vec![IExpr::Loop(
                            2,
                            vec![
                                IExpr::BrIf(1, b(bin(Op::I32GeU, get(i), get(n)))),
                                IExpr::Store8(
                                    b(bin(
                                        Op::I32Add,
                                        bin(Op::I32Add, IExpr::GlobalGet(cmd), IExpr::I32(4)),
                                        get(i),
                                    )),
                                    b(IExpr::ArrayGetU(s, b(get(str_)), b(get(i)))),
                                ),
                                set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                                IExpr::Br(2),
                            ],
                        )],
                    ),
                    IExpr::GlobalSet(
                        cmd,
                        b(bin(
                            Op::I32Add,
                            IExpr::GlobalGet(cmd),
                            bin(
                                Op::I32Add,
                                IExpr::I32(4),
                                bin(
                                    Op::I32And,
                                    bin(Op::I32Add, get(n), IExpr::I32(3)),
                                    IExpr::I32(-4),
                                ),
                            ),
                        )),
                    ),
                ],
                export: None,
            }
        })
    }

    fn static_bytes(&mut self, s: &str) -> (u32, u32) {
        if let Some(&r) = self.strings.get(s) {
            return r;
        }
        let off = self.m.data.len() as u32;
        self.m.data.extend_from_slice(s.as_bytes());
        self.strings.insert(s.to_string(), (off, s.len() as u32));
        (off, s.len() as u32)
    }

    /// Emits one command: opcode, integer operands, then string operands.
    fn op(&mut self, cx: &mut FnCx, code: i32, ints: Vec<IExpr>, strs: Vec<S>) -> IExpr {
        let cmd = self.u().g.cmd;
        let reserve = self.helper_reserve();
        let put = self.helper_put_i32();
        let mut out = Vec::new();
        let mut size = IExpr::I32(4 * (1 + ints.len() as i32));
        let mut writes = Vec::new();
        for s in strs {
            match s {
                S::Static(text) => {
                    let (off, len) = self.static_bytes(&text);
                    let padded = 4 + ((len as i32 + 3) & !3);
                    size = bin(Op::I32Add, size, IExpr::I32(padded));
                    writes.push(IExpr::Call(put, vec![IExpr::I32(len as i32)]));
                    writes.push(IExpr::MemoryInit(b(IExpr::GlobalGet(cmd)), off, len));
                    writes.push(IExpr::GlobalSet(
                        cmd,
                        b(bin(
                            Op::I32Add,
                            IExpr::GlobalGet(cmd),
                            IExpr::I32(padded - 4),
                        )),
                    ));
                }
                S::Dyn(e) => {
                    let t = cx.temp(VT::Ref(self.str_ty));
                    out.push(set(t, e));
                    let sz = self.helper_str_size();
                    size = bin(Op::I32Add, size, IExpr::Call(sz, vec![get(t)]));
                    let ps = self.helper_put_str();
                    writes.push(IExpr::Call(ps, vec![get(t)]));
                }
            }
        }
        out.push(IExpr::Call(reserve, vec![size]));
        out.push(IExpr::Call(put, vec![IExpr::I32(code)]));
        for i in ints {
            out.push(IExpr::Call(put, vec![i]));
        }
        out.extend(writes);
        IExpr::Seq(out)
    }

    fn helper_new_node(&mut self) -> FuncRef {
        let g = self.u().g.next_node;
        self.helper("new_node", |_| Func {
            name: "new_node".into(),
            params: vec![],
            result: Some(VT::I32),
            locals: vec![],
            body: vec![
                IExpr::GlobalGet(g),
                IExpr::GlobalSet(g, b(bin(Op::I32Add, IExpr::GlobalGet(g), IExpr::I32(1)))),
            ],
            export: None,
        })
    }

    fn helper_flush(&mut self) -> FuncRef {
        let cmd = self.u().g.cmd;
        let flush = self.m.import("flush", vec![VT::I32], None);
        self.helper("cmd_flush", |_| Func {
            name: "cmd_flush".into(),
            params: vec![],
            result: None,
            locals: vec![],
            body: vec![IExpr::If(
                None,
                b(bin(Op::I32GtS, IExpr::GlobalGet(cmd), IExpr::I32(0))),
                vec![
                    IExpr::Call(flush, vec![IExpr::GlobalGet(cmd)]),
                    IExpr::GlobalSet(cmd, b(IExpr::I32(0))),
                ],
                vec![],
            )],
            export: None,
        })
    }

    // ----- handler slots -----

    fn helper_alloc_slot(&mut self) -> FuncRef {
        let g = self.u().g;
        let (any_arr, i32_arr) = (self.u().any_arr, self.u().i32_arr);
        self.helper("slot_alloc", |_| {
            let (scope, kind, slot, cap, a2, k2, f2) = (0, 1, 2, 3, 4, 5, 6);
            let grow = vec![
                set(
                    cap,
                    bin(
                        Op::I32Mul,
                        IExpr::ArrayLen(b(IExpr::GlobalGet(g.slot_scope))),
                        IExpr::I32(2),
                    ),
                ),
                set(a2, IExpr::ArrayNewDefault(any_arr, b(get(cap)))),
                set(k2, IExpr::ArrayNewDefault(i32_arr, b(get(cap)))),
                set(f2, IExpr::ArrayNewDefault(i32_arr, b(get(cap)))),
                IExpr::ArrayCopy(
                    any_arr,
                    [
                        b(get(a2)),
                        b(IExpr::I32(0)),
                        b(IExpr::GlobalGet(g.slot_scope)),
                        b(IExpr::I32(0)),
                        b(IExpr::GlobalGet(g.slot_n)),
                    ],
                ),
                IExpr::ArrayCopy(
                    i32_arr,
                    [
                        b(get(k2)),
                        b(IExpr::I32(0)),
                        b(IExpr::GlobalGet(g.slot_kind)),
                        b(IExpr::I32(0)),
                        b(IExpr::GlobalGet(g.slot_n)),
                    ],
                ),
                IExpr::GlobalSet(g.slot_scope, b(get(a2))),
                IExpr::GlobalSet(g.slot_kind, b(get(k2))),
                IExpr::GlobalSet(g.free, b(get(f2))),
            ];
            Func {
                name: "slot_alloc".into(),
                params: vec![VT::Any, VT::I32],
                result: Some(VT::I32),
                locals: vec![
                    VT::I32,
                    VT::I32,
                    VT::Ref(any_arr),
                    VT::Ref(i32_arr),
                    VT::Ref(i32_arr),
                ],
                body: vec![
                    IExpr::If(
                        None,
                        b(un(UnOp::RefIsNull, IExpr::GlobalGet(g.slot_scope))),
                        vec![
                            IExpr::GlobalSet(
                                g.slot_scope,
                                b(IExpr::ArrayNewDefault(any_arr, b(IExpr::I32(64)))),
                            ),
                            IExpr::GlobalSet(
                                g.slot_kind,
                                b(IExpr::ArrayNewDefault(i32_arr, b(IExpr::I32(64)))),
                            ),
                            IExpr::GlobalSet(
                                g.free,
                                b(IExpr::ArrayNewDefault(i32_arr, b(IExpr::I32(64)))),
                            ),
                        ],
                        vec![],
                    ),
                    IExpr::If(
                        None,
                        b(bin(Op::I32GtS, IExpr::GlobalGet(g.free_n), IExpr::I32(0))),
                        vec![
                            IExpr::GlobalSet(
                                g.free_n,
                                b(bin(Op::I32Sub, IExpr::GlobalGet(g.free_n), IExpr::I32(1))),
                            ),
                            set(
                                slot,
                                IExpr::ArrayGet(
                                    i32_arr,
                                    b(IExpr::GlobalGet(g.free)),
                                    b(IExpr::GlobalGet(g.free_n)),
                                ),
                            ),
                        ],
                        vec![
                            IExpr::If(
                                None,
                                b(bin(
                                    Op::I32GeU,
                                    IExpr::GlobalGet(g.slot_n),
                                    IExpr::ArrayLen(b(IExpr::GlobalGet(g.slot_scope))),
                                )),
                                grow,
                                vec![],
                            ),
                            set(slot, IExpr::GlobalGet(g.slot_n)),
                            IExpr::GlobalSet(
                                g.slot_n,
                                b(bin(Op::I32Add, IExpr::GlobalGet(g.slot_n), IExpr::I32(1))),
                            ),
                        ],
                    ),
                    IExpr::ArraySet(
                        any_arr,
                        b(IExpr::GlobalGet(g.slot_scope)),
                        b(get(slot)),
                        b(get(scope)),
                    ),
                    IExpr::ArraySet(
                        i32_arr,
                        b(IExpr::GlobalGet(g.slot_kind)),
                        b(get(slot)),
                        b(get(kind)),
                    ),
                    get(slot),
                ],
                export: None,
            }
        })
    }

    fn helper_release_slot(&mut self) -> FuncRef {
        let g = self.u().g;
        let (any_arr, i32_arr) = (self.u().any_arr, self.u().i32_arr);
        self.helper("slot_release", |_| Func {
            name: "slot_release".into(),
            params: vec![VT::I32],
            result: None,
            locals: vec![],
            body: vec![
                IExpr::ArraySet(
                    any_arr,
                    b(IExpr::GlobalGet(g.slot_scope)),
                    b(get(0)),
                    b(IExpr::NullAny),
                ),
                IExpr::ArraySet(
                    i32_arr,
                    b(IExpr::GlobalGet(g.free)),
                    b(IExpr::GlobalGet(g.free_n)),
                    b(get(0)),
                ),
                IExpr::GlobalSet(
                    g.free_n,
                    b(bin(Op::I32Add, IExpr::GlobalGet(g.free_n), IExpr::I32(1))),
                ),
            ],
            export: None,
        })
    }

    // ----- scopes -----

    fn gen_scope(&mut self, si: usize, comp: &TComp, views: &[TView]) {
        let ty = self.scope_ty(si);
        let locals = comp.locals.clone();
        let mk = |params: Vec<VT>| {
            let mut cx = FnCx::new(params, locals.clone(), vec![]);
            cx.scope = Some(ScopeRef {
                scope: si,
                local: 0,
            });
            cx
        };
        let mut g = Gen {
            si,
            mount: mk(vec![VT::Ref(ty), VT::I32, VT::I32]),
            update: mk(vec![VT::Ref(ty), VT::I64]),
            destroy: mk(vec![VT::Ref(ty), VT::I32]),
            mb: Vec::new(),
            ub: Vec::new(),
            db: Vec::new(),
            roots: Vec::new(),
        };
        let is_comp = self.u().scopes[si].parent.is_none();
        let ci = self.u().scopes[si].comp;
        if is_comp {
            self.gen_comp_values(&mut g, comp);
        }
        for v in views {
            self.view(&mut g, comp, v, Pos::Root);
        }
        if is_comp {
            for &h in &comp.mount_handlers {
                let f = self.handler_fn(ci, h, si);
                g.mb.push(IExpr::Call(FuncRef::Local(f), vec![get(0)]));
            }
        }
        self.finish_scope(g);
    }

    /// Component prologue: initialize state and derived values; recompute derived values on update.
    fn gen_comp_values(&mut self, g: &mut Gen, comp: &TComp) {
        let si = g.si;
        let ty = self.scope_ty(si);
        for (i, v) in comp.vars.iter().enumerate() {
            let vt = self.vt(&v.ty).unwrap();
            let f = Self::var_field(i);
            match &v.kind {
                TVarKind::Prop(_) => {}
                TVarKind::State(init) | TVarKind::Derived(init) => {
                    let x = self.expr(&mut g.mount, init);
                    g.mb.push(IExpr::StructSet(ty, f, b(get(0)), b(x)));
                }
            }
            if let TVarKind::Derived(e) = &v.kind {
                let deps = self.deps(&g.update, e);
                let t = g.update.temp(vt);
                let x = self.expr(&mut g.update, e);
                let old = self.read_field(si, f, get(0), Some(vt));
                let differ = self.values_differ(&v.ty, get(t), old);
                g.ub.push(IExpr::If(
                    None,
                    b(bin(
                        Op::I64Ne,
                        bin(Op::I64And, get(1), IExpr::I64(deps)),
                        IExpr::I64(0),
                    )),
                    vec![
                        set(t, x),
                        IExpr::If(
                            None,
                            b(differ),
                            vec![
                                IExpr::StructSet(ty, f, b(get(0)), b(get(t))),
                                set(1, bin(Op::I64Or, get(1), IExpr::I64(1i64 << i))),
                            ],
                            vec![],
                        ),
                    ],
                    vec![],
                ));
            }
        }
    }

    fn finish_scope(&mut self, g: Gen) {
        let si = g.si;
        let fns = self.u().scopes[si].fns;
        let ty = self.scope_ty(si);
        let name = format!("scope{si}");
        // move / first are derived from the root items.
        let mut mv = Vec::new();
        let mut first = Vec::new();
        let mut detach = Vec::new();
        let mut mv_cx_locals = vec![];
        for r in &g.roots {
            match r {
                Root::Node(f) => {
                    let id = IExpr::StructGet(ty, *f, b(get(0)));
                    mv.push(self.op_raw(opc::INSERT, vec![get(1), id.clone(), get(2)]));
                    first.push(IExpr::Return(Some(b(id.clone()))));
                    detach.push(self.op_raw(opc::REMOVE, vec![id]));
                }
                Root::Switch {
                    anchor,
                    idx,
                    branches,
                } => {
                    let a = IExpr::StructGet(ty, *anchor, b(get(0)));
                    let mut mv_arms = Vec::new();
                    let mut first_arms = Vec::new();
                    for (k, (bs, bf)) in branches.iter().enumerate() {
                        let bfns = self.u().scopes[*bs].fns;
                        let frag = un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *bf, b(get(0))));
                        mv_arms.push((
                            k,
                            IExpr::Call(
                                FuncRef::Local(bfns.mv),
                                vec![frag.clone(), get(1), get(2)],
                            ),
                        ));
                        first_arms.push((k, IExpr::Call(FuncRef::Local(bfns.first), vec![frag])));
                    }
                    let cur = IExpr::StructGet(ty, *idx, b(get(0)));
                    mv.push(switch_on(cur.clone(), mv_arms));
                    mv.push(self.op_raw(opc::INSERT, vec![get(1), a.clone(), get(2)]));
                    // first: the branch's first node, else the anchor.
                    let fl = 1;
                    mv_cx_locals.push(VT::I32);
                    let probe: Vec<(usize, IExpr)> = first_arms
                        .into_iter()
                        .map(|(k, e)| (k, set(fl, e)))
                        .collect();
                    first.push(set(fl, IExpr::I32(-1)));
                    first.push(switch_on(cur, probe));
                    first.push(IExpr::If(
                        None,
                        b(bin(Op::I32GeS, get(fl), IExpr::I32(0))),
                        vec![IExpr::Return(Some(b(get(fl))))],
                        vec![],
                    ));
                    first.push(IExpr::Return(Some(b(a))));
                }
                Root::List {
                    anchor,
                    frags,
                    count,
                    scope,
                } => {
                    let a = IExpr::StructGet(ty, *anchor, b(get(0)));
                    let lfns = self.u().scopes[*scope].fns;
                    let fty = self.scope_ty(*scope);
                    let arr = self.m.array_type(Storage::Val(VT::RefNull(fty)));
                    let item = |i: IExpr| {
                        un(
                            UnOp::RefAsNonNull,
                            IExpr::ArrayGet(
                                arr,
                                b(un(
                                    UnOp::RefAsNonNull,
                                    IExpr::StructGet(ty, *frags, b(get(0))),
                                )),
                                b(i),
                            ),
                        )
                    };
                    let n = IExpr::StructGet(ty, *count, b(get(0)));
                    // move: every item in order, then the anchor (locals: i at 3)
                    mv.push(for_range(
                        3,
                        n.clone(),
                        vec![IExpr::Call(
                            FuncRef::Local(lfns.mv),
                            vec![item(get(3)), get(1), get(2)],
                        )],
                        900,
                    ));
                    mv.push(self.op_raw(opc::INSERT, vec![get(1), a.clone(), get(2)]));
                    // first: first item with a node, else the anchor (locals: i at 1, r at 2)
                    first.push(for_range(
                        1,
                        n,
                        vec![
                            set(
                                2,
                                IExpr::Call(FuncRef::Local(lfns.first), vec![item(get(1))]),
                            ),
                            IExpr::If(
                                None,
                                b(bin(Op::I32GeS, get(2), IExpr::I32(0))),
                                vec![IExpr::Return(Some(b(get(2))))],
                                vec![],
                            ),
                        ],
                        901,
                    ));
                    first.push(IExpr::Return(Some(b(a))));
                }
                Root::Child { field, comp } => {
                    let cs = self.u().comp_scope[*comp];
                    let cfns = self.u().scopes[cs].fns;
                    let c = un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *field, b(get(0))));
                    mv.push(IExpr::Call(
                        FuncRef::Local(cfns.mv),
                        vec![c.clone(), get(1), get(2)],
                    ));
                    first.push(set(1, IExpr::Call(FuncRef::Local(cfns.first), vec![c])));
                    first.push(IExpr::If(
                        None,
                        b(bin(Op::I32GeS, get(1), IExpr::I32(0))),
                        vec![IExpr::Return(Some(b(get(1))))],
                        vec![],
                    ));
                }
            }
        }
        first.push(IExpr::I32(-1));
        // Destroy: nested cleanup always runs; root nodes are removed only when detaching.
        let mut db = g.db;
        db.push(IExpr::If(None, b(get(1)), detach, vec![]));
        if self.u().scopes[si].parent.is_none() {
            let dead = self.u().scopes[si].comp_fields.dead;
            db.push(IExpr::StructSet(ty, dead, b(get(0)), b(IExpr::I32(1))));
        }
        let funcs = [
            (
                fns.mount,
                format!("{name}_mount"),
                vec![VT::Ref(ty), VT::I32, VT::I32],
                None,
                g.mount.body_locals(),
                g.mb,
            ),
            (
                fns.update,
                format!("{name}_update"),
                vec![VT::Ref(ty), VT::I64],
                None,
                g.update.body_locals(),
                g.ub,
            ),
            (
                fns.destroy,
                format!("{name}_destroy"),
                vec![VT::Ref(ty), VT::I32],
                None,
                g.destroy.body_locals(),
                db,
            ),
            (
                fns.mv,
                format!("{name}_move"),
                vec![VT::Ref(ty), VT::I32, VT::I32],
                None,
                vec![VT::I32],
                mv,
            ),
            (
                fns.first,
                format!("{name}_first"),
                vec![VT::Ref(ty)],
                Some(VT::I32),
                vec![VT::I32, VT::I32],
                first,
            ),
        ];
        for (idx, name, params, result, locals, body) in funcs {
            self.m.funcs[idx as usize] = Func {
                name,
                params,
                result,
                locals,
                body,
                export: None,
            };
        }
    }

    /// A command with integer operands only, built without a function context.
    fn op_raw(&mut self, code: i32, ints: Vec<IExpr>) -> IExpr {
        let mut cx = FnCx::new(vec![], vec![], vec![]);
        self.op(&mut cx, code, ints, vec![])
    }

    fn insert(&mut self, cx: &mut FnCx, pos: Pos, node: IExpr) -> IExpr {
        match pos {
            Pos::Root => self.op(cx, opc::INSERT, vec![get(1), node, get(2)], vec![]),
            Pos::Elem(p) => self.op(cx, opc::INSERT, vec![get(p), node, IExpr::I32(0)], vec![]),
        }
    }

    fn parent_node(pos: Pos) -> IExpr {
        match pos {
            Pos::Root => get(1),
            Pos::Elem(p) => get(p),
        }
    }

    /// Mask test `mask & deps != 0` in the update function.
    fn dirty(deps: i64) -> IExpr {
        bin(
            Op::I64Ne,
            bin(Op::I64And, get(1), IExpr::I64(deps)),
            IExpr::I64(0),
        )
    }

    fn view(&mut self, g: &mut Gen, comp: &TComp, v: &TView, pos: Pos) {
        match v {
            TView::Text(e) => self.view_text(g, e, pos),
            TView::Element(el) => self.view_element(g, comp, el, pos),
            TView::Switch { kind, branches } => self.view_switch(g, comp, kind, branches, pos),
            TView::For {
                item,
                index,
                iter,
                key,
                body,
                span,
            } => self.view_for(g, comp, *item, *index, iter, key.as_ref(), body, *span, pos),
            TView::Component {
                comp: child,
                props,
                events,
                ..
            } => self.view_component(g, *child, props, events, pos),
        }
    }

    fn view_text(&mut self, g: &mut Gen, e: &TExpr, pos: Pos) {
        let nn = self.helper_new_node();
        let id = g.mount.temp(VT::I32);
        g.mb.push(set(id, IExpr::Call(nn, vec![])));
        let deps = self.deps(&g.mount, e);
        let text = match &e.kind {
            TK::Str(s) => S::Static(s.clone()),
            _ => S::Dyn(self.expr(&mut g.mount, e)),
        };
        let create = self.op(&mut g.mount, opc::TEXT, vec![get(id)], vec![text]);
        g.mb.push(create);
        let ins = self.insert(&mut g.mount, pos, get(id));
        g.mb.push(ins);
        let needs_field = deps != 0 || matches!(pos, Pos::Root);
        if !needs_field {
            return;
        }
        let ty = self.scope_ty(g.si);
        let f = self.add_field(g.si, VT::I32);
        g.mb.push(IExpr::StructSet(ty, f, b(get(0)), b(get(id))));
        if matches!(pos, Pos::Root) {
            g.roots.push(Root::Node(f));
        }
        if deps != 0 {
            let x = self.expr(&mut g.update, e);
            let upd = self.op(
                &mut g.update,
                opc::SET_TEXT,
                vec![IExpr::StructGet(ty, f, b(get(0)))],
                vec![S::Dyn(x)],
            );
            g.ub.push(IExpr::If(None, b(Self::dirty(deps)), vec![upd], vec![]));
        }
    }

    fn attr_cmd(
        &mut self,
        cx: &mut FnCx,
        kind: AttrKind,
        id: IExpr,
        value: IExpr,
        initial: bool,
    ) -> IExpr {
        match kind {
            AttrKind::Attr(n) => self.op(
                cx,
                opc::SET_ATTR,
                vec![id],
                vec![S::Static(n.into()), S::Dyn(value)],
            ),
            AttrKind::Prop(n) => self.op(
                cx,
                opc::SET_PROP,
                vec![id],
                vec![S::Static(n.into()), S::Dyn(value)],
            ),
            AttrKind::Route => {
                let id_concat = self
                    .fns
                    .iter()
                    .position(|f| f.name == "str_concat")
                    .unwrap();
                let f = self.fn_inst(id_concat, &[]);
                let hash = self.string_const("#");
                let v = IExpr::Call(FuncRef::Local(f), vec![hash, value]);
                self.op(
                    cx,
                    opc::SET_ATTR,
                    vec![id],
                    vec![S::Static("href".into()), S::Dyn(v)],
                )
            }
            AttrKind::Checked => self.op(cx, opc::SET_CHECKED, vec![id, value], vec![]),
            AttrKind::BoolAttr(n) => {
                let on = self.op(
                    cx,
                    opc::SET_ATTR,
                    vec![id.clone()],
                    vec![S::Static(n.into()), S::Static(String::new())],
                );
                let off = if initial {
                    IExpr::Seq(vec![])
                } else {
                    self.op(cx, opc::REMOVE_ATTR, vec![id], vec![S::Static(n.into())])
                };
                IExpr::If(None, b(value), vec![on], vec![off])
            }
        }
    }

    fn view_element(&mut self, g: &mut Gen, comp: &TComp, el: &TElem, pos: Pos) {
        let ty = self.scope_ty(g.si);
        let nn = self.helper_new_node();
        let id = g.mount.temp(VT::I32);
        g.mb.push(set(id, IExpr::Call(nn, vec![])));
        let create = self.op(
            &mut g.mount,
            opc::CREATE,
            vec![get(id)],
            vec![S::Static(el.tag.clone())],
        );
        g.mb.push(create);
        // Checkbox: events and value attributes target an inner <input type=checkbox>.
        let target = match &el.special {
            Special::Checkbox(_) => {
                let input = g.mount.temp(VT::I32);
                g.mb.push(set(input, IExpr::Call(nn, vec![])));
                let c = self.op(
                    &mut g.mount,
                    opc::CREATE,
                    vec![get(input)],
                    vec![S::Static("input".into())],
                );
                let t = self.op(
                    &mut g.mount,
                    opc::SET_ATTR,
                    vec![get(input)],
                    vec![S::Static("type".into()), S::Static("checkbox".into())],
                );
                let ins = self.op(
                    &mut g.mount,
                    opc::INSERT,
                    vec![get(id), get(input), IExpr::I32(0)],
                    vec![],
                );
                g.mb.extend([c, t, ins]);
                input
            }
            _ => id,
        };
        for (n, v) in &el.static_attrs {
            let c = self.op(
                &mut g.mount,
                opc::SET_ATTR,
                vec![get(id)],
                vec![S::Static(n.to_string()), S::Static(v.clone())],
            );
            g.mb.push(c);
        }
        if !el.classes.is_empty() {
            let c = self.op(
                &mut g.mount,
                opc::CLASS,
                vec![get(id)],
                vec![S::Static(el.classes.join(" "))],
            );
            g.mb.push(c);
        }
        // Field holding the target node id, created on first need.
        let mut field: Option<u32> = None;
        let mut ensure_field = |this: &mut Self, g: &mut Gen| -> u32 {
            *field.get_or_insert_with(|| {
                let f = this.add_field(g.si, VT::I32);
                g.mb.push(IExpr::StructSet(ty, f, b(get(0)), b(get(target))));
                f
            })
        };
        if let Special::Select(opts) = &el.special {
            let fill = self.select_options(&mut g.mount, get(target), opts);
            g.mb.push(fill);
            let deps = self.deps(&g.mount, opts);
            if deps != 0 {
                let f = ensure_field(self, g);
                let tid = IExpr::StructGet(ty, f, b(get(0)));
                let clear = self.op(&mut g.update, opc::CLEAR, vec![tid.clone()], vec![]);
                let fill = self.select_options(&mut g.update, tid.clone(), opts);
                // Re-apply the selected value after rebuilding the options.
                let mut reapply = Vec::new();
                for (k, v) in &el.attrs {
                    if *k == AttrKind::Prop("value") {
                        let x = self.expr(&mut g.update, v);
                        reapply.push(self.attr_cmd(&mut g.update, *k, tid.clone(), x, false));
                    }
                }
                let mut body = vec![clear, fill];
                body.extend(reapply);
                g.ub.push(IExpr::If(None, b(Self::dirty(deps)), body, vec![]));
            }
        }
        for (kind, v) in &el.attrs {
            let x = self.expr(&mut g.mount, v);
            let c = self.attr_cmd(&mut g.mount, *kind, get(target), x, true);
            g.mb.push(c);
            let deps = self.deps(&g.mount, v);
            if deps != 0 {
                let f = ensure_field(self, g);
                let x = self.expr(&mut g.update, v);
                let c = self.attr_cmd(
                    &mut g.update,
                    *kind,
                    IExpr::StructGet(ty, f, b(get(0))),
                    x,
                    false,
                );
                g.ub.push(IExpr::If(None, b(Self::dirty(deps)), vec![c], vec![]));
            }
        }
        // Class toggles are attached to the outer element.
        let mut outer_field: Option<u32> = None;
        for (class, cond) in &el.class_ifs {
            let x = self.expr(&mut g.mount, cond);
            let c = self.op(
                &mut g.mount,
                opc::TOGGLE_CLASS,
                vec![get(id), x],
                vec![S::Static(class.clone())],
            );
            g.mb.push(c);
            let deps = self.deps(&g.mount, cond);
            if deps != 0 {
                let f = if target == id {
                    ensure_field(self, g)
                } else {
                    *outer_field.get_or_insert_with(|| {
                        let f = self.add_field(g.si, VT::I32);
                        g.mb.push(IExpr::StructSet(ty, f, b(get(0)), b(get(id))));
                        f
                    })
                };
                let x = self.expr(&mut g.update, cond);
                let c = self.op(
                    &mut g.update,
                    opc::TOGGLE_CLASS,
                    vec![IExpr::StructGet(ty, f, b(get(0))), x],
                    vec![S::Static(class.clone())],
                );
                g.ub.push(IExpr::If(None, b(Self::dirty(deps)), vec![c], vec![]));
            }
        }
        // Events: one handler slot each, released on destroy.
        for (spec, h) in &el.events {
            let ci = self.u().scopes[g.si].comp;
            let kind = self.handler_kind(ci, *h, g.si);
            let alloc = self.helper_alloc_slot();
            let release = self.helper_release_slot();
            let sf = self.add_field(g.si, VT::I32);
            let slot = g.mount.temp(VT::I32);
            g.mb.push(set(
                slot,
                IExpr::Call(alloc, vec![get(0), IExpr::I32(kind as i32)]),
            ));
            g.mb.push(IExpr::StructSet(ty, sf, b(get(0)), b(get(slot))));
            let l = self.op(
                &mut g.mount,
                opc::LISTEN,
                vec![get(target), get(slot)],
                vec![S::Static(spec.clone())],
            );
            g.mb.push(l);
            g.db.push(IExpr::Call(
                release,
                vec![IExpr::StructGet(ty, sf, b(get(0)))],
            ));
        }
        if let Special::Checkbox(label) = &el.special {
            self.view_text(g, label, Pos::Elem(id));
        }
        for c in &el.children {
            self.view(g, comp, c, Pos::Elem(id));
        }
        let ins = self.insert(&mut g.mount, pos, get(id));
        g.mb.push(ins);
        if matches!(pos, Pos::Root) {
            let f = if target == id {
                ensure_field(self, g)
            } else {
                outer_field.unwrap_or_else(|| {
                    let f = self.add_field(g.si, VT::I32);
                    g.mb.push(IExpr::StructSet(ty, f, b(get(0)), b(get(id))));
                    f
                })
            };
            g.roots.push(Root::Node(f));
        }
    }

    /// Appends one `<option>` per string of `opts` to the select element `sel`.
    fn select_options(&mut self, cx: &mut FnCx, sel: IExpr, opts: &TExpr) -> IExpr {
        let arr_ty = self.ty(cx, &opts.ty);
        let VT::Ref(at) = self.vt(&arr_ty).unwrap() else {
            unreachable!()
        };
        let a = cx.temp(VT::Ref(at));
        let i = cx.temp(VT::I32);
        let s = cx.temp(VT::I32);
        let o = cx.temp(VT::I32);
        let t = cx.temp(VT::I32);
        let txt = cx.temp(VT::Ref(self.str_ty));
        let nn = self.helper_new_node();
        let arr = self.expr(cx, opts);
        let create = self.op(
            cx,
            opc::CREATE,
            vec![get(o)],
            vec![S::Static("option".into())],
        );
        let val = self.op(
            cx,
            opc::SET_ATTR,
            vec![get(o)],
            vec![S::Static("value".into()), S::Dyn(get(txt))],
        );
        let text = self.op(cx, opc::TEXT, vec![get(t)], vec![S::Dyn(get(txt))]);
        let ins_t = self.op(cx, opc::INSERT, vec![get(o), get(t), IExpr::I32(0)], vec![]);
        let ins_o = self.op(cx, opc::INSERT, vec![get(s), get(o), IExpr::I32(0)], vec![]);
        let l = cx.label();
        let l2 = cx.label();
        IExpr::Seq(vec![
            set(s, sel),
            set(a, arr),
            set(i, IExpr::I32(0)),
            IExpr::Block(
                l,
                None,
                vec![IExpr::Loop(
                    l2,
                    vec![
                        IExpr::BrIf(l, b(bin(Op::I32GeU, get(i), IExpr::ArrayLen(b(get(a)))))),
                        set(txt, IExpr::ArrayGet(at, b(get(a)), b(get(i)))),
                        set(o, IExpr::Call(nn, vec![])),
                        set(t, IExpr::Call(nn, vec![])),
                        create,
                        val,
                        text,
                        ins_t,
                        ins_o,
                        set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                        IExpr::Br(l2),
                    ],
                )],
            ),
        ])
    }

    /// Creates the anchor comment for a block and the fields every block needs.
    fn block_anchor(&mut self, g: &mut Gen, pos: Pos) -> (u32, u32) {
        let ty = self.scope_ty(g.si);
        let nn = self.helper_new_node();
        let a = g.mount.temp(VT::I32);
        g.mb.push(set(a, IExpr::Call(nn, vec![])));
        let c = self.op(&mut g.mount, opc::COMMENT, vec![get(a)], vec![]);
        g.mb.push(c);
        let ins = self.insert(&mut g.mount, pos, get(a));
        g.mb.push(ins);
        let fa = self.add_field(g.si, VT::I32);
        let fp = self.add_field(g.si, VT::I32);
        g.mb.push(IExpr::StructSet(ty, fa, b(get(0)), b(get(a))));
        g.mb.push(IExpr::StructSet(
            ty,
            fp,
            b(get(0)),
            b(Self::parent_node(pos)),
        ));
        (fa, fp)
    }

    fn view_switch(
        &mut self,
        g: &mut Gen,
        comp: &TComp,
        kind: &SwitchKind,
        branches: &[Vec<TView>],
        pos: Pos,
    ) {
        let si = g.si;
        let ty = self.scope_ty(si);
        let (fa, fp) = self.block_anchor(g, pos);
        let fidx = self.add_field(si, VT::I32);
        // One fragment scope per branch.
        let mut bscopes = Vec::new();
        for br in branches {
            let ci = self.u().scopes[si].comp;
            let bs = self.new_scope(ci, Some(si));
            let bty = self.scope_ty(bs);
            let bf = self.add_field(si, VT::Ref(bty));
            bscopes.push((bs, bf, br.clone()));
        }
        // Match bindings live in the branch fragment.
        let pats: Vec<Option<TPat>> = match kind {
            SwitchKind::Match(_, pats) => pats.iter().cloned().map(Some).collect(),
            _ => branches.iter().map(|_| None).collect(),
        };
        for ((bs, _, _), p) in bscopes.iter().zip(&pats) {
            if let Some(p) = p {
                for id in pat_binds(p) {
                    let t = comp.locals[id].ty.clone();
                    let vt = self.vt(&t).unwrap_or(VT::I32);
                    let f = self.add_field(*bs, vt);
                    self.u().scopes[*bs].bound.insert(id, f);
                }
            }
        }
        let sel_deps = match kind {
            SwitchKind::If(x) | SwitchKind::Match(x, _) => self.deps(&g.mount, x),
            SwitchKind::Routes(_) => BIT_ROUTE,
        };
        let mut branch_deps = Vec::new();
        for (bs, _, br) in &bscopes {
            let mut probe = FnCx::new(vec![], comp.locals.clone(), vec![]);
            probe.scope = Some(ScopeRef {
                scope: *bs,
                local: 0,
            });
            branch_deps.push(self.views_deps(&probe, br));
        }
        // mount: select + create
        let (sel_m, newidx_m, scrut_m) = self.switch_select(&mut g.mount, kind);
        g.mb.push(sel_m);
        let create_m = self.switch_create(
            &mut g.mount,
            si,
            &bscopes,
            &pats,
            get(newidx_m),
            scrut_m,
            fp,
            fa,
        );
        g.mb.push(IExpr::StructSet(ty, fidx, b(get(0)), b(get(newidx_m))));
        g.mb.push(create_m);
        // update: reselect when dependencies changed
        let (sel_u, newidx_u, scrut_u) = self.switch_select(&mut g.update, kind);
        let cur = IExpr::StructGet(ty, fidx, b(get(0)));
        let destroy_old: Vec<(usize, IExpr)> = bscopes
            .iter()
            .enumerate()
            .map(|(k, (bs, bf, _))| {
                let f = self.u().scopes[*bs].fns.destroy;
                let bty = self.u().scopes[*bs].ty;
                (
                    k,
                    IExpr::Seq(vec![
                        IExpr::Call(
                            FuncRef::Local(f),
                            vec![
                                un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *bf, b(get(0)))),
                                IExpr::I32(1),
                            ],
                        ),
                        IExpr::StructSet(ty, *bf, b(get(0)), b(IExpr::Null(bty))),
                    ]),
                )
            })
            .collect();
        let create_u = self.switch_create(
            &mut g.update,
            si,
            &bscopes,
            &pats,
            get(newidx_u),
            scrut_u.clone(),
            fp,
            fa,
        );
        let update_cur: Vec<(usize, IExpr)> = bscopes
            .iter()
            .enumerate()
            .map(|(k, (bs, bf, _))| {
                let f = self.u().scopes[*bs].fns.update;
                let d = branch_deps[k];
                (
                    k,
                    IExpr::If(
                        None,
                        b(Self::dirty(d)),
                        vec![IExpr::Call(
                            FuncRef::Local(f),
                            vec![
                                un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *bf, b(get(0)))),
                                get(1),
                            ],
                        )],
                        vec![],
                    ),
                )
            })
            .collect();
        // Same branch of a `match`: refresh its bindings and signal BIT_BOUND.
        let rebind: Vec<(usize, IExpr)> = match (kind, scrut_u) {
            (SwitchKind::Match(..), Some((sl, sty))) => bscopes
                .iter()
                .enumerate()
                .zip(&pats)
                .map(|((k, (bs, bf, _)), p)| {
                    let mut body = Vec::new();
                    let bty = self.u().scopes[*bs].ty;
                    let fr = un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *bf, b(get(0))));
                    if let Some(p) = p {
                        body.extend(self.pat_bind(&mut g.update, p, &sty, &get(sl)));
                        for id in pat_binds(p) {
                            let f = self.u().scopes[*bs].bound[&id];
                            if let Some(l) = self.local(&mut g.update, id) {
                                body.push(IExpr::StructSet(bty, f, b(fr.clone()), b(get(l))));
                            }
                        }
                    }
                    let uf = self.u().scopes[*bs].fns.update;
                    body.push(IExpr::Call(
                        FuncRef::Local(uf),
                        vec![fr, bin(Op::I64Or, get(1), IExpr::I64(BIT_BOUND))],
                    ));
                    (k, IExpr::Seq(body))
                })
                .collect(),
            _ => vec![],
        };
        let same_branch = if rebind.is_empty() {
            switch_on(cur.clone(), update_cur.clone())
        } else {
            switch_on(cur.clone(), rebind)
        };
        g.ub.push(IExpr::If(
            None,
            b(Self::dirty(sel_deps)),
            vec![
                sel_u,
                IExpr::If(
                    None,
                    b(bin(Op::I32Ne, get(newidx_u), cur.clone())),
                    vec![
                        switch_on(cur.clone(), destroy_old),
                        IExpr::StructSet(ty, fidx, b(get(0)), b(get(newidx_u))),
                        create_u,
                    ],
                    vec![same_branch],
                ),
            ],
            vec![switch_on(cur.clone(), update_cur)],
        ));
        // destroy
        let destroy_cur: Vec<(usize, IExpr)> = bscopes
            .iter()
            .enumerate()
            .map(|(k, (bs, bf, _))| {
                let f = self.u().scopes[*bs].fns.destroy;
                (
                    k,
                    IExpr::Call(
                        FuncRef::Local(f),
                        vec![
                            un(UnOp::RefAsNonNull, IExpr::StructGet(ty, *bf, b(get(0)))),
                            get(1),
                        ],
                    ),
                )
            })
            .collect();
        g.db.push(switch_on(
            IExpr::StructGet(ty, fidx, b(get(0))),
            destroy_cur,
        ));
        let rm = self.op_raw(opc::REMOVE, vec![IExpr::StructGet(ty, fa, b(get(0)))]);
        if matches!(pos, Pos::Root) {
            g.roots.push(Root::Switch {
                anchor: fa,
                idx: fidx,
                branches: bscopes.iter().map(|(s, f, _)| (*s, *f)).collect(),
            });
        } else {
            let _ = rm;
        }
        for (bs, _, br) in bscopes {
            self.gen_scope(bs, comp, &br);
        }
    }

    /// Computes the selected branch index into a fresh local; returns (code, idx local, scrutinee local).
    fn switch_select(
        &mut self,
        cx: &mut FnCx,
        kind: &SwitchKind,
    ) -> (IExpr, u32, Option<(u32, Type)>) {
        let idx = cx.temp(VT::I32);
        match kind {
            SwitchKind::If(c) => {
                let x = self.expr(cx, c);
                (
                    set(
                        idx,
                        IExpr::If(
                            Some(VT::I32),
                            b(x),
                            vec![IExpr::I32(0)],
                            vec![IExpr::I32(1)],
                        ),
                    ),
                    idx,
                    None,
                )
            }
            SwitchKind::Routes(paths) => {
                let route = self.u().g.route;
                let id = self.fns.iter().position(|f| f.name == "str_eq").unwrap();
                let eq = self.fn_inst(id, &[]);
                let mut e = IExpr::I32(paths.len() as i32 - 1);
                for (k, p) in paths.iter().enumerate().rev() {
                    if let Some(p) = p {
                        let lit = self.string_const(p);
                        let test = IExpr::Call(
                            FuncRef::Local(eq),
                            vec![un(UnOp::RefAsNonNull, IExpr::GlobalGet(route)), lit],
                        );
                        e = IExpr::If(Some(VT::I32), b(test), vec![IExpr::I32(k as i32)], vec![e]);
                    }
                }
                (set(idx, e), idx, None)
            }
            SwitchKind::Match(s, pats) => {
                let st = self.ty(cx, &s.ty);
                let vt = self.vt(&st).unwrap();
                let sl = cx.temp(vt);
                let x = self.expr(cx, s);
                let mut e = IExpr::Unreachable;
                for (k, p) in pats.iter().enumerate().rev() {
                    e = match self.pat_test(cx, p, &st, &get(sl)) {
                        None => IExpr::I32(k as i32),
                        Some(t) => {
                            IExpr::If(Some(VT::I32), b(t), vec![IExpr::I32(k as i32)], vec![e])
                        }
                    };
                }
                (
                    IExpr::Seq(vec![set(sl, x), set(idx, e)]),
                    idx,
                    Some((sl, st)),
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn switch_create(
        &mut self,
        cx: &mut FnCx,
        si: usize,
        bscopes: &[(usize, u32, Vec<TView>)],
        pats: &[Option<TPat>],
        idx: IExpr,
        scrut: Option<(u32, Type)>,
        fp: u32,
        fa: u32,
    ) -> IExpr {
        let ty = self.scope_ty(si);
        let mut arms = Vec::new();
        for (k, ((bs, bf, _), p)) in bscopes.iter().zip(pats).enumerate() {
            let bty = self.scope_ty(*bs);
            let fns = self.u().scopes[*bs].fns;
            let f = cx.temp(VT::Ref(bty));
            let mut body = vec![
                set(f, IExpr::StructNewDefault(bty)),
                IExpr::StructSet(bty, 0, b(get(f)), b(get(0))),
            ];
            if let (Some(p), Some((sl, sty))) = (p, &scrut) {
                body.extend(self.pat_bind(cx, p, sty, &get(*sl)));
                for id in pat_binds(p) {
                    let fld = self.u().scopes[*bs].bound[&id];
                    if let Some(l) = self.local(cx, id) {
                        body.push(IExpr::StructSet(bty, fld, b(get(f)), b(get(l))));
                    }
                }
            }
            body.push(IExpr::Call(
                FuncRef::Local(fns.mount),
                vec![
                    get(f),
                    IExpr::StructGet(ty, fp, b(get(0))),
                    IExpr::StructGet(ty, fa, b(get(0))),
                ],
            ));
            body.push(IExpr::StructSet(ty, *bf, b(get(0)), b(get(f))));
            arms.push((k, IExpr::Seq(body)));
        }
        switch_on(idx, arms)
    }

    #[allow(clippy::too_many_arguments)]
    fn view_for(
        &mut self,
        g: &mut Gen,
        comp: &TComp,
        item: LocalId,
        index: Option<LocalId>,
        iter: &TExpr,
        key: Option<&TExpr>,
        body: &[TView],
        span: Span,
        pos: Pos,
    ) {
        let si = g.si;
        let ty = self.scope_ty(si);
        let (fa, fp) = self.block_anchor(g, pos);
        let ci = self.u().scopes[si].comp;
        let fs = self.new_scope(ci, Some(si));
        let fty = self.scope_ty(fs);
        let arr_frags = self.m.array_type(Storage::Val(VT::RefNull(fty)));
        let ffrags = self.add_field(si, VT::Ref(arr_frags));
        let fcount = self.add_field(si, VT::I32);
        let item_ty = comp.locals[item].ty.clone();
        let item_vt = self.vt(&item_ty).unwrap_or(VT::I32);
        let f_item = self.add_field(fs, item_vt);
        self.u().scopes[fs].bound.insert(item, f_item);
        let f_index = self.add_field(fs, VT::I64);
        if let Some(ix) = index {
            self.u().scopes[fs].bound.insert(ix, f_index);
        }
        let key_ty = key.map(|k| k.ty.clone()).unwrap_or(Type::Int);
        let key_vt = self.vt(&key_ty).unwrap();
        let f_key = self.add_field(fs, key_vt);
        self.u().scopes[fs].key_field = Some(f_key);
        let ffns = self.u().scopes[fs].fns;
        let iter_ty = self.ty(&g.mount, &iter.ty);
        let VT::Ref(src_arr) = self.vt(&iter_ty).unwrap() else {
            unreachable!()
        };
        let iter_deps = self.deps(&g.mount, iter);
        let mut probe = FnCx::new(vec![], comp.locals.clone(), vec![]);
        probe.scope = Some(ScopeRef {
            scope: fs,
            local: 0,
        });
        let body_deps = self.views_deps(&probe, body);
        let loc = {
            let sm = &self.u().sm;
            let (l, c) = sm.line_col(span.lo);
            format!("{}:{l}:{c}", sm.file_of(span.lo).path)
        };

        // Key of the item held in wasm local `il` at position `ix` (i64), evaluated in `cx`.
        let key_of = |this: &mut Self, cx: &mut FnCx, il: u32, ix: u32| -> IExpr {
            match key {
                None => get(ix),
                Some(k) => {
                    cx.unbound.push(item);
                    cx.map.insert(item, il);
                    if let Some(i) = index {
                        cx.unbound.push(i);
                        cx.map.insert(i, ix);
                    }
                    let x = this.expr(cx, k);
                    cx.unbound.retain(|u| *u != item && Some(*u) != index);
                    cx.map.remove(&item);
                    if let Some(i) = index {
                        cx.map.remove(&i);
                    }
                    x
                }
            }
        };
        let frags_of = |s: IExpr| un(UnOp::RefAsNonNull, IExpr::StructGet(ty, ffrags, b(s)));

        // ----- mount: create every item -----
        {
            let cx = &mut g.mount;
            let a = cx.temp(VT::Ref(src_arr));
            let n = cx.temp(VT::I32);
            let i = cx.temp(VT::I32);
            let il = cx.temp(item_vt);
            let ix = cx.temp(VT::I64);
            let f = cx.temp(VT::Ref(fty));
            let arr = self.expr(cx, iter);
            let key_x = key_of(self, cx, il, ix);
            let dup = if self.u().dev && key.is_some() {
                self.dup_key_check(
                    cx,
                    arr_frags,
                    fty,
                    f_key,
                    &key_ty,
                    frags_of(get(0)),
                    get(f),
                    get(i),
                    &loc,
                )
            } else {
                IExpr::Seq(vec![])
            };
            let l1 = cx.label();
            let l2 = cx.label();
            g.mb.push(IExpr::Seq(vec![
                set(a, arr),
                set(n, IExpr::ArrayLen(b(get(a)))),
                IExpr::StructSet(
                    ty,
                    ffrags,
                    b(get(0)),
                    b(IExpr::ArrayNewDefault(arr_frags, b(get(n)))),
                ),
                IExpr::StructSet(ty, fcount, b(get(0)), b(get(n))),
                set(i, IExpr::I32(0)),
                IExpr::Block(
                    l1,
                    None,
                    vec![IExpr::Loop(
                        l2,
                        vec![
                            IExpr::BrIf(l1, b(bin(Op::I32GeU, get(i), get(n)))),
                            set(il, IExpr::ArrayGet(src_arr, b(get(a)), b(get(i)))),
                            set(ix, un(UnOp::I64ExtendI32S, get(i))),
                            set(f, IExpr::StructNewDefault(fty)),
                            IExpr::StructSet(fty, 0, b(get(f)), b(get(0))),
                            IExpr::StructSet(fty, f_item, b(get(f)), b(get(il))),
                            IExpr::StructSet(fty, f_index, b(get(f)), b(get(ix))),
                            IExpr::StructSet(fty, f_key, b(get(f)), b(key_x)),
                            dup,
                            IExpr::Call(
                                FuncRef::Local(ffns.mount),
                                vec![
                                    get(f),
                                    IExpr::StructGet(ty, fp, b(get(0))),
                                    IExpr::StructGet(ty, fa, b(get(0))),
                                ],
                            ),
                            IExpr::ArraySet(arr_frags, b(frags_of(get(0))), b(get(i)), b(get(f))),
                            set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                            IExpr::Br(l2),
                        ],
                    )],
                ),
            ]));
        }

        // ----- update: keyed reconciliation when the list changed, else plain updates -----
        {
            let cx = &mut g.update;
            let i32_arr = self.u().i32_arr;
            let a = cx.temp(VT::Ref(src_arr));
            let n = cx.temp(VT::I32);
            let on = cx.temp(VT::I32);
            let old = cx.temp(VT::Ref(arr_frags));
            let new = cx.temp(VT::Ref(arr_frags));
            let used = cx.temp(VT::Ref(i32_arr));
            let src = cx.temp(VT::Ref(i32_arr));
            let i = cx.temp(VT::I32);
            let j = cx.temp(VT::I32);
            let found = cx.temp(VT::I32);
            let il = cx.temp(item_vt);
            let ix = cx.temp(VT::I64);
            let kl = cx.temp(key_vt);
            let f = cx.temp(VT::Ref(fty));
            let anchor = cx.temp(VT::I32);
            let mask = cx.temp(VT::I64);
            let arr = self.expr(cx, iter);
            let key_x = key_of(self, cx, il, ix);
            let key_neq = self.values_differ(
                &key_ty,
                IExpr::StructGet(
                    fty,
                    f_key,
                    b(un(
                        UnOp::RefAsNonNull,
                        IExpr::ArrayGet(arr_frags, b(get(old)), b(get(j))),
                    )),
                ),
                get(kl),
            );
            let item_changed =
                self.values_differ(&item_ty, IExpr::StructGet(fty, f_item, b(get(f))), get(il));
            let dup = if self.u().dev && key.is_some() {
                self.dup_key_check(
                    cx,
                    arr_frags,
                    fty,
                    f_key,
                    &key_ty,
                    get(new),
                    get(f),
                    get(i),
                    &loc,
                )
            } else {
                IExpr::Seq(vec![])
            };
            let lb = |cx: &mut FnCx| (cx.label(), cx.label());
            let (b1, l1) = lb(cx);
            let (b2, l2) = lb(cx);
            let (b3, l3) = lb(cx);
            let (b4, l4) = lb(cx);
            let (b5, l5) = lb(cx);
            let (b6, l6) = lb(cx);
            let (b7, l7) = lb(cx);
            let (len, lo, hi, mid) = (
                cx.temp(VT::I32),
                cx.temp(VT::I32),
                cx.temp(VT::I32),
                cx.temp(VT::I32),
            );
            let (pred, tails) = (cx.temp(VT::Ref(i32_arr)), cx.temp(VT::Ref(i32_arr)));
            let src_at = |at: IExpr| IExpr::ArrayGet(i32_arr, b(get(src)), b(at));
            let pnode = IExpr::StructGet(ty, fp, b(get(0)));
            let reconcile = vec![
                set(a, arr),
                set(n, IExpr::ArrayLen(b(get(a)))),
                set(old, frags_of(get(0))),
                set(on, IExpr::StructGet(ty, fcount, b(get(0)))),
                set(new, IExpr::ArrayNewDefault(arr_frags, b(get(n)))),
                set(used, IExpr::ArrayNewDefault(i32_arr, b(get(on)))),
                set(src, IExpr::ArrayNewDefault(i32_arr, b(get(n)))),
                set(i, IExpr::I32(0)),
                // Match each new item to an unused old fragment with the same key.
                IExpr::Block(
                    b1,
                    None,
                    vec![IExpr::Loop(
                        l1,
                        vec![
                            IExpr::BrIf(b1, b(bin(Op::I32GeU, get(i), get(n)))),
                            set(il, IExpr::ArrayGet(src_arr, b(get(a)), b(get(i)))),
                            set(ix, un(UnOp::I64ExtendI32S, get(i))),
                            set(kl, key_x),
                            set(found, IExpr::I32(-1)),
                            // Try the same position first, then scan.
                            set(j, get(i)),
                            IExpr::If(
                                None,
                                b(bin(Op::I32LtS, get(j), get(on))),
                                vec![IExpr::If(
                                    None,
                                    b(un(
                                        UnOp::I32Eqz,
                                        IExpr::ArrayGet(i32_arr, b(get(used)), b(get(j))),
                                    )),
                                    vec![IExpr::If(
                                        None,
                                        b(un(UnOp::I32Eqz, key_neq.clone())),
                                        vec![set(found, get(j))],
                                        vec![],
                                    )],
                                    vec![],
                                )],
                                vec![],
                            ),
                            set(j, IExpr::I32(0)),
                            IExpr::Block(
                                b2,
                                None,
                                vec![IExpr::Loop(
                                    l2,
                                    vec![
                                        IExpr::BrIf(
                                            b2,
                                            b(bin(Op::I32GeS, get(found), IExpr::I32(0))),
                                        ),
                                        IExpr::BrIf(b2, b(bin(Op::I32GeU, get(j), get(on)))),
                                        IExpr::If(
                                            None,
                                            b(un(
                                                UnOp::I32Eqz,
                                                IExpr::ArrayGet(i32_arr, b(get(used)), b(get(j))),
                                            )),
                                            vec![IExpr::If(
                                                None,
                                                b(un(UnOp::I32Eqz, key_neq)),
                                                vec![set(found, get(j))],
                                                vec![],
                                            )],
                                            vec![],
                                        ),
                                        set(j, bin(Op::I32Add, get(j), IExpr::I32(1))),
                                        IExpr::Br(l2),
                                    ],
                                )],
                            ),
                            IExpr::If(
                                None,
                                b(bin(Op::I32GeS, get(found), IExpr::I32(0))),
                                vec![
                                    IExpr::ArraySet(
                                        i32_arr,
                                        b(get(used)),
                                        b(get(found)),
                                        b(IExpr::I32(1)),
                                    ),
                                    IExpr::ArraySet(
                                        i32_arr,
                                        b(get(src)),
                                        b(get(i)),
                                        b(bin(Op::I32Add, get(found), IExpr::I32(1))),
                                    ),
                                    set(
                                        f,
                                        un(
                                            UnOp::RefAsNonNull,
                                            IExpr::ArrayGet(arr_frags, b(get(old)), b(get(found))),
                                        ),
                                    ),
                                    set(mask, get(1)),
                                    IExpr::If(
                                        None,
                                        b(item_changed),
                                        vec![
                                            IExpr::StructSet(fty, f_item, b(get(f)), b(get(il))),
                                            set(
                                                mask,
                                                bin(Op::I64Or, get(mask), IExpr::I64(BIT_BOUND)),
                                            ),
                                        ],
                                        vec![],
                                    ),
                                    IExpr::If(
                                        None,
                                        b(bin(
                                            Op::I64Ne,
                                            IExpr::StructGet(fty, f_index, b(get(f))),
                                            get(ix),
                                        )),
                                        vec![
                                            IExpr::StructSet(fty, f_index, b(get(f)), b(get(ix))),
                                            set(
                                                mask,
                                                bin(Op::I64Or, get(mask), IExpr::I64(BIT_BOUND)),
                                            ),
                                        ],
                                        vec![],
                                    ),
                                    IExpr::If(
                                        None,
                                        b(bin(
                                            Op::I64Ne,
                                            bin(Op::I64And, get(mask), IExpr::I64(body_deps)),
                                            IExpr::I64(0),
                                        )),
                                        vec![IExpr::Call(
                                            FuncRef::Local(ffns.update),
                                            vec![get(f), get(mask)],
                                        )],
                                        vec![],
                                    ),
                                ],
                                vec![
                                    set(f, IExpr::StructNewDefault(fty)),
                                    IExpr::StructSet(fty, 0, b(get(f)), b(get(0))),
                                    IExpr::StructSet(fty, f_item, b(get(f)), b(get(il))),
                                    IExpr::StructSet(fty, f_index, b(get(f)), b(get(ix))),
                                    IExpr::StructSet(fty, f_key, b(get(f)), b(get(kl))),
                                ],
                            ),
                            dup,
                            IExpr::ArraySet(arr_frags, b(get(new)), b(get(i)), b(get(f))),
                            set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                            IExpr::Br(l1),
                        ],
                    )],
                ),
                // Destroy fragments whose key disappeared.
                set(j, IExpr::I32(0)),
                IExpr::Block(
                    b3,
                    None,
                    vec![IExpr::Loop(
                        l3,
                        vec![
                            IExpr::BrIf(b3, b(bin(Op::I32GeU, get(j), get(on)))),
                            IExpr::If(
                                None,
                                b(un(
                                    UnOp::I32Eqz,
                                    IExpr::ArrayGet(i32_arr, b(get(used)), b(get(j))),
                                )),
                                vec![IExpr::Call(
                                    FuncRef::Local(ffns.destroy),
                                    vec![
                                        un(
                                            UnOp::RefAsNonNull,
                                            IExpr::ArrayGet(arr_frags, b(get(old)), b(get(j))),
                                        ),
                                        IExpr::I32(1),
                                    ],
                                )],
                                vec![],
                            ),
                            set(j, bin(Op::I32Add, get(j), IExpr::I32(1))),
                            IExpr::Br(l3),
                        ],
                    )],
                ),
                // Keep the longest run of fragments whose old order is preserved (longest
                // increasing subsequence of old indices); every other kept fragment moves: src < 0.
                set(len, IExpr::I32(0)),
                set(pred, IExpr::ArrayNewDefault(i32_arr, b(get(n)))),
                set(tails, IExpr::ArrayNewDefault(i32_arr, b(get(n)))),
                set(i, IExpr::I32(0)),
                IExpr::Block(
                    b4,
                    None,
                    vec![IExpr::Loop(
                        l4,
                        vec![
                            IExpr::BrIf(b4, b(bin(Op::I32GeU, get(i), get(n)))),
                            set(j, src_at(get(i))),
                            IExpr::If(
                                None,
                                b(bin(Op::I32GtS, get(j), IExpr::I32(0))),
                                vec![
                                    // Binary search: first tail whose value is >= j.
                                    set(lo, IExpr::I32(0)),
                                    set(hi, get(len)),
                                    IExpr::Block(
                                        b6,
                                        None,
                                        vec![IExpr::Loop(
                                            l6,
                                            vec![
                                                IExpr::BrIf(
                                                    b6,
                                                    b(bin(Op::I32GeS, get(lo), get(hi))),
                                                ),
                                                set(
                                                    mid,
                                                    bin(
                                                        Op::I32ShrU,
                                                        bin(Op::I32Add, get(lo), get(hi)),
                                                        IExpr::I32(1),
                                                    ),
                                                ),
                                                // Entries already visited hold their negated old index.
                                                IExpr::If(
                                                    None,
                                                    b(bin(
                                                        Op::I32LtS,
                                                        bin(
                                                            Op::I32Sub,
                                                            IExpr::I32(0),
                                                            src_at(IExpr::ArrayGet(
                                                                i32_arr,
                                                                b(get(tails)),
                                                                b(get(mid)),
                                                            )),
                                                        ),
                                                        get(j),
                                                    )),
                                                    vec![set(
                                                        lo,
                                                        bin(Op::I32Add, get(mid), IExpr::I32(1)),
                                                    )],
                                                    vec![set(hi, get(mid))],
                                                ),
                                                IExpr::Br(l6),
                                            ],
                                        )],
                                    ),
                                    IExpr::If(
                                        None,
                                        b(bin(Op::I32GtS, get(lo), IExpr::I32(0))),
                                        vec![IExpr::ArraySet(
                                            i32_arr,
                                            b(get(pred)),
                                            b(get(i)),
                                            b(IExpr::ArrayGet(
                                                i32_arr,
                                                b(get(tails)),
                                                b(bin(Op::I32Sub, get(lo), IExpr::I32(1))),
                                            )),
                                        )],
                                        vec![],
                                    ),
                                    IExpr::ArraySet(i32_arr, b(get(tails)), b(get(lo)), b(get(i))),
                                    IExpr::If(
                                        None,
                                        b(bin(Op::I32Eq, get(lo), get(len))),
                                        vec![set(len, bin(Op::I32Add, get(len), IExpr::I32(1)))],
                                        vec![],
                                    ),
                                    // Provisionally mark as moved; the subsequence is restored below.
                                    IExpr::ArraySet(
                                        i32_arr,
                                        b(get(src)),
                                        b(get(i)),
                                        b(bin(Op::I32Sub, IExpr::I32(0), get(j))),
                                    ),
                                ],
                                vec![],
                            ),
                            set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                            IExpr::Br(l4),
                        ],
                    )],
                ),
                IExpr::If(
                    None,
                    b(bin(Op::I32GtS, get(len), IExpr::I32(0))),
                    vec![
                        set(
                            i,
                            IExpr::ArrayGet(
                                i32_arr,
                                b(get(tails)),
                                b(bin(Op::I32Sub, get(len), IExpr::I32(1))),
                            ),
                        ),
                        IExpr::Block(
                            b7,
                            None,
                            vec![IExpr::Loop(
                                l7,
                                vec![
                                    IExpr::BrIf(b7, b(bin(Op::I32LeS, get(len), IExpr::I32(0)))),
                                    IExpr::ArraySet(
                                        i32_arr,
                                        b(get(src)),
                                        b(get(i)),
                                        b(bin(Op::I32Sub, IExpr::I32(0), src_at(get(i)))),
                                    ),
                                    set(i, IExpr::ArrayGet(i32_arr, b(get(pred)), b(get(i)))),
                                    set(len, bin(Op::I32Sub, get(len), IExpr::I32(1))),
                                    IExpr::Br(l7),
                                ],
                            )],
                        ),
                    ],
                    vec![],
                ),
                // Place from the end: mount new fragments and move displaced ones before the next sibling.
                set(anchor, IExpr::StructGet(ty, fa, b(get(0)))),
                set(i, get(n)),
                IExpr::Block(
                    b5,
                    None,
                    vec![IExpr::Loop(
                        l5,
                        vec![
                            IExpr::BrIf(b5, b(bin(Op::I32LeS, get(i), IExpr::I32(0)))),
                            set(i, bin(Op::I32Sub, get(i), IExpr::I32(1))),
                            set(
                                f,
                                un(
                                    UnOp::RefAsNonNull,
                                    IExpr::ArrayGet(arr_frags, b(get(new)), b(get(i))),
                                ),
                            ),
                            set(j, IExpr::ArrayGet(i32_arr, b(get(src)), b(get(i)))),
                            IExpr::If(
                                None,
                                b(un(UnOp::I32Eqz, get(j))),
                                vec![IExpr::Call(
                                    FuncRef::Local(ffns.mount),
                                    vec![get(f), pnode.clone(), get(anchor)],
                                )],
                                vec![IExpr::If(
                                    None,
                                    b(bin(Op::I32LtS, get(j), IExpr::I32(0))),
                                    vec![IExpr::Call(
                                        FuncRef::Local(ffns.mv),
                                        vec![get(f), pnode.clone(), get(anchor)],
                                    )],
                                    vec![],
                                )],
                            ),
                            set(j, IExpr::Call(FuncRef::Local(ffns.first), vec![get(f)])),
                            IExpr::If(
                                None,
                                b(bin(Op::I32GeS, get(j), IExpr::I32(0))),
                                vec![set(anchor, get(j))],
                                vec![],
                            ),
                            IExpr::Br(l5),
                        ],
                    )],
                ),
                IExpr::StructSet(ty, ffrags, b(get(0)), b(get(new))),
                IExpr::StructSet(ty, fcount, b(get(0)), b(get(n))),
            ];
            let plain = for_range(
                i,
                IExpr::StructGet(ty, fcount, b(get(0))),
                vec![IExpr::Call(
                    FuncRef::Local(ffns.update),
                    vec![
                        un(
                            UnOp::RefAsNonNull,
                            IExpr::ArrayGet(arr_frags, b(frags_of(get(0))), b(get(i))),
                        ),
                        get(1),
                    ],
                )],
                cx.label(),
            );
            g.ub.push(IExpr::If(
                None,
                b(Self::dirty(iter_deps)),
                reconcile,
                vec![IExpr::If(
                    None,
                    b(Self::dirty(body_deps)),
                    vec![plain],
                    vec![],
                )],
            ));
        }

        // ----- destroy -----
        {
            let cx = &mut g.destroy;
            let i = cx.temp(VT::I32);
            let l = cx.label();
            g.db.push(for_range(
                i,
                IExpr::StructGet(ty, fcount, b(get(0))),
                vec![IExpr::Call(
                    FuncRef::Local(ffns.destroy),
                    vec![
                        un(
                            UnOp::RefAsNonNull,
                            IExpr::ArrayGet(arr_frags, b(frags_of(get(0))), b(get(i))),
                        ),
                        get(1),
                    ],
                )],
                l,
            ));
        }
        if matches!(pos, Pos::Root) {
            g.roots.push(Root::List {
                anchor: fa,
                frags: ffrags,
                count: fcount,
                scope: fs,
            });
        }
        self.gen_scope(fs, comp, body);
    }

    /// Dev builds: warn when the key of fragment `f` (at position `i`) equals an earlier key.
    #[allow(clippy::too_many_arguments)]
    fn dup_key_check(
        &mut self,
        cx: &mut FnCx,
        arr_frags: u32,
        fty: u32,
        f_key: u32,
        key_ty: &Type,
        frags: IExpr,
        f: IExpr,
        i: IExpr,
        loc: &str,
    ) -> IExpr {
        let k = cx.temp(VT::I32);
        let fl = cx.temp(VT::Ref(fty));
        let fa = cx.temp(VT::Ref(arr_frags));
        let n = cx.temp(VT::I32);
        let warn = self.op(
            cx,
            opc::WARN,
            vec![],
            vec![S::Static(format!("duplicate key in `for` loop at {loc}"))],
        );
        let earlier = IExpr::StructGet(
            fty,
            f_key,
            b(un(
                UnOp::RefAsNonNull,
                IExpr::ArrayGet(arr_frags, b(get(fa)), b(get(k))),
            )),
        );
        let differ = self.values_differ(key_ty, earlier, IExpr::StructGet(fty, f_key, b(get(fl))));
        let l = cx.label();
        IExpr::Seq(vec![
            set(fl, f),
            set(fa, frags),
            set(n, i),
            for_range(
                k,
                get(n),
                vec![IExpr::If(None, b(differ), vec![], vec![warn])],
                l,
            ),
        ])
    }

    /// `emit name(value)`: calls the parent's listener, if any.
    pub fn ui_emit(&mut self, cx: &mut FnCx, ev: usize, value: Option<&TExpr>) -> IExpr {
        let root = self.comp_root_of(cx.scope.unwrap().scope);
        let rty = self.scope_ty(root);
        let (ff, sf, sig) = self.u().scopes[root].comp_fields.events[ev];
        let c = cx.temp(VT::Ref(rty));
        let path = self.scope_path(cx, root);
        let mut args = vec![IExpr::StructGet(rty, sf, b(get(c)))];
        if let Some(v) = value {
            let vt = self.vt(&self.ty(cx, &v.ty));
            args.push(self.expr_as(cx, v, vt));
        }
        IExpr::Seq(vec![
            set(c, un(UnOp::RefAsNonNull, path)),
            IExpr::If(
                None,
                b(un(
                    UnOp::I32Eqz,
                    un(UnOp::RefIsNull, IExpr::StructGet(rty, ff, b(get(c)))),
                )),
                vec![IExpr::CallRef(
                    sig,
                    args,
                    b(IExpr::StructGet(rty, ff, b(get(c)))),
                )],
                vec![],
            ),
        ])
    }

    /// `(anyref scope, payload)` adapter calling a parent's handler for a child event.
    fn event_wrapper(&mut self, comp: usize, h: usize, si: usize, sig: u32) -> u32 {
        let target = self.handler_fn(comp, h, si);
        let TypeDef::Func { params, .. } = self.m.types[sig as usize].clone() else {
            unreachable!()
        };
        let sty = self.scope_ty(si);
        let mut args = vec![IExpr::RefCast(sty, b(get(0)))];
        if params.len() > 1 {
            args.push(get(1));
        }
        self.m.funcs.push(Func {
            name: format!("event_wrapper_{comp}_{h}"),
            params,
            result: None,
            locals: vec![],
            body: vec![IExpr::Call(FuncRef::Local(target), args)],
            export: None,
        });
        let idx = self.m.funcs.len() as u32 - 1;
        self.m.sig_override.push((idx, sig));
        idx
    }

    fn view_component(
        &mut self,
        g: &mut Gen,
        child: usize,
        props: &[TExpr],
        events: &[(usize, usize)],
        pos: Pos,
    ) {
        let si = g.si;
        let ty = self.scope_ty(si);
        let cs = self.u().comp_scope[child];
        let cty = self.scope_ty(cs);
        let cfns = self.u().scopes[cs].fns;
        let field = self.add_field(si, VT::Ref(cty));
        let c = g.mount.temp(VT::Ref(cty));
        g.mb.push(set(c, IExpr::StructNewDefault(cty)));
        let mut prop_updates = Vec::new();
        let cm = g.update.temp(VT::I64);
        let cl = g.update.temp(VT::Ref(cty));
        for (i, p) in props.iter().enumerate() {
            let fld = Self::var_field(i);
            if matches!(p.kind, TK::Unit) && p.ty == Type::Never {
                let f = self.default_fn(child, i);
                g.mb.push(IExpr::StructSet(
                    cty,
                    fld,
                    b(get(c)),
                    b(IExpr::Call(FuncRef::Local(f), vec![])),
                ));
                continue;
            }
            let x = self.expr(&mut g.mount, p);
            g.mb.push(IExpr::StructSet(cty, fld, b(get(c)), b(x)));
            let deps = self.deps(&g.mount, p);
            if deps != 0 {
                let pt = self.u().comps[child].vars[i].ty.clone();
                let vt = self.vt(&pt).unwrap();
                let t = g.update.temp(vt);
                let x = self.expr(&mut g.update, p);
                let old = self.read_field(cs, fld, get(cl), Some(vt));
                let differ = self.values_differ(&pt, get(t), old);
                prop_updates.push(IExpr::If(
                    None,
                    b(Self::dirty(deps)),
                    vec![
                        set(t, x),
                        IExpr::If(
                            None,
                            b(differ),
                            vec![
                                IExpr::StructSet(cty, fld, b(get(cl)), b(get(t))),
                                set(cm, bin(Op::I64Or, get(cm), IExpr::I64(1i64 << i))),
                            ],
                            vec![],
                        ),
                    ],
                    vec![],
                ));
            }
        }
        let parent_comp = self.u().scopes[si].comp;
        for &(ev, h) in events {
            let (ff, sf, sig) = self.u().scopes[cs].comp_fields.events[ev];
            let w = self.event_wrapper(parent_comp, h, si, sig);
            g.mb.push(IExpr::StructSet(cty, ff, b(get(c)), b(IExpr::RefFunc(w))));
            g.mb.push(IExpr::StructSet(cty, sf, b(get(c)), b(get(0))));
        }
        let (pn, an) = match pos {
            Pos::Root => (get(1), get(2)),
            Pos::Elem(p) => (get(p), IExpr::I32(0)),
        };
        g.mb.push(IExpr::StructSet(ty, field, b(get(0)), b(get(c))));
        g.mb.push(IExpr::Call(
            FuncRef::Local(cfns.mount),
            vec![get(c), pn, an],
        ));
        let mut ub = vec![
            set(
                cl,
                un(UnOp::RefAsNonNull, IExpr::StructGet(ty, field, b(get(0)))),
            ),
            set(cm, bin(Op::I64And, get(1), IExpr::I64(BIT_ROUTE))),
        ];
        ub.extend(prop_updates);
        ub.push(IExpr::If(
            None,
            b(bin(Op::I64Ne, get(cm), IExpr::I64(0))),
            vec![IExpr::Call(
                FuncRef::Local(cfns.update),
                vec![get(cl), get(cm)],
            )],
            vec![],
        ));
        g.ub.extend(ub);
        g.db.push(IExpr::Call(
            FuncRef::Local(cfns.destroy),
            vec![
                un(UnOp::RefAsNonNull, IExpr::StructGet(ty, field, b(get(0)))),
                get(1),
            ],
        ));
        if matches!(pos, Pos::Root) {
            g.roots.push(Root::Child { field, comp: child });
        }
    }

    /// A function computing the default value of property `i` of component `comp`.
    fn default_fn(&mut self, comp: usize, i: usize) -> u32 {
        if let Some(&f) = self.u().defaults.get(&(comp, i)) {
            return f;
        }
        let idx = self.placeholder_fn();
        self.u().defaults.insert((comp, i), idx);
        let c = self.u().comps[comp].clone();
        let TVarKind::Prop(Some(def)) = &c.vars[i].kind else {
            unreachable!()
        };
        let mut cx = FnCx::new(vec![], c.locals.clone(), vec![]);
        let vt = self.vt(&c.vars[i].ty);
        let body = self.expr_as(&mut cx, def, vt);
        self.m.funcs[idx as usize] = Func {
            name: format!("{}_default{i}", c.name),
            params: vec![],
            result: vt,
            locals: cx.body_locals(),
            body: vec![body],
            export: None,
        };
        idx
    }

    fn handler_kind(&mut self, comp: usize, h: usize, si: usize) -> usize {
        let f = self.handler_fn(comp, h, si);
        let payload = self.u().comps[comp].handlers[h].payload;
        let u = self.u();
        if let Some(k) = u
            .handler_kinds
            .iter()
            .position(|&(s, fi, _)| s == si && fi == f)
        {
            return k;
        }
        u.handler_kinds.push((si, f, payload));
        u.handler_kinds.len() - 1
    }

    /// The function running handler `h` of `comp` with scope `si`; it ends by updating
    /// the component if any state changed.
    fn handler_fn(&mut self, comp: usize, h: usize, si: usize) -> u32 {
        if let Some(&f) = self.u().handler_fns.get(&(comp, h)) {
            return f;
        }
        let idx = self.placeholder_fn();
        self.u().handler_fns.insert((comp, h), idx);
        let c = self.u().comps[comp].clone();
        let hd = &c.handlers[h];
        let ty = self.scope_ty(si);
        let mut params = vec![VT::Ref(ty)];
        match hd.payload {
            Payload::Str => params.push(VT::Ref(self.str_ty)),
            Payload::Bool => params.push(VT::I32),
            Payload::Event => {
                if let Some(vt) = hd.event_ty.as_ref().and_then(|t| self.vt(t)) {
                    params.push(vt);
                }
            }
            Payload::None => {}
        }
        let mut cx = FnCx::new(params.clone(), c.locals.clone(), vec![]);
        cx.scope = Some(ScopeRef {
            scope: si,
            local: 0,
        });
        if let Some(p) = hd.param {
            cx.map.insert(p, 1);
        }
        let mut body = vec![self.stmt_expr(&mut cx, &hd.body)];
        body.extend(self.handler_epilogue(&mut cx, si));
        // `return` inside a handler must still run the update: wrap the body in a block.
        let body = wrap_returns(body);
        self.m.funcs[idx as usize] = Func {
            name: format!("{}_on{h}", c.name),
            params,
            result: None,
            locals: cx.body_locals(),
            body,
            export: None,
        };
        idx
    }

    /// Updates the component of scope `si` if a handler changed any of its state.
    fn handler_epilogue(&mut self, cx: &mut FnCx, si: usize) -> Vec<IExpr> {
        let root = self.comp_root_of(si);
        let rty = self.scope_ty(root);
        let rfns = self.u().scopes[root].fns;
        let cl = cx.temp(VT::Ref(rty));
        let m = cx.temp(VT::I64);
        let path = self.scope_path(cx, root);
        // A handler may have caused its own component to be destroyed (e.g. via an event).
        let dead = self.u().scopes[root].comp_fields.dead;
        vec![
            set(cl, un(UnOp::RefAsNonNull, path)),
            set(m, IExpr::StructGet(rty, 0, b(get(cl)))),
            IExpr::If(
                None,
                b(IExpr::StructGet(rty, dead, b(get(cl)))),
                vec![set(m, IExpr::I64(0))],
                vec![],
            ),
            IExpr::If(
                None,
                b(bin(Op::I64Ne, get(m), IExpr::I64(0))),
                vec![
                    IExpr::StructSet(rty, 0, b(get(cl)), b(IExpr::I64(0))),
                    IExpr::Call(FuncRef::Local(rfns.update), vec![get(cl), get(m)]),
                ],
                vec![],
            ),
        ]
    }

    fn gen_entry_points(&mut self, root_comp: usize) {
        let g = self.u().g;
        let rs = self.u().comp_scope[root_comp];
        let rty = self.scope_ty(rs);
        let rfns = self.u().scopes[rs].fns;
        let flush = self.helper_flush();
        let title = self.u().title.clone();
        // start()
        let mut cx = FnCx::new(vec![], vec![], vec![]);
        let slash = self.string_const("/");
        let t = self.op(&mut cx, opc::TITLE, vec![], vec![S::Static(title)]);
        let app = cx.temp(VT::Ref(rty));
        let start = Func {
            name: "start".into(),
            params: vec![],
            result: None,
            locals: vec![],
            body: vec![
                IExpr::If(
                    None,
                    b(un(UnOp::RefIsNull, IExpr::GlobalGet(g.route))),
                    vec![IExpr::GlobalSet(g.route, b(slash))],
                    vec![],
                ),
                t,
                set(app, IExpr::StructNewDefault(rty)),
                IExpr::GlobalSet(g.app, b(get(app))),
                IExpr::Call(
                    FuncRef::Local(rfns.mount),
                    vec![get(app), IExpr::I32(0), IExpr::I32(0)],
                ),
                IExpr::Call(flush, vec![]),
            ],
            export: Some("start".into()),
        };
        let start = Func {
            locals: cx.body_locals(),
            ..start
        };
        self.m.funcs.push(start);
        // set_route(len)
        let from = self.helper_str_from_mem();
        self.m.funcs.push(Func {
            name: "set_route".into(),
            params: vec![VT::I32],
            result: None,
            locals: vec![],
            body: vec![
                IExpr::GlobalSet(g.route, b(IExpr::Call(from, vec![get(0)]))),
                IExpr::If(
                    None,
                    b(un(
                        UnOp::I32Eqz,
                        un(UnOp::RefIsNull, IExpr::GlobalGet(g.app)),
                    )),
                    vec![
                        IExpr::Call(
                            FuncRef::Local(rfns.update),
                            vec![
                                un(UnOp::RefAsNonNull, IExpr::GlobalGet(g.app)),
                                IExpr::I64(BIT_ROUTE),
                            ],
                        ),
                        IExpr::Call(flush, vec![]),
                    ],
                    vec![],
                ),
            ],
            export: Some("set_route".into()),
        });
        // dispatch(slot, len)
        let (any_arr, i32_arr) = (self.u().any_arr, self.u().i32_arr);
        let kinds = self.u().handler_kinds.clone();
        let (slot, len, scope, kind) = (0, 1, 2, 3);
        let mut arms = Vec::new();
        for (k, (si, f, payload)) in kinds.iter().enumerate() {
            let sty = self.scope_ty(*si);
            let mut args = vec![IExpr::RefCast(sty, b(get(scope)))];
            match payload {
                Payload::Str => args.push(IExpr::Call(from, vec![get(len)])),
                Payload::Bool => args.push(bin(Op::I32Ne, get(len), IExpr::I32(0))),
                Payload::None => {}
                Payload::Event => unreachable!("component events are not dispatched by the host"),
            }
            arms.push((k, IExpr::Call(FuncRef::Local(*f), args)));
        }
        self.m.funcs.push(Func {
            name: "dispatch".into(),
            params: vec![VT::I32, VT::I32],
            result: None,
            locals: vec![VT::Any, VT::I32],
            body: vec![
                IExpr::If(
                    None,
                    b(un(UnOp::RefIsNull, IExpr::GlobalGet(g.slot_scope))),
                    vec![IExpr::Return(None)],
                    vec![],
                ),
                IExpr::If(
                    None,
                    b(bin(Op::I32GeU, get(slot), IExpr::GlobalGet(g.slot_n))),
                    vec![IExpr::Return(None)],
                    vec![],
                ),
                set(
                    scope,
                    IExpr::ArrayGet(any_arr, b(IExpr::GlobalGet(g.slot_scope)), b(get(slot))),
                ),
                // A released slot (removed element) is ignored.
                IExpr::If(
                    None,
                    b(un(UnOp::RefIsNull, get(scope))),
                    vec![IExpr::Return(None)],
                    vec![],
                ),
                set(
                    kind,
                    IExpr::ArrayGet(i32_arr, b(IExpr::GlobalGet(g.slot_kind)), b(get(slot))),
                ),
                switch_on(get(kind), arms),
                IExpr::Call(flush, vec![]),
            ],
            export: Some("dispatch".into()),
        });
        // fetch_done(slot, kind, status, len): the host's answer to a FETCH command.
        let fetches = self.u().fetch_kinds.clone();
        if !fetches.is_empty() {
            let release = self.helper_release_slot();
            let (slot, kind, status, len, scope, k) = (0, 1, 2, 3, 4, 5);
            let ext = |i| un(UnOp::I64ExtendI32S, get(i));
            let arms = fetches
                .iter()
                .enumerate()
                .map(|(i, &(si, f))| {
                    let sty = self.scope_ty(si);
                    (
                        i,
                        IExpr::Call(
                            FuncRef::Local(f),
                            vec![
                                IExpr::RefCast(sty, b(get(scope))),
                                ext(kind),
                                ext(status),
                                ext(len),
                            ],
                        ),
                    )
                })
                .collect();
            self.m.funcs.push(Func {
                name: "fetch_done".into(),
                params: vec![VT::I32; 4],
                result: None,
                locals: vec![VT::Any, VT::I32],
                body: vec![
                    IExpr::If(
                        None,
                        b(bin(Op::I32GeU, get(slot), IExpr::GlobalGet(g.slot_n))),
                        vec![IExpr::Return(None)],
                        vec![],
                    ),
                    set(
                        scope,
                        IExpr::ArrayGet(any_arr, b(IExpr::GlobalGet(g.slot_scope)), b(get(slot))),
                    ),
                    set(
                        k,
                        IExpr::ArrayGet(i32_arr, b(IExpr::GlobalGet(g.slot_kind)), b(get(slot))),
                    ),
                    IExpr::If(
                        None,
                        b(bin(
                            Op::I32Or,
                            un(UnOp::RefIsNull, get(scope)),
                            bin(Op::I32LtS, get(k), IExpr::I32(FETCH_KIND_BASE)),
                        )),
                        vec![IExpr::Return(None)],
                        vec![],
                    ),
                    IExpr::Call(release, vec![get(slot)]),
                    switch_on(bin(Op::I32Sub, get(k), IExpr::I32(FETCH_KIND_BASE)), arms),
                    IExpr::Call(flush, vec![]),
                ],
                export: Some("fetch_done".into()),
            });
        }
        // live_slots(): handler slots currently registered (for leak tests).
        self.m.funcs.push(Func {
            name: "live_slots".into(),
            params: vec![],
            result: Some(VT::I32),
            locals: vec![],
            body: vec![bin(
                Op::I32Sub,
                IExpr::GlobalGet(g.slot_n),
                IExpr::GlobalGet(g.free_n),
            )],
            export: Some("live_slots".into()),
        });
        let _ = IO_BASE;
    }
}

/// Locals bound by a pattern.
fn pat_binds(p: &TPat) -> Vec<LocalId> {
    match p {
        TPat::Bind(id) => vec![*id],
        TPat::Variant(_, _, _, subs) => subs.iter().flat_map(pat_binds).collect(),
        _ => vec![],
    }
}

/// `if idx == 0 {..} else if idx == 1 {..}` over the given arms.
fn switch_on(idx: IExpr, arms: Vec<(usize, IExpr)>) -> IExpr {
    let mut e = IExpr::Seq(vec![]);
    for (k, body) in arms.into_iter().rev() {
        e = IExpr::If(
            None,
            b(bin(Op::I32Eq, idx.clone(), IExpr::I32(k as i32))),
            vec![body],
            vec![e],
        );
    }
    e
}

/// `for local in 0..n { body }` with an i32 counter.
fn for_range(local: u32, n: IExpr, body: Vec<IExpr>, label: Label) -> IExpr {
    let (brk, lp) = (label * 2 + 100000, label * 2 + 100001);
    let mut inner = vec![IExpr::BrIf(brk, b(bin(Op::I32GeS, get(local), n)))];
    inner.extend(body);
    inner.push(set(local, bin(Op::I32Add, get(local), IExpr::I32(1))));
    inner.push(IExpr::Br(lp));
    IExpr::Seq(vec![
        set(local, IExpr::I32(0)),
        IExpr::Block(brk, None, vec![IExpr::Loop(lp, inner)]),
    ])
}

/// Turns `return` in a handler body into a branch to the epilogue.
fn wrap_returns(body: Vec<IExpr>) -> Vec<IExpr> {
    const L: Label = 999_999;
    fn rewrite(e: &mut IExpr) {
        if let IExpr::Return(None) = e {
            *e = IExpr::Br(L);
            return;
        }
        crate::opt::visit_children(e, rewrite);
    }
    let mut it = body.into_iter();
    let mut first = it.next().unwrap();
    rewrite(&mut first);
    let mut out = vec![IExpr::Block(L, None, vec![first])];
    out.extend(it);
    out
}
