//! Typed IR -> monomorphic IR. Instantiates generic functions and types on demand,
//! compiles pattern matching to tag tests, and maps VCWL types to Wasm GC types.

use std::collections::HashMap;

use crate::ast::BinOp;
use crate::ir::*;
use crate::tir::{self, AdtKind, LocalId, TExpr, TK, TPat, TStmt, Type};

/// Start of the host I/O region in linear memory (page 0 holds the DOM command buffer).
pub const IO_BASE: i32 = 65536;

#[derive(Clone, Debug)]
pub enum Layout {
    Struct(u32),
    /// Base type (holds the tag) and one subtype per variant.
    Enum(u32, Vec<u32>),
}

pub struct Lower<'a> {
    pub adts: &'a [tir::Adt],
    pub fns: &'a [tir::TFn],
    pub m: Module,
    pub str_ty: u32,
    layouts: HashMap<(usize, Vec<Type>), Layout>,
    insts: HashMap<(usize, Vec<Type>), u32>,
    queue: Vec<(usize, Vec<Type>, u32)>,
    helpers: HashMap<&'static str, u32>,
    pub strings: HashMap<String, (u32, u32)>,
    pub ui: Option<Box<crate::lower_ui::UiState>>,
    /// Function value types: (closure struct `{code, env}`, code signature `(env, params) -> ret`).
    closures: HashMap<Type, (u32, u32)>,
    /// Adapters that let a named function instance be called as a closure.
    fn_values: HashMap<(usize, Vec<Type>), u32>,
    /// `[T]` struct type by element array type, and back.
    vecs: HashMap<u32, u32>,
    vec_data: HashMap<u32, u32>,
    tip_ty: Option<u32>,
    /// Helpers generated per element array type.
    typed_helpers: HashMap<(&'static str, u32), u32>,
}

/// Per-function lowering state.
pub struct FnCx {
    pub locals: Vec<VT>,
    pub n_params: u32,
    pub map: HashMap<LocalId, u32>,
    pub tlocals: Vec<tir::Local>,
    pub subst: Vec<(String, Type)>,
    pub next_label: Label,
    pub loops: Vec<(Label, Label)>,
    /// UI scope of the function, when it is generated for a component.
    pub scope: Option<crate::lower_ui::ScopeRef>,
    /// View-bound locals temporarily held in plain wasm locals (e.g. while computing list keys).
    pub unbound: Vec<LocalId>,
}

impl FnCx {
    pub fn new(params: Vec<VT>, tlocals: Vec<tir::Local>, subst: Vec<(String, Type)>) -> FnCx {
        FnCx {
            n_params: params.len() as u32,
            locals: params,
            map: HashMap::new(),
            tlocals,
            subst,
            next_label: 0,
            loops: Vec::new(),
            scope: None,
            unbound: Vec::new(),
        }
    }
    pub fn temp(&mut self, t: VT) -> u32 {
        self.locals.push(t);
        self.locals.len() as u32 - 1
    }
    pub fn label(&mut self) -> Label {
        self.next_label += 1;
        self.next_label
    }
    /// Locals beyond the parameters, as declared in the function body.
    pub fn body_locals(&self) -> Vec<VT> {
        self.locals[self.n_params as usize..].to_vec()
    }
}

