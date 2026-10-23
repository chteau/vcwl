//! Typed intermediate representation: the checker's output. Names are resolved, every
//! expression carries its type, and generic code still mentions type parameters
//! (monomorphization happens during lowering).

use crate::ast::BinOp;
use crate::diag::Span;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    Never,
    Array(Box<Type>),
    /// A struct or enum with its type arguments.
    Adt(usize, Vec<Type>),
    Param(String),
    /// A function value (closure or named function): `fn(A, B) -> R`.
    Fn(Vec<Type>, Box<Type>),
    /// Inference variable; never survives checking.
    Var(u32),
}

impl Type {
    pub fn subst(&self, map: &[(String, Type)]) -> Type {
        match self {
            Type::Param(n) => map
                .iter()
                .find(|(m, _)| m == n)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| self.clone()),
            Type::Array(t) => Type::Array(Box::new(t.subst(map))),
            Type::Adt(id, args) => Type::Adt(*id, args.iter().map(|a| a.subst(map)).collect()),
            Type::Fn(ps, r) => Type::Fn(
                ps.iter().map(|p| p.subst(map)).collect(),
                Box::new(r.subst(map)),
            ),
            _ => self.clone(),
        }
    }
}

pub type LocalId = usize;

#[derive(Clone, Debug)]
pub struct TExpr {
    pub kind: TK,
    pub ty: Type,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TK {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    Local(LocalId),
    /// Component property, state or derived value (index into the component's vars).
    CompVar(usize),
    /// The current route path (a global reactive value).
    Route,
    /// Whether the user of this component filled its slot number `.0` (fallback content).
    SlotFilled(usize),
    Bin(BinOp, Box<TExpr>, Box<TExpr>),
    Not(Box<TExpr>),
    Neg(Box<TExpr>),
    Call(usize, Vec<Type>, Vec<TExpr>),
    Intrinsic(&'static str, Vec<Type>, Vec<TExpr>),
    Field(Box<TExpr>, usize),
    StructNew(usize, Vec<Type>, Vec<TExpr>),
    Variant(usize, Vec<Type>, usize, Vec<TExpr>),
    Array(Vec<TExpr>),
    Index(Box<TExpr>, Box<TExpr>),
    If(Box<TExpr>, Box<TExpr>, Option<Box<TExpr>>),
    Block(Vec<TStmt>, Option<Box<TExpr>>),
    Match(Box<TExpr>, Vec<(TPat, TExpr)>),
    Return(Option<Box<TExpr>>),
    Lambda(Box<TLambda>),
    /// A named function used as a value, with its type arguments.
    FnRef(usize, Vec<Type>),
    /// Calls a function value.
    CallValue(Box<TExpr>, Vec<TExpr>),
}

/// A closure. Its body refers to captured values only through `captures`' inner locals.
#[derive(Clone, Debug)]
pub struct TLambda {
    pub params: Vec<LocalId>,
    /// The value captured when the closure is created (evaluated outside it), and the local
    /// holding it inside the body.
    pub captures: Vec<(TExpr, LocalId)>,
    pub body: TExpr,
}

#[derive(Clone, Debug)]
pub enum TStmt {
    Let(LocalId, TExpr),
    Assign(Place, TExpr),
    Expr(TExpr),
    While(TExpr, TExpr),
    ForRange(LocalId, TExpr, TExpr, TExpr),
    ForArray(LocalId, TExpr, TExpr),
    Break,
    Continue,
    /// Raise component event `idx` with an optional payload.
    Emit(usize, Option<TExpr>),
    Fetch(Box<TFetch>),
}

#[derive(Clone, Debug)]
pub struct TFetch {
    /// HTTP method in upper case.
    pub method: &'static str,
    pub url: TExpr,
    /// The request body, already encoded as JSON text.
    pub body: Option<TExpr>,
    /// Locals receiving the host's result kind, HTTP status and text length.
    pub params: [LocalId; 3],
    /// Runs when the response arrives: decodes it and evaluates the arms.
    pub on_done: TExpr,
    /// For `await`: handler values the rest of the handler uses, as in `TLambda`.
    pub captures: Vec<(TExpr, LocalId)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Place {
    Local(LocalId),
    CompVar(usize),
}

#[derive(Clone, Debug)]
pub enum TPat {
    Wild,
    Bind(LocalId),
    Int(i64),
    Str(String),
    Bool(bool),
    Variant(usize, Vec<Type>, usize, Vec<TPat>),
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub struct TFn {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<LocalId>,
    pub locals: Vec<Local>,
    pub ret: Type,
    pub body: TExpr,
    pub exported: bool,
}

#[derive(Clone, Debug)]
pub enum AdtKind {
    Struct(Vec<(String, Type)>),
    Enum(Vec<(String, Vec<Type>)>),
}

#[derive(Clone, Debug)]
pub struct Adt {
    pub name: String,
    pub generics: Vec<String>,
    pub kind: AdtKind,
}

impl Adt {
    pub fn fields(&self) -> &[(String, Type)] {
        match &self.kind {
            AdtKind::Struct(f) => f,
            AdtKind::Enum(_) => &[],
        }
    }
    pub fn variants(&self) -> &[(String, Vec<Type>)] {
        match &self.kind {
            AdtKind::Enum(v) => v,
            AdtKind::Struct(_) => &[],
        }
    }
}
