//! Monomorphic, structured intermediate representation close to WebAssembly GC.
//! Produced by `lower`, optimized by `opt`, emitted by `codegen`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VT {
    I32,
    I64,
    F64,
    /// Non-null reference to a module type.
    Ref(u32),
    /// Nullable reference to a module type.
    RefNull(u32),
    /// Nullable `anyref`.
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Storage {
    I8,
    Val(VT),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeDef {
    Struct {
        fields: Vec<(Storage, bool)>,
        sup: Option<u32>,
        is_final: bool,
    },
    Array {
        elem: Storage,
    },
    /// A function type inside the GC rec group (for typed function references).
    Func {
        params: Vec<VT>,
        result: Option<VT>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FuncRef {
    Import(u32),
    Local(u32),
}

pub type Label = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    I32Add,
    I32Sub,
    I32Mul,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrU,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32LeS,
    I32GeS,
    I32GeU,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64RemS,
    I64And,
    I64Or,
    I64Shl,
    I64ShrU,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LeS,
    I64GtS,
    I64GeS,
    I64GeU,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Le,
    F64Gt,
    F64Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    I32Eqz,
    I64Eqz,
    I32WrapI64,
    I64ExtendI32U,
    I64ExtendI32S,
    F64ConvertI64S,
    I64TruncSatF64S,
    F64Neg,
    F64Floor,
    F64Ceil,
    F64Sqrt,
    F64Nearest,
    F64Trunc,
    RefIsNull,
    RefAsNonNull,
}

#[derive(Clone, Debug)]
pub enum IExpr {
    I32(i32),
    I64(i64),
    F64(f64),
    Null(u32),
    NullAny,
    LocalGet(u32),
    LocalSet(u32, Box<IExpr>),
    LocalTee(u32, Box<IExpr>),
    GlobalGet(u32),
    GlobalSet(u32, Box<IExpr>),
    Bin(Op, Box<IExpr>, Box<IExpr>),
    Un(UnOp, Box<IExpr>),
    Call(FuncRef, Vec<IExpr>),
    /// Typed `if` with optional result.
    If(Option<VT>, Box<IExpr>, Vec<IExpr>, Vec<IExpr>),
    Block(Label, Option<VT>, Vec<IExpr>),
    Loop(Label, Vec<IExpr>),
    Br(Label),
    BrIf(Label, Box<IExpr>),
    Return(Option<Box<IExpr>>),
    Drop(Box<IExpr>),
    Seq(Vec<IExpr>),
    Unreachable,
    StructNew(u32, Vec<IExpr>),
    StructNewDefault(u32),
    RefFunc(u32),
    /// `call_ref` through a function reference of rec-group type `u32`.
    CallRef(u32, Vec<IExpr>, Box<IExpr>),
    StructGet(u32, u32, Box<IExpr>),
    StructSet(u32, u32, Box<IExpr>, Box<IExpr>),
    ArrayNew(u32, Box<IExpr>, Box<IExpr>),
    ArrayNewDefault(u32, Box<IExpr>),
    ArrayNewFixed(u32, Vec<IExpr>),
    /// `array.new_data` from the module's single passive data segment: (offset, len).
    ArrayNewData(u32, u32, u32),
    ArrayGet(u32, Box<IExpr>, Box<IExpr>),
    ArrayGetU(u32, Box<IExpr>, Box<IExpr>),
    ArraySet(u32, Box<IExpr>, Box<IExpr>, Box<IExpr>),
    ArrayLen(Box<IExpr>),
    ArrayCopy(u32, [Box<IExpr>; 5]),
    RefCast(u32, Box<IExpr>),
    RefTest(u32, Box<IExpr>),
    RefEq(Box<IExpr>, Box<IExpr>),
    Load8U(Box<IExpr>),
    Store8(Box<IExpr>, Box<IExpr>),
    Load32(Box<IExpr>),
    Store32(Box<IExpr>, Box<IExpr>),
    MemorySize,
    MemoryGrow(Box<IExpr>),
    /// `memory.init` from the passive data segment: (dest address, segment offset, length).
    MemoryInit(Box<IExpr>, u32, u32),
}

#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub params: Vec<VT>,
    pub result: Option<VT>,
    pub locals: Vec<VT>,
    pub body: Vec<IExpr>,
    pub export: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub name: &'static str,
    pub params: Vec<VT>,
    pub result: Option<VT>,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub ty: VT,
    pub init: IExpr,
    pub export: Option<String>,
}

#[derive(Default, Debug)]
pub struct Module {
    pub types: Vec<TypeDef>,
    pub imports: Vec<Import>,
    pub funcs: Vec<Func>,
    pub globals: Vec<Global>,
    pub data: Vec<u8>,
    /// Exported linear memory used for host I/O (pages).
    pub memory_pages: u32,
    /// Functions whose type must be a specific rec-group function type (targets of `ref.func`).
    pub sig_override: Vec<(u32, u32)>,
}

impl Module {
    pub fn add_type(&mut self, t: TypeDef) -> u32 {
        self.types.push(t);
        self.types.len() as u32 - 1
    }

    /// Returns the index of a structurally identical array type, creating it if needed.
    pub fn array_type(&mut self, elem: Storage) -> u32 {
        let t = TypeDef::Array { elem };
        match self.types.iter().position(|x| *x == t) {
            Some(i) => i as u32,
            None => self.add_type(t),
        }
    }

    pub fn import(&mut self, name: &'static str, params: Vec<VT>, result: Option<VT>) -> FuncRef {
        if let Some(i) = self.imports.iter().position(|im| im.name == name) {
            return FuncRef::Import(i as u32);
        }
        self.imports.push(Import {
            name,
            params,
            result,
        });
        FuncRef::Import(self.imports.len() as u32 - 1)
    }
}

/// Shorthands for building IR trees.
pub fn b(e: IExpr) -> Box<IExpr> {
    Box::new(e)
}

pub fn bin(op: Op, x: IExpr, y: IExpr) -> IExpr {
    IExpr::Bin(op, b(x), b(y))
}

pub fn un(op: UnOp, x: IExpr) -> IExpr {
    IExpr::Un(op, b(x))
}

pub fn set(l: u32, e: IExpr) -> IExpr {
    IExpr::LocalSet(l, b(e))
}

pub fn get(l: u32) -> IExpr {
    IExpr::LocalGet(l)
}