impl<'a> Lower<'a> {
    pub fn new(adts: &'a [tir::Adt], fns: &'a [tir::TFn]) -> Lower<'a> {
        let mut m = Module {
            memory_pages: 2,
            ..Default::default()
        };
        let str_ty = m.array_type(Storage::I8);
        Lower {
            adts,
            fns,
            m,
            str_ty,
            layouts: HashMap::new(),
            insts: HashMap::new(),
            queue: Vec::new(),
            helpers: HashMap::new(),
            strings: HashMap::new(),
            ui: None,
            closures: HashMap::new(),
            fn_values: HashMap::new(),
            vecs: HashMap::new(),
            vec_data: HashMap::new(),
            tip_ty: None,
            typed_helpers: HashMap::new(),
        }
    }

    /// Lowers all queued function instances until the queue is empty.
    pub fn drain(&mut self) {
        while let Some((id, targs, idx)) = self.queue.pop() {
            let f = self.lower_fn(id, &targs);
            let export = std::mem::take(&mut self.m.funcs[idx as usize].export);
            self.m.funcs[idx as usize] = Func { export, ..f };
        }
    }

    pub fn export_fn(&mut self, id: usize, name: &str) {
        let idx = self.fn_inst(id, &[]);
        self.m.funcs[idx as usize].export = Some(name.to_string());
    }

    // ----- types -----

    pub fn vt(&mut self, t: &Type) -> Option<VT> {
        Some(match t {
            Type::Int => VT::I64,
            Type::Float => VT::F64,
            Type::Bool => VT::I32,
            Type::Str => VT::Ref(self.str_ty),
            Type::Unit | Type::Never => return None,
            Type::Array(e) => VT::Ref(self.vec_type(e)),
            Type::Adt(..) => VT::Ref(self.base_type(t)),
            Type::Fn(..) => VT::Ref(self.closure_type(t).0),
            Type::Param(p) => panic!("unsubstituted type parameter {p}"),
            Type::Var(_) => panic!("inference variable survived checking"),
        })
    }

    /// Storage for a value in a struct field or array; `()` values are stored as a dummy i32.
    pub fn storage(&mut self, t: &Type) -> Storage {
        Storage::Val(self.vt(t).unwrap_or(VT::I32))
    }

    pub fn base_type(&mut self, t: &Type) -> u32 {
        match self.layout(t) {
            Layout::Struct(i) | Layout::Enum(i, _) => i,
        }
    }

    pub fn layout(&mut self, t: &Type) -> Layout {
        let Type::Adt(id, args) = t else {
            panic!("layout of non-ADT")
        };
        let key = (*id, args.clone());
        if let Some(l) = self.layouts.get(&key) {
            return l.clone();
        }
        let adt = &self.adts[*id];
        let map: Vec<(String, Type)> = adt.generics.iter().cloned().zip(args.clone()).collect();
        let placeholder = TypeDef::Struct {
            fields: vec![],
            sup: None,
            is_final: true,
        };
        match &adt.kind {
            AdtKind::Struct(fields) => {
                let idx = self.m.add_type(placeholder);
                self.layouts.insert(key, Layout::Struct(idx));
                let fs = fields
                    .iter()
                    .map(|(_, t)| (self.storage(&t.subst(&map)), false))
                    .collect();
                self.m.types[idx as usize] = TypeDef::Struct {
                    fields: fs,
                    sup: None,
                    is_final: true,
                };
                Layout::Struct(idx)
            }
            AdtKind::Enum(variants) => {
                let base = self.m.add_type(TypeDef::Struct {
                    fields: vec![(Storage::Val(VT::I32), false)],
                    sup: None,
                    is_final: false,
                });
                let vs: Vec<u32> = variants
                    .iter()
                    .map(|_| self.m.add_type(placeholder.clone()))
                    .collect();
                let l = Layout::Enum(base, vs.clone());
                self.layouts.insert(key, l.clone());
                for ((_, ftys), vi) in variants.iter().zip(&vs) {
                    let mut fs = vec![(Storage::Val(VT::I32), false)];
                    for ft in ftys {
                        fs.push((self.storage(&ft.subst(&map)), false));
                    }
                    self.m.types[*vi as usize] = TypeDef::Struct {
                        fields: fs,
                        sup: Some(base),
                        is_final: true,
                    };
                }
                l
            }
        }
    }

    /// `[T]` is `{len, data, tip}`. `data` (a mutable Wasm array) may be longer than `len` and
    /// shared by the versions of an array built by `push`; `tip.used` is the length of the
    /// newest version, the only one that may write past its own end. Arrays not grown by `push`
    /// have no tip.
    pub fn vec_type(&mut self, elem: &Type) -> u32 {
        let st = self.storage(elem);
        let data = self.m.array_type(st);
        if let Some(&v) = self.vecs.get(&data) {
            return v;
        }
        let tip = self.tip_type();
        let v = self.m.add_type(TypeDef::Struct {
            fields: vec![
                (Storage::Val(VT::I32), false),
                (Storage::Val(VT::Ref(data)), false),
                (Storage::Val(VT::RefNull(tip)), false),
            ],
            sup: None,
            is_final: true,
        });
        self.vecs.insert(data, v);
        self.vec_data.insert(v, data);
        v
    }

    fn tip_type(&mut self) -> u32 {
        if let Some(t) = self.tip_ty {
            return t;
        }
        let t = self.m.add_type(TypeDef::Struct {
            fields: vec![(Storage::Val(VT::I32), true)],
            sup: None,
            is_final: true,
        });
        self.tip_ty = Some(t);
        t
    }

    /// The element array type of `[T]` struct type `vec`.
    pub fn vec_data_type(&self, vec: u32) -> u32 {
        self.vec_data[&vec]
    }

    /// Length (`i32`) of array value `v` of struct type `vec`.
    pub fn vec_len(vec: u32, v: IExpr) -> IExpr {
        IExpr::StructGet(vec, 0, b(v))
    }

    /// Element storage of array value `v`; only the first `vec_len` elements belong to it.
    pub fn vec_items(vec: u32, v: IExpr) -> IExpr {
        IExpr::StructGet(vec, 1, b(v))
    }

    /// An array with exactly the elements of Wasm array `data`.
    pub fn vec_from(&mut self, vec: u32, data: IExpr, len: IExpr) -> IExpr {
        let tip = self.tip_type();
        IExpr::StructNew(vec, vec![len, data, IExpr::Null(tip)])
    }

    fn typed_helper(
        &mut self,
        name: &'static str,
        data: u32,
        build: impl FnOnce(&mut Self) -> Func,
    ) -> FuncRef {
        if let Some(&i) = self.typed_helpers.get(&(name, data)) {
            return FuncRef::Local(i);
        }
        let f = build(self);
        self.m.funcs.push(f);
        let i = self.m.funcs.len() as u32 - 1;
        self.typed_helpers.insert((name, data), i);
        FuncRef::Local(i)
    }

    /// `(n: i64, fill) -> [T]`.
    fn helper_arr_alloc(&mut self, vec: u32) -> FuncRef {
        let data = self.vec_data_type(vec);
        let TypeDef::Array {
            elem: Storage::Val(et),
        } = self.m.types[data as usize]
        else {
            unreachable!()
        };
        self.typed_helper("arr_alloc", data, |this| {
            let n = un(UnOp::I32WrapI64, get(0));
            let body = this.vec_from(vec, IExpr::ArrayNew(data, b(get(1)), b(n.clone())), n);
            Func {
                name: "arr_alloc".into(),
                params: vec![VT::I64, et],
                result: Some(VT::Ref(vec)),
                locals: vec![],
                body: vec![body],
                export: None,
            }
        })
    }

    /// `(a: [T], x) -> [T]`: writes `x` in place when `a` is the newest version of its storage
    /// and there is room; otherwise copies into storage twice as large.
    fn helper_arr_push(&mut self, vec: u32) -> FuncRef {
        let data = self.vec_data_type(vec);
        let tip = self.tip_type();
        let TypeDef::Array {
            elem: Storage::Val(et),
        } = self.m.types[data as usize]
        else {
            unreachable!()
        };
        self.typed_helper("arr_push", data, |_| {
            // params: a, x; locals: len, items, tip, grown
            let (a, x, len, items, t, grown) = (0, 1, 2, 3, 4, 5);
            let len1 = bin(Op::I32Add, get(len), IExpr::I32(1));
            let fits = IExpr::If(
                Some(VT::I32),
                b(un(UnOp::I32Eqz, un(UnOp::RefIsNull, get(t)))),
                vec![bin(
                    Op::I32And,
                    bin(Op::I32Eq, IExpr::StructGet(tip, 0, b(get(t))), get(len)),
                    bin(Op::I32LtU, get(len), IExpr::ArrayLen(b(get(items)))),
                )],
                vec![IExpr::I32(0)],
            );
            let cap = IExpr::If(
                Some(VT::I32),
                b(bin(Op::I32LtU, get(len), IExpr::I32(4))),
                vec![IExpr::I32(8)],
                vec![bin(Op::I32Add, get(len), get(len))],
            );
            let body = vec![
                set(len, IExpr::StructGet(vec, 0, b(get(a)))),
                set(items, IExpr::StructGet(vec, 1, b(get(a)))),
                set(t, IExpr::StructGet(vec, 2, b(get(a)))),
                IExpr::If(
                    None,
                    b(fits),
                    vec![
                        IExpr::ArraySet(data, b(get(items)), b(get(len)), b(get(x))),
                        IExpr::StructSet(
                            tip,
                            0,
                            b(un(UnOp::RefAsNonNull, get(t))),
                            b(len1.clone()),
                        ),
                        IExpr::Return(Some(b(IExpr::StructNew(
                            vec,
                            vec![len1.clone(), get(items), get(t)],
                        )))),
                    ],
                    vec![],
                ),
                set(grown, IExpr::ArrayNew(data, b(get(x)), b(cap))),
                IExpr::ArrayCopy(
                    data,
                    [
                        b(get(grown)),
                        b(IExpr::I32(0)),
                        b(get(items)),
                        b(IExpr::I32(0)),
                        b(get(len)),
                    ],
                ),
                IExpr::StructNew(
                    vec,
                    vec![len1.clone(), get(grown), IExpr::StructNew(tip, vec![len1])],
                ),
            ];
            Func {
                name: "arr_push".into(),
                params: vec![VT::Ref(vec), et],
                result: Some(VT::Ref(vec)),
                locals: vec![VT::I32, VT::Ref(data), VT::RefNull(tip), VT::Ref(data)],
                body,
                export: None,
            }
        })
    }

    /// The closure struct and code signature of function type `t`.
    pub fn closure_type(&mut self, t: &Type) -> (u32, u32) {
        if let Some(&c) = self.closures.get(t) {
            return c;
        }
        let Type::Fn(ps, r) = t else {
            panic!("closure type of non-function")
        };
        let mut params = vec![VT::Any];
        params.extend(ps.iter().filter_map(|p| self.vt(p)));
        let result = self.vt(r);
        let sig = self.m.add_type(TypeDef::Func { params, result });
        let cl = self.m.add_type(TypeDef::Struct {
            fields: vec![
                (Storage::Val(VT::Ref(sig)), false),
                (Storage::Val(VT::Any), false),
            ],
            sup: None,
            is_final: true,
        });
        self.closures.insert(t.clone(), (cl, sig));
        (cl, sig)
    }

    /// Adds `f` as a function of signature `sig` (a `ref.func` target) and returns its index.
    fn push_ref_target(&mut self, f: Func, sig: u32) -> u32 {
        self.m.funcs.push(f);
        let idx = self.m.funcs.len() as u32 - 1;
        self.m.sig_override.push((idx, sig));
        idx
    }

    /// The struct type holding captured values (shared by identical capture lists, since the
    /// function that creates an environment and the one that reads it compute it separately).
    pub fn env_type(&mut self, cx: &FnCx, captures: &[(TExpr, LocalId)]) -> u32 {
        let fields = captures
            .iter()
            .map(|(c, _)| {
                let t = self.ty(cx, &c.ty);
                (self.storage(&t), false)
            })
            .collect();
        let t = TypeDef::Struct {
            fields,
            sup: None,
            is_final: true,
        };
        match self.m.types.iter().position(|x| *x == t) {
            Some(i) => i as u32,
            None => self.m.add_type(t),
        }
    }

    /// Evaluates captured values (in the creating function) into a new environment struct.
    pub fn capture_env(&mut self, cx: &mut FnCx, captures: &[(TExpr, LocalId)]) -> (u32, IExpr) {
        let et = self.env_type(cx, captures);
        let vals = captures
            .iter()
            .map(|(c, _)| self.field_value(cx, c))
            .collect();
        (et, IExpr::StructNew(et, vals))
    }

    /// Copies an environment (`anyref` parameter `param`) into the locals holding the captures.
    pub fn load_env(
        &mut self,
        cx: &mut FnCx,
        et: u32,
        captures: &[(TExpr, LocalId)],
        param: u32,
    ) -> Vec<IExpr> {
        let e = cx.temp(VT::Ref(et));
        let mut out = vec![set(e, IExpr::RefCast(et, b(get(param))))];
        for (i, (_, loc)) in captures.iter().enumerate() {
            if let Some(x) = self.local(cx, *loc) {
                out.push(set(x, IExpr::StructGet(et, i as u32, b(get(e)))));
            }
        }
        out
    }

    /// A closure: its captured values go in an environment struct, its body in a new function.
    fn lower_lambda(&mut self, cx: &mut FnCx, l: &tir::TLambda, ty: &Type) -> IExpr {
        let (cl, sig) = self.closure_type(ty);
        let Type::Fn(_, r) = ty else { unreachable!() };
        let mut inner = FnCx::new(vec![VT::Any], cx.tlocals.clone(), cx.subst.clone());
        for &p in &l.params {
            let t = inner.tlocals[p].ty.subst(&inner.subst);
            if let Some(vt) = self.vt(&t) {
                let x = inner.temp(vt);
                inner.map.insert(p, x);
            }
        }
        inner.n_params = inner.locals.len() as u32;
        let params = inner.locals.clone();
        let mut body = Vec::new();
        let env = if l.captures.is_empty() {
            IExpr::NullAny
        } else {
            let (et, env) = self.capture_env(cx, &l.captures);
            body.extend(self.load_env(&mut inner, et, &l.captures, 0));
            env
        };
        let result = self.vt(r);
        body.push(self.expr_as(&mut inner, &l.body, result));
        let f = Func {
            name: "closure".into(),
            params,
            result,
            locals: inner.body_locals(),
            body,
            export: None,
        };
        let idx = self.push_ref_target(f, sig);
        IExpr::StructNew(cl, vec![IExpr::RefFunc(idx), env])
    }

    /// Named function `id` as a closure value.
    fn fn_value(&mut self, id: usize, targs: Vec<Type>, ty: &Type) -> IExpr {
        let (cl, sig) = self.closure_type(ty);
        let idx = match self.fn_values.get(&(id, targs.clone())) {
            Some(&i) => i,
            None => {
                let target = self.fn_inst(id, &targs);
                let TypeDef::Func { params, result } = self.m.types[sig as usize].clone() else {
                    unreachable!()
                };
                let args = (1..params.len() as u32).map(get).collect();
                let f = Func {
                    name: format!("{}_value", self.fns[id].name),
                    params,
                    result,
                    locals: vec![],
                    body: vec![IExpr::Call(FuncRef::Local(target), args)],
                    export: None,
                };
                let i = self.push_ref_target(f, sig);
                self.fn_values.insert((id, targs), i);
                i
            }
        };
        IExpr::StructNew(cl, vec![IExpr::RefFunc(idx), IExpr::NullAny])
    }

    pub fn fn_inst(&mut self, id: usize, targs: &[Type]) -> u32 {
        let key = (id, targs.to_vec());
        if let Some(&i) = self.insts.get(&key) {
            return i;
        }
        let idx = self.m.funcs.len() as u32;
        self.m.funcs.push(Func {
            name: String::new(),
            params: vec![],
            result: None,
            locals: vec![],
            body: vec![],
            export: None,
        });
        self.insts.insert(key, idx);
        self.queue.push((id, targs.to_vec(), idx));
        idx
    }

    fn lower_fn(&mut self, id: usize, targs: &[Type]) -> Func {
        let f = &self.fns[id];
        let subst: Vec<(String, Type)> = f
            .generics
            .iter()
            .cloned()
            .zip(targs.iter().cloned())
            .collect();
        let mut cx = FnCx::new(vec![], f.locals.clone(), subst.clone());
        for &p in &f.params {
            let t = f.locals[p].ty.subst(&subst);
            if let Some(vt) = self.vt(&t) {
                let l = cx.temp(vt);
                cx.map.insert(p, l);
            }
        }
        cx.n_params = cx.locals.len() as u32;
        let params = cx.locals.clone();
        let ret = f.ret.subst(&subst);
        let result = self.vt(&ret);
        let body = self.expr_as(&mut cx, &f.body, result);
        let name = if targs.is_empty() {
            f.name.clone()
        } else {
            format!("{}<{}>", f.name, targs.len())
        };
        Func {
            name,
            params,
            result,
            locals: cx.body_locals(),
            body: vec![body],
            export: None,
        }
    }

    pub fn ty(&self, cx: &FnCx, t: &Type) -> Type {
        t.subst(&cx.subst)
    }

    pub fn local(&mut self, cx: &mut FnCx, id: LocalId) -> Option<u32> {
        if let Some(&l) = cx.map.get(&id) {
            return Some(l);
        }
        let t = cx.tlocals[id].ty.subst(&cx.subst);
        let vt = self.vt(&t)?;
        let l = cx.temp(vt);
        cx.map.insert(id, l);
        Some(l)
    }

    pub fn string_const(&mut self, s: &str) -> IExpr {
        let (off, len) = *self.strings.entry(s.to_string()).or_insert_with(|| {
            let off = self.m.data.len() as u32;
            self.m.data.extend_from_slice(s.as_bytes());
            (off, s.len() as u32)
        });
        IExpr::ArrayNewData(self.str_ty, off, len)
    }

    // ----- expressions -----

    /// Lowers `e`, dropping its value when `want` is `None`.
    pub fn expr_as(&mut self, cx: &mut FnCx, e: &TExpr, want: Option<VT>) -> IExpr {
        let ty = self.ty(cx, &e.ty);
        let has = self.vt(&ty).is_some();
        let x = self.expr(cx, e);
        if has && want.is_none() {
            IExpr::Drop(b(x))
        } else {
            x
        }
    }

    pub fn stmt_expr(&mut self, cx: &mut FnCx, e: &TExpr) -> IExpr {
        self.expr_as(cx, e, None)
    }

    pub fn expr(&mut self, cx: &mut FnCx, e: &TExpr) -> IExpr {
        let ty = self.ty(cx, &e.ty);
        match &e.kind {
            TK::Int(n) => IExpr::I64(*n),
            TK::Float(n) => IExpr::F64(*n),
            TK::Bool(v) => IExpr::I32(*v as i32),
            TK::Str(s) => self.string_const(s),
            TK::Unit => IExpr::Seq(vec![]),
            TK::Local(id) if self.ui_bound(cx, *id) => self.ui_read(cx, e),
            TK::Local(id) => match self.local(cx, *id) {
                Some(l) => get(l),
                None => IExpr::Seq(vec![]),
            },
            TK::CompVar(_) | TK::Route => self.ui_read(cx, e),
            TK::SlotFilled(i) => self.ui_slot_filled(cx, *i),
            TK::Bin(op, a, bb) => self.binop(cx, *op, a, bb),
            TK::Not(a) => un(UnOp::I32Eqz, self.expr(cx, a)),
            TK::Neg(a) => {
                let x = self.expr(cx, a);
                if ty == Type::Float {
                    un(UnOp::F64Neg, x)
                } else {
                    bin(Op::I64Sub, IExpr::I64(0), x)
                }
            }
            TK::Call(id, targs, args) => {
                let targs: Vec<Type> = targs.iter().map(|t| self.ty(cx, t)).collect();
                let f = self.fn_inst(*id, &targs);
                let args = args.iter().map(|a| self.expr(cx, a)).collect();
                IExpr::Call(FuncRef::Local(f), args)
            }
            TK::Intrinsic(name, targs, args) => {
                let targs: Vec<Type> = targs.iter().map(|t| self.ty(cx, t)).collect();
                let args: Vec<IExpr> = args.iter().map(|a| self.expr(cx, a)).collect();
                self.intrinsic(name, &targs, args)
            }
            TK::Field(obj, i) => {
                let ot = self.ty(cx, &obj.ty);
                let s = self.base_type(&ot);
                let o = self.expr(cx, obj);
                let g = IExpr::StructGet(s, *i as u32, b(o));
                if self.vt(&ty).is_none() {
                    IExpr::Drop(b(g))
                } else {
                    g
                }
            }
            TK::StructNew(_, _, fields) => {
                let s = self.base_type(&ty);
                let fs = fields.iter().map(|f| self.field_value(cx, f)).collect();
                IExpr::StructNew(s, fs)
            }
            TK::Variant(_, _, v, fields) => {
                let Layout::Enum(base, vs) = self.layout(&ty) else {
                    unreachable!()
                };
                if fields.is_empty() {
                    return IExpr::StructNew(base, vec![IExpr::I32(*v as i32)]);
                }
                let mut fs = vec![IExpr::I32(*v as i32)];
                fs.extend(fields.iter().map(|f| self.field_value(cx, f)));
                IExpr::StructNew(vs[*v], fs)
            }
            TK::Array(items) => {
                let VT::Ref(at) = self.vt(&ty).unwrap() else {
                    unreachable!()
                };
                let xs: Vec<IExpr> = items.iter().map(|f| self.field_value(cx, f)).collect();
                let n = IExpr::I32(xs.len() as i32);
                let data = IExpr::ArrayNewFixed(self.vec_data_type(at), xs);
                self.vec_from(at, data, n)
            }
            TK::Index(arr, idx) => {
                let at_ty = self.ty(cx, &arr.ty);
                let VT::Ref(at) = self.vt(&at_ty).unwrap() else {
                    unreachable!()
                };
                let a = self.expr(cx, arr);
                let i = self.expr(cx, idx);
                let g = self.bounds_checked_get(cx, at, a, i);
                if self.vt(&ty).is_none() {
                    IExpr::Drop(b(g))
                } else {
                    g
                }
            }
            TK::If(c, t, f) => {
                let vt = self.vt(&ty);
                let c = self.expr(cx, c);
                let t = self.expr_as(cx, t, vt);
                let f = match f {
                    Some(f) => vec![self.expr_as(cx, f, vt)],
                    None => vec![],
                };
                IExpr::If(vt, b(c), vec![t], f)
            }
            TK::Block(stmts, tail) => {
                let vt = self.vt(&ty);
                let mut out: Vec<IExpr> = stmts.iter().map(|s| self.stmt(cx, s)).collect();
                if let Some(t) = tail {
                    out.push(self.expr_as(cx, t, vt));
                }
                IExpr::Seq(out)
            }
            TK::Match(scrut, arms) => self.lower_match(cx, scrut, arms, &ty),
            TK::Return(v) => {
                let v = v.as_ref().map(|v| {
                    let vt = self.vt(&self.ty(cx, &v.ty));
                    b(self.expr_as(cx, v, vt))
                });
                match v {
                    Some(v) if matches!(*v, IExpr::Seq(ref xs) if xs.is_empty()) => {
                        IExpr::Return(None)
                    }
                    v => IExpr::Return(v),
                }
            }
            TK::Lambda(l) => self.lower_lambda(cx, l, &ty),
            TK::FnRef(id, targs) => {
                let targs = targs.iter().map(|t| self.ty(cx, t)).collect();
                self.fn_value(*id, targs, &ty)
            }
            TK::CallValue(f, args) => {
                let fty = self.ty(cx, &f.ty);
                let (cl, sig) = self.closure_type(&fty);
                let t = cx.temp(VT::Ref(cl));
                let fv = self.expr(cx, f);
                let mut xs = vec![IExpr::StructGet(cl, 1, b(IExpr::LocalTee(t, b(fv))))];
                xs.extend(args.iter().map(|a| self.expr(cx, a)));
                IExpr::CallRef(sig, xs, b(IExpr::StructGet(cl, 0, b(get(t)))))
            }
        }
    }

    /// Value for a struct field / array slot: `()` values become a dummy `i32`.
    fn field_value(&mut self, cx: &mut FnCx, e: &TExpr) -> IExpr {
        let t = self.ty(cx, &e.ty);
        let x = self.expr(cx, e);
        if self.vt(&t).is_none() {
            IExpr::Seq(vec![x, IExpr::I32(0)])
        } else {
            x
        }
    }

    /// `a[i]` for array value `a` of struct type `at`.
    pub fn bounds_checked_get(&mut self, cx: &mut FnCx, at: u32, a: IExpr, i: IExpr) -> IExpr {
        let ta = cx.temp(VT::Ref(at));
        let data = self.vec_data_type(at);
        let ti = cx.temp(VT::I64);
        let panic = self.panic_call("array index out of bounds");
        IExpr::Seq(vec![
            set(ta, a),
            set(ti, i),
            IExpr::If(
                None,
                b(bin(
                    Op::I64GeU,
                    get(ti),
                    un(UnOp::I64ExtendI32U, Self::vec_len(at, get(ta))),
                )),
                vec![panic],
                vec![],
            ),
            IExpr::ArrayGet(
                data,
                b(Self::vec_items(at, get(ta))),
                b(un(UnOp::I32WrapI64, get(ti))),
            ),
        ])
    }

    pub fn panic_call(&mut self, msg: &str) -> IExpr {
        let s = self.string_const(msg);
        self.intrinsic("__panic", &[], vec![s])
    }

    fn binop(&mut self, cx: &mut FnCx, op: BinOp, a: &TExpr, bb: &TExpr) -> IExpr {
        let t = self.ty(cx, &a.ty);
        let t = if t == Type::Never {
            self.ty(cx, &bb.ty)
        } else {
            t
        };
        let x = self.expr(cx, a);
        let y = self.expr(cx, bb);
        use BinOp::*;
        match op {
            And => return IExpr::If(Some(VT::I32), b(x), vec![y], vec![IExpr::I32(0)]),
            Or => return IExpr::If(Some(VT::I32), b(x), vec![IExpr::I32(1)], vec![y]),
            _ => {}
        }
        let o = match (&t, op) {
            (Type::Int, Add) => Op::I64Add,
            (Type::Int, Sub) => Op::I64Sub,
            (Type::Int, Mul) => Op::I64Mul,
            (Type::Int, Div) => Op::I64DivS,
            (Type::Int, Rem) => Op::I64RemS,
            (Type::Int, Eq) => Op::I64Eq,
            (Type::Int, Ne) => Op::I64Ne,
            (Type::Int, Lt) => Op::I64LtS,
            (Type::Int, Le) => Op::I64LeS,
            (Type::Int, Gt) => Op::I64GtS,
            (Type::Int, Ge) => Op::I64GeS,
            (Type::Float, Add) => Op::F64Add,
            (Type::Float, Sub) => Op::F64Sub,
            (Type::Float, Mul) => Op::F64Mul,
            (Type::Float, Div) => Op::F64Div,
            (Type::Float, Rem) => {
                // x - trunc(x / y) * y, matching the sign of the dividend.
                let tx = cx.temp(VT::F64);
                let ty_ = cx.temp(VT::F64);
                return IExpr::Seq(vec![
                    set(tx, x),
                    set(ty_, y),
                    bin(
                        Op::F64Sub,
                        get(tx),
                        bin(
                            Op::F64Mul,
                            un(UnOp::F64Trunc, bin(Op::F64Div, get(tx), get(ty_))),
                            get(ty_),
                        ),
                    ),
                ]);
            }
            (Type::Float, Eq) => Op::F64Eq,
            (Type::Float, Ne) => Op::F64Ne,
            (Type::Float, Lt) => Op::F64Lt,
            (Type::Float, Le) => Op::F64Le,
            (Type::Float, Gt) => Op::F64Gt,
            (Type::Float, Ge) => Op::F64Ge,
            (Type::Bool, Eq) => Op::I32Eq,
            (Type::Bool, Ne) => Op::I32Ne,
            (Type::Adt(..), Eq | Ne) => {
                let base = self.base_type(&t);
                let o = if op == Eq { Op::I32Eq } else { Op::I32Ne };
                return bin(
                    o,
                    IExpr::StructGet(base, 0, b(x)),
                    IExpr::StructGet(base, 0, b(y)),
                );
            }
            _ => unreachable!("checker admitted {op:?} on {t:?}"),
        };
        bin(o, x, y)
    }

    // ----- statements -----

    pub fn stmt(&mut self, cx: &mut FnCx, s: &TStmt) -> IExpr {
        match s {
            TStmt::Let(id, e) => {
                let v = self.expr(cx, e);
                match self.local(cx, *id) {
                    Some(l) => set(l, v),
                    None => v,
                }
            }
            TStmt::Assign(place, e) => match place {
                tir::Place::Local(id) => {
                    let v = self.expr(cx, e);
                    match self.local(cx, *id) {
                        Some(l) => set(l, v),
                        None => v,
                    }
                }
                tir::Place::CompVar(i) => self.ui_assign(cx, *i, e),
            },
            TStmt::Expr(e) => self.stmt_expr(cx, e),
            TStmt::While(c, body) => {
                let (brk, cont, lp) = (cx.label(), cx.label(), cx.label());
                let c = self.expr(cx, c);
                cx.loops.push((brk, cont));
                let body = self.stmt_expr(cx, body);
                cx.loops.pop();
                IExpr::Block(
                    brk,
                    None,
                    vec![IExpr::Loop(
                        lp,
                        vec![
                            IExpr::BrIf(brk, b(un(UnOp::I32Eqz, c))),
                            IExpr::Block(cont, None, vec![body]),
                            IExpr::Br(lp),
                        ],
                    )],
                )
            }
            TStmt::ForRange(id, a, bnd, body) => {
                let (brk, cont, lp) = (cx.label(), cx.label(), cx.label());
                let i = self.local(cx, *id).unwrap();
                let end = cx.temp(VT::I64);
                let a = self.expr(cx, a);
                let bnd = self.expr(cx, bnd);
                cx.loops.push((brk, cont));
                let body = self.stmt_expr(cx, body);
                cx.loops.pop();
                let counter = cx.temp(VT::I64);
                IExpr::Seq(vec![
                    set(counter, a),
                    set(end, bnd),
                    IExpr::Block(
                        brk,
                        None,
                        vec![IExpr::Loop(
                            lp,
                            vec![
                                IExpr::BrIf(brk, b(bin(Op::I64GeS, get(counter), get(end)))),
                                set(i, get(counter)),
                                IExpr::Block(cont, None, vec![body]),
                                set(counter, bin(Op::I64Add, get(counter), IExpr::I64(1))),
                                IExpr::Br(lp),
                            ],
                        )],
                    ),
                ])
            }
            TStmt::ForArray(id, arr, body) => {
                let (brk, cont, lp) = (cx.label(), cx.label(), cx.label());
                let at_ty = self.ty(cx, &arr.ty);
                let VT::Ref(at) = self.vt(&at_ty).unwrap() else {
                    unreachable!()
                };
                let data = self.vec_data_type(at);
                let ta = cx.temp(VT::Ref(data));
                let n = cx.temp(VT::I32);
                let idx = cx.temp(VT::I32);
                let a = self.expr(cx, arr);
                let elem = self.local(cx, *id);
                cx.loops.push((brk, cont));
                let body = self.stmt_expr(cx, body);
                cx.loops.pop();
                let load = IExpr::ArrayGet(data, b(get(ta)), b(get(idx)));
                let tv = cx.temp(VT::Ref(at));
                let bind = match elem {
                    Some(l) => set(l, load),
                    None => IExpr::Seq(vec![]),
                };
                IExpr::Seq(vec![
                    set(tv, a),
                    set(ta, Self::vec_items(at, get(tv))),
                    set(n, Self::vec_len(at, get(tv))),
                    set(idx, IExpr::I32(0)),
                    IExpr::Block(
                        brk,
                        None,
                        vec![IExpr::Loop(
                            lp,
                            vec![
                                IExpr::BrIf(brk, b(bin(Op::I32GeU, get(idx), get(n)))),
                                bind,
                                IExpr::Block(cont, None, vec![body]),
                                set(idx, bin(Op::I32Add, get(idx), IExpr::I32(1))),
                                IExpr::Br(lp),
                            ],
                        )],
                    ),
                ])
            }
            TStmt::Break => IExpr::Br(cx.loops.last().expect("break outside loop").0),
            TStmt::Continue => IExpr::Br(cx.loops.last().expect("continue outside loop").1),
            TStmt::Fetch(f) => self.lower_fetch(cx, f),
            TStmt::Emit(ev, v) => self.ui_emit(cx, *ev, v.as_ref()),
        }
    }

    // ----- pattern matching -----

    pub fn lower_match(
        &mut self,
        cx: &mut FnCx,
        scrut: &TExpr,
        arms: &[(TPat, TExpr)],
        ty: &Type,
    ) -> IExpr {
        let st = self.ty(cx, &scrut.ty);
        let svt = self.vt(&st);
        let s = self.expr(cx, scrut);
        let vt = self.vt(ty);
        let (tmp, init) = match svt {
            Some(v) => {
                let t = cx.temp(v);
                (Some(t), set(t, s))
            }
            None => (None, s),
        };
        let val = tmp.map(get).unwrap_or(IExpr::Seq(vec![]));
        let mut chain = vec![IExpr::Unreachable];
        for (pat, body) in arms.iter().rev() {
            let test = self.pat_test(cx, pat, &st, &val);
            let mut then = self.pat_bind(cx, pat, &st, &val);
            then.push(self.expr_as(cx, body, vt));
            chain = match test {
                None => then,
                Some(t) => vec![IExpr::If(vt, b(t), then, chain)],
            };
        }
        let mut out = vec![init];
        out.extend(chain);
        IExpr::Seq(out)
    }

    /// Condition under which `pat` matches `v`, or `None` when it always matches.
    pub fn pat_test(&mut self, cx: &mut FnCx, pat: &TPat, t: &Type, v: &IExpr) -> Option<IExpr> {
        match pat {
            TPat::Wild | TPat::Bind(_) => None,
            TPat::Int(n) => Some(bin(Op::I64Eq, v.clone(), IExpr::I64(*n))),
            TPat::Bool(true) => Some(v.clone()),
            TPat::Bool(false) => Some(un(UnOp::I32Eqz, v.clone())),
            TPat::Str(s) => {
                let lit = self.string_const(s);
                let f = self.str_eq_fn();
                Some(IExpr::Call(f, vec![v.clone(), lit]))
            }
            TPat::Variant(adt, _, vi, subs) => {
                let Layout::Enum(base, vs) = self.layout(t) else {
                    unreachable!()
                };
                let tag = bin(
                    Op::I32Eq,
                    IExpr::StructGet(base, 0, b(v.clone())),
                    IExpr::I32(*vi as i32),
                );
                let ftys = self.variant_fields(*adt, t, *vi);
                let mut test = tag;
                for (i, (sp, ft)) in subs.iter().zip(&ftys).enumerate() {
                    let fv = IExpr::StructGet(
                        vs[*vi],
                        i as u32 + 1,
                        b(IExpr::RefCast(vs[*vi], b(v.clone()))),
                    );
                    if let Some(st) = self.pat_test(cx, sp, ft, &fv) {
                        test = IExpr::If(Some(VT::I32), b(test), vec![st], vec![IExpr::I32(0)]);
                    }
                }
                Some(test)
            }
        }
    }

    pub fn pat_bind(&mut self, cx: &mut FnCx, pat: &TPat, t: &Type, v: &IExpr) -> Vec<IExpr> {
        match pat {
            TPat::Bind(id) => match self.local(cx, *id) {
                Some(l) => vec![set(l, v.clone())],
                None => vec![],
            },
            TPat::Variant(adt, _, vi, subs) => {
                let Layout::Enum(_, vs) = self.layout(t) else {
                    unreachable!()
                };
                let ftys = self.variant_fields(*adt, t, *vi);
                let mut out = Vec::new();
                for (i, (sp, ft)) in subs.iter().zip(&ftys).enumerate() {
                    let fv = IExpr::StructGet(
                        vs[*vi],
                        i as u32 + 1,
                        b(IExpr::RefCast(vs[*vi], b(v.clone()))),
                    );
                    out.extend(self.pat_bind(cx, sp, ft, &fv));
                }
                out
            }
            _ => vec![],
        }
    }

    fn variant_fields(&self, adt: usize, t: &Type, vi: usize) -> Vec<Type> {
        let Type::Adt(_, args) = t else {
            unreachable!()
        };
        let a = &self.adts[adt];
        let map: Vec<(String, Type)> = a.generics.iter().cloned().zip(args.clone()).collect();
        a.variants()[vi].1.iter().map(|f| f.subst(&map)).collect()
    }

    fn str_eq_fn(&mut self) -> FuncRef {
        let id = self
            .fns
            .iter()
            .position(|f| f.name == "str_eq")
            .expect("prelude str_eq");
        FuncRef::Local(self.fn_inst(id, &[]))
    }

    // ----- intrinsics and host helpers -----

    pub fn intrinsic(&mut self, name: &str, targs: &[Type], mut a: Vec<IExpr>) -> IExpr {
        let s = self.str_ty;
        let wrap = |e: IExpr| un(UnOp::I32WrapI64, e);
        let ext = |e: IExpr| un(UnOp::I64ExtendI32U, e);
        let vec = |this: &mut Self| this.vec_type(&targs[0]);
        let unit = targs.first().is_some_and(|t| self.vt(t).is_none());
        let stored = |v: IExpr| {
            if unit {
                IExpr::Seq(vec![v, IExpr::I32(0)])
            } else {
                v
            }
        };
        let mut take = || a.remove(0);
        match name {
            "__str_len" => ext(IExpr::ArrayLen(b(take()))),
            "__str_byte" => {
                let (x, i) = (take(), take());
                ext(IExpr::ArrayGetU(s, b(x), b(wrap(i))))
            }
            "__str_alloc" => IExpr::ArrayNewDefault(s, b(wrap(take()))),
            "__str_set_byte" => {
                let (x, i, v) = (take(), take(), take());
                IExpr::ArraySet(s, b(x), b(wrap(i)), b(wrap(v)))
            }
            "__str_copy" => {
                let (d, di, src, si, n) = (take(), take(), take(), take(), take());
                IExpr::ArrayCopy(s, [b(d), b(wrap(di)), b(src), b(wrap(si)), b(wrap(n))])
            }
            // The prelude only writes into arrays it has just allocated.
            "__arr_copy" => {
                let v = vec(self);
                let t = self.vec_data_type(v);
                let (d, di, src, si, n) = (take(), take(), take(), take(), take());
                let (d, src) = (Self::vec_items(v, d), Self::vec_items(v, src));
                IExpr::ArrayCopy(t, [b(d), b(wrap(di)), b(src), b(wrap(si)), b(wrap(n))])
            }
            "__arr_len" => {
                let v = vec(self);
                ext(Self::vec_len(v, take()))
            }
            "__arr_alloc" => {
                let v = vec(self);
                let f = self.helper_arr_alloc(v);
                let (n, fill) = (take(), take());
                IExpr::Call(f, vec![n, stored(fill)])
            }
            "__arr_push" => {
                let v = vec(self);
                let f = self.helper_arr_push(v);
                let (x, e) = (take(), take());
                IExpr::Call(f, vec![x, stored(e)])
            }
            "__arr_set" => {
                let v = vec(self);
                let t = self.vec_data_type(v);
                let (x, i, e) = (take(), take(), take());
                IExpr::ArraySet(t, b(Self::vec_items(v, x)), b(wrap(i)), b(stored(e)))
            }
            "__float_from_int" => un(UnOp::F64ConvertI64S, take()),
            "__int_from_float" => un(UnOp::I64TruncSatF64S, take()),
            "__float_floor" => un(UnOp::F64Floor, take()),
            "__float_ceil" => un(UnOp::F64Ceil, take()),
            "__float_sqrt" => un(UnOp::F64Sqrt, take()),
            "__float_round" => un(UnOp::F64Nearest, take()),
            "__float_to_string" => {
                let f = self.m.import("fmt_f64", vec![VT::F64], Some(VT::I32));
                let from = self.helper_str_from_mem();
                IExpr::Call(from, vec![IExpr::Call(f, vec![take()])])
            }
            "__now_ms" => {
                let f = self.m.import("now", vec![], Some(VT::F64));
                IExpr::Call(f, vec![])
            }
            "__print" | "__panic" | "__navigate" | "__copy" | "__respond" | "__reject"
            | "__store_delete" => {
                let host = match name {
                    "__print" => "print",
                    "__panic" => "panic",
                    "__copy" => "copy",
                    "__respond" => "respond",
                    "__reject" => "reject",
                    "__store_delete" => "store_delete",
                    _ => "navigate",
                };
                let f = self.m.import(host, vec![VT::I32], None);
                let to = self.helper_str_to_mem();
                let call = IExpr::Call(f, vec![IExpr::Call(to, vec![take()])]);
                if name == "__panic" {
                    IExpr::Seq(vec![call, IExpr::Unreachable])
                } else {
                    call
                }
            }
            // Server host: a string in, a handle or a text length (-1: none) out.
            "__json_parse" | "__env" | "__store_get" => {
                let host = match name {
                    "__json_parse" => "json_parse",
                    "__env" => "env",
                    _ => "store_get",
                };
                let f = self.m.import(host, vec![VT::I32], Some(VT::I32));
                let to = self.helper_str_to_mem();
                un(
                    UnOp::I64ExtendI32S,
                    IExpr::Call(f, vec![IExpr::Call(to, vec![take()])]),
                )
            }
            "__js_call" | "__host" => {
                let host = if name == "__host" { "host" } else { "js_call" };
                let f = self.m.import(host, vec![VT::I32, VT::I32], Some(VT::I32));
                let to = self.helper_str_to_mem();
                let (name_len, text) = (take(), take());
                un(
                    UnOp::I64ExtendI32S,
                    IExpr::Call(f, vec![wrap(name_len), IExpr::Call(to, vec![text])]),
                )
            }
            "__json_drop" => {
                let f = self.m.import("json_drop", vec![VT::I32], None);
                IExpr::Call(f, vec![wrap(take())])
            }
            "__store_set" => {
                let f = self.m.import("store_set", vec![VT::I32, VT::I32], None);
                let to = self.helper_str_to_mem();
                let (key_len, text) = (take(), take());
                IExpr::Call(f, vec![wrap(key_len), IExpr::Call(to, vec![text])])
            }
            "__str_from_io" => {
                let from = self.helper_str_from_mem();
                IExpr::Call(from, vec![wrap(take())])
            }
            "__json_kind" | "__json_len" => {
                let host = if name == "__json_kind" {
                    "json_kind"
                } else {
                    "json_len"
                };
                let f = self.m.import(host, vec![VT::I32], Some(VT::I32));
                un(UnOp::I64ExtendI32S, IExpr::Call(f, vec![wrap(take())]))
            }
            "__json_bool" => {
                let f = self.m.import("json_bool", vec![VT::I32], Some(VT::I32));
                IExpr::Call(f, vec![wrap(take())])
            }
            "__json_num" => {
                let f = self.m.import("json_num", vec![VT::I32], Some(VT::F64));
                IExpr::Call(f, vec![wrap(take())])
            }
            "__json_index" => {
                let f = self
                    .m
                    .import("json_index", vec![VT::I32, VT::I32], Some(VT::I32));
                let (h, i) = (take(), take());
                ext(IExpr::Call(f, vec![wrap(h), wrap(i)]))
            }
            "__json_field" => {
                let f = self
                    .m
                    .import("json_field", vec![VT::I32, VT::I32], Some(VT::I32));
                let to = self.helper_str_to_mem();
                let (h, key) = (take(), take());
                ext(IExpr::Call(f, vec![wrap(h), IExpr::Call(to, vec![key])]))
            }
            // Host functions returning text write it to the I/O region and return its length.
            "__json_str" | "__json_num_text" | "__json_quote" => {
                let from = self.helper_str_from_mem();
                let arg = take();
                let (host, param, arg) = match name {
                    "__json_str" => ("json_str", VT::I32, wrap(arg)),
                    "__json_num_text" => ("json_num_text", VT::F64, arg),
                    _ => (
                        "json_quote",
                        VT::I32,
                        IExpr::Call(self.helper_str_to_mem(), vec![arg]),
                    ),
                };
                let f = self.m.import(host, vec![param], Some(VT::I32));
                IExpr::Call(from, vec![IExpr::Call(f, vec![arg])])
            }
            _ => self.ui_intrinsic(name, targs, a),
        }
    }

    /// Adds (once) a generated helper function.
    pub fn helper(&mut self, name: &'static str, build: impl FnOnce(&mut Self) -> Func) -> FuncRef {
        if let Some(&i) = self.helpers.get(name) {
            return FuncRef::Local(i);
        }
        let idx = self.m.funcs.len() as u32;
        self.m.funcs.push(Func {
            name: name.into(),
            params: vec![],
            result: None,
            locals: vec![],
            body: vec![],
            export: None,
        });
        self.helpers.insert(name, idx);
        let f = build(self);
        self.m.funcs[idx as usize] = f;
        FuncRef::Local(idx)
    }

    /// Grows linear memory so that `[IO_BASE, IO_BASE + len)` is addressable.
    fn ensure_io(len: IExpr) -> IExpr {
        let need = bin(
            Op::I32ShrU,
            bin(
                Op::I32Add,
                bin(Op::I32Add, IExpr::I32(IO_BASE), len),
                IExpr::I32(65535),
            ),
            IExpr::I32(16),
        );
        let cur = IExpr::MemorySize;
        IExpr::If(
            None,
            b(bin(Op::I32GtS, need.clone(), cur.clone())),
            vec![IExpr::Drop(b(IExpr::MemoryGrow(b(bin(
                Op::I32Sub,
                need,
                cur,
            )))))],
            vec![],
        )
    }

    /// `(s: str) -> i32`: copies a string into the I/O region and returns its byte length.
    pub fn helper_str_to_mem(&mut self) -> FuncRef {
        let s = self.str_ty;
        self.helper("str_to_mem", |_| {
            let (str_, i, n) = (0, 1, 2);
            let (brk, lp) = (1, 2);
            Func {
                name: "str_to_mem".into(),
                params: vec![VT::Ref(s)],
                result: Some(VT::I32),
                locals: vec![VT::I32, VT::I32],
                body: vec![
                    set(n, IExpr::ArrayLen(b(get(str_)))),
                    Self::ensure_io(get(n)),
                    IExpr::Block(
                        brk,
                        None,
                        vec![IExpr::Loop(
                            lp,
                            vec![
                                IExpr::BrIf(brk, b(bin(Op::I32GeU, get(i), get(n)))),
                                IExpr::Store8(
                                    b(bin(Op::I32Add, IExpr::I32(IO_BASE), get(i))),
                                    b(IExpr::ArrayGetU(s, b(get(str_)), b(get(i)))),
                                ),
                                set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                                IExpr::Br(lp),
                            ],
                        )],
                    ),
                    get(n),
                ],
                export: None,
            }
        })
    }

    /// `(len: i32) -> str`: builds a string from bytes the host wrote to the I/O region.
    pub fn helper_str_from_mem(&mut self) -> FuncRef {
        let s = self.str_ty;
        self.helper("str_from_mem", |_| {
            let (n, out, i) = (0, 1, 2);
            let (brk, lp) = (1, 2);
            Func {
                name: "str_from_mem".into(),
                params: vec![VT::I32],
                result: Some(VT::Ref(s)),
                locals: vec![VT::Ref(s), VT::I32],
                body: vec![
                    set(out, IExpr::ArrayNewDefault(s, b(get(n)))),
                    IExpr::Block(
                        brk,
                        None,
                        vec![IExpr::Loop(
                            lp,
                            vec![
                                IExpr::BrIf(brk, b(bin(Op::I32GeU, get(i), get(n)))),
                                IExpr::ArraySet(
                                    s,
                                    b(get(out)),
                                    b(get(i)),
                                    b(IExpr::Load8U(b(bin(
                                        Op::I32Add,
                                        IExpr::I32(IO_BASE),
                                        get(i),
                                    )))),
                                ),
                                set(i, bin(Op::I32Add, get(i), IExpr::I32(1))),
                                IExpr::Br(lp),
                            ],
                        )],
                    ),
                    get(out),
                ],
                export: None,
            }
        })
    }
}
