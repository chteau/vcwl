//! IR -> WebAssembly binary (GC proposal, standardized in Wasm 3.0).

use wasm_encoder::{
    AbstractHeapType, ArrayType, BlockType, CodeSection, ElementSection, Elements, CompositeInnerType, CompositeType, ConstExpr, DataCountSection,
    DataSection, EntityType, ExportKind, ExportSection, FieldType, FuncType, Function, FunctionSection, GlobalSection,
    GlobalType, HeapType, ImportSection, Instruction as I, MemArg, MemorySection, MemoryType, RefType, StorageType,
    StructType, SubType, TypeSection, ValType,
};

use crate::ir::*;

pub const HOST_MODULE: &str = "vcwl";

fn val(t: VT) -> ValType {
    match t {
        VT::I32 => ValType::I32,
        VT::I64 => ValType::I64,
        VT::F64 => ValType::F64,
        VT::Ref(i) => ValType::Ref(RefType { nullable: false, heap_type: HeapType::Concrete(i) }),
        VT::RefNull(i) => ValType::Ref(RefType { nullable: true, heap_type: HeapType::Concrete(i) }),
        VT::Any => ValType::Ref(RefType::ANYREF),
    }
}

fn storage(s: Storage) -> StorageType {
    match s {
        Storage::I8 => StorageType::I8,
        Storage::Val(v) => StorageType::Val(val(v)),
    }
}

fn composite(inner: CompositeInnerType) -> CompositeType {
    CompositeType { inner, shared: false, descriptor: None, describes: None }
}

pub fn emit(m: &Module) -> Vec<u8> {
    let mut module = wasm_encoder::Module::new();
    let n_gc = m.types.len() as u32;

    // Function signatures are deduplicated and placed after the GC rec group.
    let mut sigs: Vec<(Vec<VT>, Option<VT>)> = Vec::new();
    let mut sig = |p: &[VT], r: Option<VT>| -> u32 {
        let key = (p.to_vec(), r);
        let i = sigs.iter().position(|s| *s == key).unwrap_or_else(|| {
            sigs.push(key);
            sigs.len() - 1
        });
        n_gc + i as u32
    };
    let import_sigs: Vec<u32> = m.imports.iter().map(|im| sig(&im.params, im.result)).collect();
    let func_sigs: Vec<u32> = m.funcs.iter().enumerate().map(|(i, f)| {
        match m.sig_override.iter().find(|(fi, _)| *fi == i as u32) {
            Some((_, t)) => *t,
            None => sig(&f.params, f.result),
        }
    }).collect();

    let mut types = TypeSection::new();
    if n_gc > 0 {
        let subs: Vec<SubType> = m.types.iter().map(|t| match t {
            TypeDef::Struct { fields, sup, is_final } => SubType {
                is_final: *is_final,
                supertype_idxs: sup.iter().copied().collect(),
                composite_type: composite(CompositeInnerType::Struct(StructType {
                    fields: fields.iter().map(|(s, m)| FieldType { element_type: storage(*s), mutable: *m }).collect(),
                })),
            },
            TypeDef::Array { elem } => SubType {
                is_final: true,
                supertype_idxs: vec![],
                composite_type: composite(CompositeInnerType::Array(ArrayType(FieldType { element_type: storage(*elem), mutable: true }))),
            },
            TypeDef::Func { params, result } => SubType {
                is_final: true,
                supertype_idxs: vec![],
                composite_type: composite(CompositeInnerType::Func(FuncType::new(params.iter().map(|t| val(*t)), result.iter().map(|t| val(*t))))),
            },
        }).collect();
        types.ty().rec(subs);
    }
    for (p, r) in &sigs {
        types.ty().func_type(&FuncType::new(p.iter().map(|t| val(*t)), r.iter().map(|t| val(*t))));
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    for (im, s) in m.imports.iter().zip(&import_sigs) {
        imports.import(HOST_MODULE, im.name, EntityType::Function(*s));
    }
    if !m.imports.is_empty() {
        module.section(&imports);
    }

    let mut funcs = FunctionSection::new();
    for s in &func_sigs {
        funcs.function(*s);
    }
    module.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: m.memory_pages as u64, maximum: Some(16384), memory64: false, shared: false, page_size_log2: None });
    module.section(&mems);

    if !m.globals.is_empty() {
        let mut globals = GlobalSection::new();
        for g in &m.globals {
            let init = match &g.init {
                IExpr::I32(v) => ConstExpr::i32_const(*v),
                IExpr::I64(v) => ConstExpr::i64_const(*v),
                IExpr::F64(v) => ConstExpr::f64_const((*v).into()),
                IExpr::Null(t) => ConstExpr::ref_null(HeapType::Concrete(*t)),
                IExpr::NullAny => ConstExpr::ref_null(HeapType::Abstract { shared: false, ty: AbstractHeapType::Any }),
                e => panic!("unsupported global initializer {e:?}"),
            };
            globals.global(GlobalType { val_type: val(g.ty), mutable: true, shared: false }, &init);
        }
        module.section(&globals);
    }

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    let n_imports = m.imports.len() as u32;
    for (i, f) in m.funcs.iter().enumerate() {
        if let Some(name) = &f.export {
            exports.export(name, ExportKind::Func, n_imports + i as u32);
        }
    }
    for (i, g) in m.globals.iter().enumerate() {
        if let Some(name) = &g.export {
            exports.export(name, ExportKind::Global, i as u32);
        }
    }
    module.section(&exports);

    // Functions referenced with `ref.func` must be declared.
    let mut refs = Vec::new();
    for f in &m.funcs {
        for e in &f.body {
            collect_ref_funcs(e, &mut refs);
        }
    }
    if !refs.is_empty() {
        refs.sort();
        refs.dedup();
        let idx: Vec<u32> = refs.iter().map(|r| n_imports + r).collect();
        let mut elems = ElementSection::new();
        elems.declared(Elements::Functions(std::borrow::Cow::Borrowed(&idx)));
        module.section(&elems);
    }

    module.section(&DataCountSection { count: 1 });

    let mut code = CodeSection::new();
    for f in &m.funcs {
        // Non-null reference locals are declared nullable: Wasm only tracks initialization
        // within a block, so every read re-asserts non-nullness instead.
        let declared = f.locals.iter().map(|t| match t {
            VT::Ref(i) => VT::RefNull(*i),
            t => *t,
        });
        let mut func = Function::new_with_locals_types(declared.map(val));
        let non_null: Vec<bool> = f.params.iter().map(|_| false).chain(f.locals.iter().map(|t| matches!(t, VT::Ref(_)))).collect();
        let mut cx = Emit { f: &mut func, labels: Vec::new(), n_imports, non_null };
        for e in &f.body {
            cx.expr(e);
        }
        func.instruction(&I::End);
        code.function(&func);
    }
    module.section(&code);

    let mut data = DataSection::new();
    data.passive(m.data.iter().copied());
    module.section(&data);

    module.finish()
}

