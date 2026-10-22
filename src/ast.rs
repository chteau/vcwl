//! Abstract syntax tree produced by the parser.

use crate::diag::Span;
use crate::lexer::Token;

pub type ExprId = u32;

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct File {
    pub path: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Import(Import),
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Component(Component),
    App(AppDecl),
    Style(NamedStyle),
}

#[derive(Clone, Debug)]
pub struct Import {
    pub path: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    pub exported: bool,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    /// Default value (component properties only).
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub exported: bool,
    pub generics: Vec<Ident>,
    pub fields: Vec<(Ident, TypeExpr)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: Ident,
    pub exported: bool,
    pub generics: Vec<Ident>,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
}

#[derive(Clone, Debug)]
pub enum TypeExprKind {
    Named(String, Vec<TypeExpr>),
    Array(Box<TypeExpr>),
    Optional(Box<TypeExpr>),
    Unit,
}

#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: Ident,
        mutable: bool,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        op: Option<BinOp>,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    For {
        var: Ident,
        iter: Expr,
        body: Block,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    /// `emit name(value)` raises a component event (event handlers only).
    Emit {
        name: Ident,
        value: Option<Expr>,
        span: Span,
    },
    /// `fetch get(url) as T { Ok(v) => ..., Err(e) => ... }` (event handlers only).
    Fetch(Box<Fetch>),
}

#[derive(Clone, Debug)]
pub struct Fetch {
    pub method: Ident,
    pub url: Expr,
    pub body: Option<Expr>,
    pub ty: TypeExpr,
    pub arms: Vec<Arm>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StrPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<StrPart>),
    Ident(String),
    Unit,
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    /// `callee<T>(args)`; the callee is a plain name (function, variant or builtin).
    Call {
        callee: Ident,
        type_args: Vec<TypeExpr>,
        args: Vec<Arg>,
    },
    /// `recv.name(args)` — a builtin method or a function called with `recv` as first argument.
    Method {
        recv: Box<Expr>,
        name: Ident,
        type_args: Vec<TypeExpr>,
        args: Vec<Arg>,
    },
    Field(Box<Expr>, Ident),
    Index(Box<Expr>, Box<Expr>),
    /// `expr?`: unwraps `Ok`/`Some`, or returns the `Err`/`None` from the enclosing function.
    Try(Box<Expr>),
    If {
        cond: Box<Expr>,
        then: Block,
        els: Option<Box<Expr>>,
    },
    Block(Block),
    Match {
        scrut: Box<Expr>,
        arms: Vec<Arm>,
    },
    StructLit {
        name: Ident,
        fields: Vec<(Ident, Expr)>,
        base: Option<Box<Expr>>,
    },
    Array(Vec<Expr>),
    Range(Box<Expr>, Box<Expr>),
    Return(Option<Box<Expr>>),
    /// Unit literals like `12px`; only valid as style values.
    Dim(f64, String),
    Color(String),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pattern,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub enum PatKind {
    Wild,
    Bind(String),
    Int(i64),
    Str(String),
    Bool(bool),
    /// `Name` or `Name(p, ...)` or `Enum.Name(p, ...)`.
    Variant {
        qual: Option<Ident>,
        name: Ident,
        args: Vec<Pattern>,
    },
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub kind: PatKind,
    pub span: Span,
}

// ----- web language -----

#[derive(Clone, Debug)]
pub struct Component {
    pub name: Ident,
    pub exported: bool,
    pub params: Vec<Param>,
    pub body: Vec<View>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum View {
    State {
        name: Ident,
        ty: Option<TypeExpr>,
        init: Expr,
        span: Span,
    },
    Let {
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Element(Element),
    If {
        cond: Expr,
        then: Vec<View>,
        els: Option<Vec<View>>,
        span: Span,
    },
    For {
        var: Ident,
        index: Option<Ident>,
        iter: Expr,
        key: Option<Expr>,
        body: Vec<View>,
        span: Span,
    },
    Match {
        scrut: Expr,
        arms: Vec<(Pattern, Vec<View>)>,
        span: Span,
    },
    On {
        event: Ident,
        param: Option<Ident>,
        body: Block,
        span: Span,
    },
    /// `event name(Type)` declares an event the component can emit to its parent.
    Event {
        name: Ident,
        ty: Option<TypeExpr>,
        span: Span,
    },
    Style(StyleBlock),
    StyleRef(Ident),
    StyleIf {
        cond: Expr,
        block: StyleBlock,
        span: Span,
    },
    Routes {
        arms: Vec<(Option<String>, Span, Vec<View>)>,
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub struct Element {
    pub name: Ident,
    pub args: Vec<ElemArg>,
    pub children: Vec<View>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ElemArg {
    Expr(Arg),
    /// Layout arguments such as `gap: 20px` keep raw style tokens.
    Style(Ident, Vec<Token>, Span),
}

#[derive(Clone, Debug)]
pub struct StyleBlock {
    pub decls: Vec<StyleDecl>,
    pub conds: Vec<(Vec<Token>, Span, StyleBlock)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct StyleDecl {
    pub prop: Ident,
    pub value: Vec<Token>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct NamedStyle {
    pub name: Ident,
    pub exported: bool,
    pub block: StyleBlock,
}

#[derive(Clone, Debug)]
pub struct AppDecl {
    pub title: String,
    pub capabilities: Vec<(Ident, Vec<Token>, Span)>,
    pub span: Span,
}
