//! Abstract syntax tree for lingo v0.1.
//!
//! Scope today:
//!   - top-level: fn / const / struct / enum / impl
//!   - typed parameters and return types (types stored as ref names; a
//!     proper type checker comes in v0.1.x)
//!   - `let` / `let mut` (shadowing forbidden)
//!   - `if` / `elif` / `else`
//!   - `for x in start..end`
//!   - `return`, `break`, `continue`
//!   - `match` with literal, wildcard, bind, and `Type.Variant(...)` patterns
//!   - call, field, struct-literal, method-call, range, unary, binary
//!   - literals: int, float, str, bool, none
//!
//! Generics, error types, defer, nursery and closures are still pending.

use crate::error::Span;

#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Fn(FnDecl),
    Const(ConstDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Impl(ImplBlock),
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeRef {
    pub name: String,
    pub type_args: Vec<TypeRef>, // e.g. `vec[int]` or `map[str, int]`
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Vec<TypeRef>, // empty = nullary variant
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub target: String,         // type name being impl'd
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        is_mut: bool,
        name: String,
        ty: Option<TypeRef>,
        value: Expr,
        span: Span,
    },
    Assign {
        target: AssignTarget,
        value: Expr,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    If {
        arms: Vec<(Expr, Block)>,
        else_block: Option<Block>,
        span: Span,
    },
    For {
        var: String,
        iter: Expr,
        body: Block,
        span: Span,
    },
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Name(String),
    Field(Box<Expr>, String), // x.field = value (only on `self` in v0.1.1)
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard(Span),
    Bind(String, Span),
    Literal(PatLit, Span),
    Variant {
        type_name: Option<String>, // None = bare variant like `none`, `some`
        variant: String,
        sub: Vec<Pattern>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum PatLit {
    Int(i64),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None_,
    Ident(String),
    Self_,
    PrintBuiltin,
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Arg>),
    Range(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    VecLit(Vec<Expr>),
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}