struct Emit<'f> {
    f: &'f mut Function,
    /// Enclosing structured-control labels, innermost last (`None` for unlabeled `if`).
    labels: Vec<Option<Label>>,
    n_imports: u32,
    /// Locals declared nullable that hold non-null references.
    non_null: Vec<bool>,
}

impl Emit<'_> {
    fn i(&mut self, ins: I) {
        self.f.instruction(&ins);
    }

    fn depth(&self, l: Label) -> u32 {
        let pos = self.labels.iter().rposition(|x| *x == Some(l)).expect("branch to unknown label");
        (self.labels.len() - 1 - pos) as u32
    }

    fn block_type(t: Option<VT>) -> BlockType {
        t.map(|t| BlockType::Result(val(t))).unwrap_or(BlockType::Empty)
    }

    fn mem(align: u32) -> MemArg {
        MemArg { offset: 0, align, memory_index: 0 }
    }

    fn expr(&mut self, e: &IExpr) {
        use IExpr::*;
        match e {
            I32(v) => self.i(I::I32Const(*v)),
            I64(v) => self.i(I::I64Const(*v)),
            F64(v) => self.i(I::F64Const((*v).into())),
            Null(t) => self.i(I::RefNull(HeapType::Concrete(*t))),
            NullAny => self.i(I::RefNull(HeapType::Abstract { shared: false, ty: AbstractHeapType::Any })),
            LocalGet(l) => {
                self.i(I::LocalGet(*l));
                if self.non_null[*l as usize] {
                    self.i(I::RefAsNonNull);
                }
            }
            LocalSet(l, v) => {
                self.expr(v);
                self.i(I::LocalSet(*l));
            }
            LocalTee(l, v) => {
                self.expr(v);
                self.i(I::LocalTee(*l));
                if self.non_null[*l as usize] {
                    self.i(I::RefAsNonNull);
                }
            }
            GlobalGet(g) => self.i(I::GlobalGet(*g)),
            GlobalSet(g, v) => {
                self.expr(v);
                self.i(I::GlobalSet(*g));
            }
            Bin(op, a, b) => {
                self.expr(a);
                self.expr(b);
                self.i(bin_ins(*op));
            }
            Un(op, a) => {
                self.expr(a);
                self.i(match op {
                    UnOp::I32Eqz => I::I32Eqz,
                    UnOp::I64Eqz => I::I64Eqz,
                    UnOp::I32WrapI64 => I::I32WrapI64,
                    UnOp::I64ExtendI32U => I::I64ExtendI32U,
                    UnOp::I64ExtendI32S => I::I64ExtendI32S,
                    UnOp::F64ConvertI64S => I::F64ConvertI64S,
                    UnOp::I64TruncSatF64S => I::I64TruncSatF64S,
                    UnOp::F64Neg => I::F64Neg,
                    UnOp::F64Floor => I::F64Floor,
                    UnOp::F64Ceil => I::F64Ceil,
                    UnOp::F64Sqrt => I::F64Sqrt,
                    UnOp::F64Nearest => I::F64Nearest,
                    UnOp::F64Trunc => I::F64Trunc,
                    UnOp::RefIsNull => I::RefIsNull,
                    UnOp::RefAsNonNull => I::RefAsNonNull,
                });
            }
            Call(f, args) => {
                args.iter().for_each(|a| self.expr(a));
                let idx = match f {
                    FuncRef::Import(i) => *i,
                    FuncRef::Local(i) => self.n_imports + i,
                };
                self.i(I::Call(idx));
            }
            If(t, c, th, el) => {
                self.expr(c);
                self.i(I::If(Self::block_type(*t)));
                self.labels.push(None);
                th.iter().for_each(|x| self.expr(x));
                if !el.is_empty() {
                    self.i(I::Else);
                    el.iter().for_each(|x| self.expr(x));
                }
                self.labels.pop();
                self.i(I::End);
            }
            Block(l, t, body) => {
                self.i(I::Block(Self::block_type(*t)));
                self.labels.push(Some(*l));
                body.iter().for_each(|x| self.expr(x));
                self.labels.pop();
                self.i(I::End);
            }
            Loop(l, body) => {
                self.i(I::Loop(BlockType::Empty));
                self.labels.push(Some(*l));
                body.iter().for_each(|x| self.expr(x));
                self.labels.pop();
                self.i(I::End);
            }
            Br(l) => {
                let d = self.depth(*l);
                self.i(I::Br(d));
            }
            BrIf(l, c) => {
                self.expr(c);
                let d = self.depth(*l);
                self.i(I::BrIf(d));
            }
            Return(v) => {
                if let Some(v) = v {
                    self.expr(v);
                }
                self.i(I::Return);
            }
            Drop(v) => {
                self.expr(v);
                self.i(I::Drop);
            }
            Seq(xs) => xs.iter().for_each(|x| self.expr(x)),
            Unreachable => self.i(I::Unreachable),
            StructNew(t, fs) => {
                fs.iter().for_each(|x| self.expr(x));
                self.i(I::StructNew(*t));
            }
            StructNewDefault(t) => self.i(I::StructNewDefault(*t)),
            RefFunc(f) => self.i(I::RefFunc(self.n_imports + f)),
            CallRef(t, args, f) => {
                args.iter().for_each(|a| self.expr(a));
                self.expr(f);
                self.i(I::CallRef(*t));
            }
            StructGet(t, i, o) => {
                self.expr(o);
                self.i(I::StructGet { struct_type_index: *t, field_index: *i });
            }
            StructSet(t, i, o, v) => {
                self.expr(o);
                self.expr(v);
                self.i(I::StructSet { struct_type_index: *t, field_index: *i });
            }
            ArrayNew(t, init, len) => {
                self.expr(init);
                self.expr(len);
                self.i(I::ArrayNew(*t));
            }
            ArrayNewDefault(t, len) => {
                self.expr(len);
                self.i(I::ArrayNewDefault(*t));
            }
            ArrayNewFixed(t, xs) => {
                xs.iter().for_each(|x| self.expr(x));
                self.i(I::ArrayNewFixed { array_type_index: *t, array_size: xs.len() as u32 });
            }
            ArrayNewData(t, off, len) => {
                self.i(I::I32Const(*off as i32));
                self.i(I::I32Const(*len as i32));
                self.i(I::ArrayNewData { array_type_index: *t, array_data_index: 0 });
            }
            ArrayGet(t, a, i) => {
                self.expr(a);
                self.expr(i);
                self.i(I::ArrayGet(*t));
            }
            ArrayGetU(t, a, i) => {
                self.expr(a);
                self.expr(i);
                self.i(I::ArrayGetU(*t));
            }
            ArraySet(t, a, i, v) => {
                self.expr(a);
                self.expr(i);
                self.expr(v);
                self.i(I::ArraySet(*t));
            }
            ArrayLen(a) => {
                self.expr(a);
                self.i(I::ArrayLen);
            }
            ArrayCopy(t, args) => {
                args.iter().for_each(|x| self.expr(x));
                self.i(I::ArrayCopy { array_type_index_dst: *t, array_type_index_src: *t });
            }
            RefCast(t, v) => {
                self.expr(v);
                self.i(I::RefCastNonNull(HeapType::Concrete(*t)));
            }
            RefTest(t, v) => {
                self.expr(v);
                self.i(I::RefTestNonNull(HeapType::Concrete(*t)));
            }
            RefEq(a, b) => {
                self.expr(a);
                self.expr(b);
                self.i(I::RefEq);
            }
            Load8U(a) => {
                self.expr(a);
                self.i(I::I32Load8U(Self::mem(0)));
            }
            Store8(a, v) => {
                self.expr(a);
                self.expr(v);
                self.i(I::I32Store8(Self::mem(0)));
            }
            Load32(a) => {
                self.expr(a);
                self.i(I::I32Load(Self::mem(0)));
            }
            Store32(a, v) => {
                self.expr(a);
                self.expr(v);
                self.i(I::I32Store(Self::mem(0)));
            }
            MemorySize => self.i(I::MemorySize(0)),
            MemoryInit(dst, off, len) => {
                self.expr(dst);
                self.i(I::I32Const(*off as i32));
                self.i(I::I32Const(*len as i32));
                self.i(I::MemoryInit { mem: 0, data_index: 0 });
            }
            MemoryGrow(n) => {
                self.expr(n);
                self.i(I::MemoryGrow(0));
            }
        }
    }
}

