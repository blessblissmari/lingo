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
    Trait(TraitDecl),
    ImplTrait(ImplTraitBlock),
    /// v0.3.0 — `import foo.bar` / `import foo.bar as b`.
    /// Resolved by `src/modules.rs` before the program reaches the
    /// interpreter or the C backend; by then every import has been
    /// flattened away and every cross-module reference has been
    /// rewritten to a globally-unique mangled name.  An `Item::Import`
    /// only ever survives between *parser → resolver*; it should never
    /// reach `interp.rs` or `codegen_c.rs`.
    Import(ImportDecl),
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    /// Dotted path as written, e.g. `import foo.bar` → `["foo", "bar"]`.
    /// Resolved to `<entry_dir>/foo/bar.lingo`.
    pub path: Vec<String>,
    /// Optional `as <name>` alias.  If absent, the last segment of `path`
    /// is the alias (so `import foo.bar` is `bar.<name>`).
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitDecl {
    pub name: String,
    /// Optional generic type parameters: `trait Encoder[T]:` parses with
    /// `type_params = ["T"]`. v0.2.5 — type params substitute into method
    /// signatures (`T`, `Self`) at impl conformance checking time; the
    /// impl supplies one concrete type per param in its `[..]` brackets.
    /// The built-in `From` trait is registered synthetically with
    /// `type_params = ["E"]` if any `impl From[..] for ..:` is seen
    /// without a user-visible declaration.
    pub type_params: Vec<String>,
    /// Required method signatures (no body) + optional default-impl methods (with body).
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitMethod {
    /// `decl.body` is empty for required methods, populated for default impls.
    pub decl: FnDecl,
    pub has_default: bool,
}

#[derive(Debug, Clone)]
pub struct ImplTraitBlock {
    pub trait_name: String,
    /// Optional generic-args between brackets after the trait name —
    /// `impl From[str] for ParseErr:` parses with `trait_args = ["str"]`,
    /// `impl Encoder[int] for IntEnc:` parses with `trait_args = ["int"]`.
    /// v0.2.5: must match the trait's declared `type_params` arity. The
    /// resolver substitutes `type_params[i] -> trait_args[i]` (and `Self`
    /// -> `target`) when checking impl conformance. Each element is a
    /// type name (no nesting for now — `Encoder[map[str, int]]` and
    /// friends are deferred).
    pub trait_args: Vec<String>,
    pub target: String,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub raises: Option<TypeRef>, // `! E` after the return type — fallible fn
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
    Raise {
        value: Expr,
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
    ShowBuiltin,
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
    MapLit(Vec<(Expr, Expr)>),
    FString(Vec<FStringPart>),
    /// postfix `?` — propagate error from a fallible call.
    /// Optional `fallback`: `expr? else <fallback>` lifts the inner error
    /// into the caller's `raises.1` type by raising `<fallback>` instead.
    /// This is how v0.2.2 closes the error-type-coercion gap (e.g. wrapping
    /// `int(s) -> int!str` failures into a caller's `int!ParseErr`).
    Try {
        inner: Box<Expr>,
        fallback: Option<Box<Expr>>,
    },
    /// `forever` — only legal as the iterable of `for _ in forever:`.
    /// Lowered to an infinite loop. Not a value; cannot be assigned, returned,
    /// printed, etc.
    Forever,
}

#[derive(Debug, Clone)]
pub enum FStringPart {
    Lit(String),
    Expr(Expr),
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

// ---------------------------------------------------------------------
// v0.2.6: trait-method signature substitution helpers
// ---------------------------------------------------------------------
//
// When checking that an `impl Trait[A1, A2] for Target:` block conforms
// to its trait declaration, we substitute the trait's declared type
// parameters (and the special `Self` name) into each trait method's
// signature before comparing it to the impl method's signature.
//
// Substitution map:
//   trait_decl.type_params[i]  ->  impl_block.trait_args[i]
//   "Self"                     ->  impl_block.target
//
// Comparison is structural over type names: `vec[int]` matches
// `vec[int]`, `Opt[str]` matches `Opt[str]`, and so on.  Param *names*
// can differ between trait and impl — only types matter.  The receiver
// `self` parameter participates in this check too: a trait method
// declares `self: Self`, and after `Self -> Target` substitution it
// must match the impl method's `self: Self` (which the parser also
// gives the placeholder name "Self" — the substitution covers both).

/// Substitute every type name in `ty` that appears as a key in `subst`
/// with the mapped concrete name.  Walks `type_args` recursively so
/// `vec[T]` becomes `vec[int]` when `T -> int`.
pub fn subst_typeref(ty: &TypeRef, subst: &std::collections::HashMap<String, String>) -> TypeRef {
    let new_name = subst.get(&ty.name).cloned().unwrap_or_else(|| ty.name.clone());
    TypeRef {
        name: new_name,
        type_args: ty.type_args.iter().map(|t| subst_typeref(t, subst)).collect(),
        span: ty.span,
    }
}

/// Structural equality over TypeRef by name (ignoring spans).
pub fn typeref_eq(a: &TypeRef, b: &TypeRef) -> bool {
    a.name == b.name
        && a.type_args.len() == b.type_args.len()
        && a.type_args.iter().zip(b.type_args.iter()).all(|(x, y)| typeref_eq(x, y))
}

/// Pretty-print a TypeRef for diagnostics (`vec[map[str, int]]`).
pub fn typeref_display(ty: &TypeRef) -> String {
    if ty.type_args.is_empty() {
        ty.name.clone()
    } else {
        let inner: Vec<String> = ty.type_args.iter().map(typeref_display).collect();
        format!("{}[{}]", ty.name, inner.join(", "))
    }
}

/// Optional-TypeRef structural equality + display.
pub fn typeref_opt_eq(a: &Option<TypeRef>, b: &Option<TypeRef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => typeref_eq(x, y),
        _ => false,
    }
}

pub fn typeref_opt_display(a: &Option<TypeRef>) -> String {
    match a {
        None => "<none>".into(),
        Some(t) => typeref_display(t),
    }
}

/// Build the substitution map a `TraitDecl` / `ImplTraitBlock` pair
/// induces.  Caller has already verified arity (`trait_args.len() ==
/// type_params.len()`).  Includes the implicit `"Self" -> target`
/// mapping.  Returned map is keyed/valued by owned String so it can
/// outlive the borrow on `trait_decl`.
pub fn build_trait_subst(
    type_params: &[String],
    trait_args: &[String],
    target: &str,
) -> std::collections::HashMap<String, String> {
    debug_assert_eq!(type_params.len(), trait_args.len());
    let mut m = std::collections::HashMap::new();
    for (p, a) in type_params.iter().zip(trait_args.iter()) {
        m.insert(p.clone(), a.clone());
    }
    m.insert("Self".to_string(), target.to_string());
    m
}

/// Check that `impl_fn`'s signature matches `trait_fn`'s after
/// substitution.  Returns the offending diagnostic string on
/// mismatch — the caller wraps it in a `LingoError` with the proper
/// span (impl method's span is the most useful pointer).
pub fn check_trait_method_sig(
    trait_name: &str,
    target: &str,
    trait_fn: &FnDecl,
    impl_fn: &FnDecl,
    subst: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    // arity
    if trait_fn.params.len() != impl_fn.params.len() {
        return Err(format!(
            "method `{tn}.{m}` for `{tgt}`: trait declares {n} parameter(s), impl provides {k}",
            tn = trait_name, m = impl_fn.name, tgt = target,
            n = trait_fn.params.len(), k = impl_fn.params.len()
        ));
    }
    // v0.2.6: the parser also gives the impl method's `self` parameter
    // the placeholder type name "Self".  We resolve that to the
    // concrete target on the impl side before comparing, using a
    // single-entry map (so `T` etc. on the impl side stay as written
    // — they're just regular type names from the impl's POV).
    let mut impl_subst: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    impl_subst.insert("Self".into(), target.to_string());

    // per-param types
    for (tp, ip) in trait_fn.params.iter().zip(impl_fn.params.iter()) {
        let expected = subst_typeref(&tp.ty, subst);
        let actual = subst_typeref(&ip.ty, &impl_subst);
        if !typeref_eq(&expected, &actual) {
            return Err(format!(
                "method `{tn}.{m}` for `{tgt}`: parameter `{pname}` expected `{exp}`, got `{got}`",
                tn = trait_name, m = impl_fn.name, tgt = target,
                pname = ip.name,
                exp = typeref_display(&expected),
                got = typeref_display(&actual),
            ));
        }
    }
    // return type
    let expected_ret = trait_fn.return_type.as_ref().map(|t| subst_typeref(t, subst));
    let actual_ret = impl_fn.return_type.as_ref().map(|t| subst_typeref(t, &impl_subst));
    if !typeref_opt_eq(&expected_ret, &actual_ret) {
        return Err(format!(
            "method `{tn}.{m}` for `{tgt}`: return type expected `{exp}`, got `{got}`",
            tn = trait_name, m = impl_fn.name, tgt = target,
            exp = typeref_opt_display(&expected_ret),
            got = typeref_opt_display(&actual_ret),
        ));
    }
    // raises clause (`! E`)
    let expected_raises = trait_fn.raises.as_ref().map(|t| subst_typeref(t, subst));
    let actual_raises = impl_fn.raises.as_ref().map(|t| subst_typeref(t, &impl_subst));
    if !typeref_opt_eq(&expected_raises, &actual_raises) {
        return Err(format!(
            "method `{tn}.{m}` for `{tgt}`: raises clause expected `{exp}`, got `{got}`",
            tn = trait_name, m = impl_fn.name, tgt = target,
            exp = typeref_opt_display(&expected_raises),
            got = typeref_opt_display(&actual_raises),
        ));
    }
    Ok(())
}