fn collect_ref_funcs(e: &IExpr, out: &mut Vec<u32>) {
    let mut e = e.clone();
    fn walk(e: &mut IExpr, out: &mut Vec<u32>) {
        if let IExpr::RefFunc(f) = e {
            out.push(*f);
        }
        crate::opt::visit_children(e, |c| walk(c, out));
    }
    walk(&mut e, out);
}

fn bin_ins(op: Op) -> I<'static> {
    use Op::*;
    match op {
        I32Add => I::I32Add,
        I32Sub => I::I32Sub,
        I32Mul => I::I32Mul,
        I32And => I::I32And,
        I32Or => I::I32Or,
        I32Xor => I::I32Xor,
        I32Shl => I::I32Shl,
        I32ShrU => I::I32ShrU,
        I32Eq => I::I32Eq,
        I32Ne => I::I32Ne,
        I32LtS => I::I32LtS,
        I32LtU => I::I32LtU,
        I32GtS => I::I32GtS,
        I32LeS => I::I32LeS,
        I32GeS => I::I32GeS,
        I32GeU => I::I32GeU,
        I64Add => I::I64Add,
        I64Sub => I::I64Sub,
        I64Mul => I::I64Mul,
        I64DivS => I::I64DivS,
        I64RemS => I::I64RemS,
        I64And => I::I64And,
        I64Or => I::I64Or,
        I64Shl => I::I64Shl,
        I64ShrU => I::I64ShrU,
        I64Eq => I::I64Eq,
        I64Ne => I::I64Ne,
        I64LtS => I::I64LtS,
        I64LeS => I::I64LeS,
        I64GtS => I::I64GtS,
        I64GeS => I::I64GeS,
        I64GeU => I::I64GeU,
        F64Add => I::F64Add,
        F64Sub => I::F64Sub,
        F64Mul => I::F64Mul,
        F64Div => I::F64Div,
        F64Eq => I::F64Eq,
        F64Ne => I::F64Ne,
        F64Lt => I::F64Lt,
        F64Le => I::F64Le,
        F64Gt => I::F64Gt,
        F64Ge => I::F64Ge,
    }
}

/// Validates a module with `wasmparser` (an implementation independent of this compiler).
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default());
    v.validate_all(bytes).map(|_| ()).map_err(|e| e.to_string())
}
