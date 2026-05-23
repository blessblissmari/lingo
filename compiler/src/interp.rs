//! Tree-walking interpreter for lingo v0.1.
//!
//! Currently supports:
//!   - top-level fns, consts, structs, enums, impl blocks
//!   - lexical scoping with **no shadowing** (compile-time-style runtime check)
//!   - `let` / `let mut`, with `name = expr` reassignment of `mut` bindings
//!   - `if` / `elif` / `else`, `for x in a..b`, `return`, `break`, `continue`
//!   - `match` with literal, wildcard, bind, and `Type.Variant(...)` patterns
//!   - struct literals `T{field: value, ...}` and field access `s.field`
//!   - method dispatch `value.method(args)` via `impl Type:`
//!   - keyword args (required when fn has >2 params)
//!   - arithmetic, comparison, boolean ops, `**`, `%`, `..` ranges
//!   - `print(...)` builtin

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::error::{LingoError, Span, Stage};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Range(i64, i64),
    Vec_(Rc<RefCell<Vec<Value>>>),
    /// Map with string-like keys (we store the key's display form so any value works).
    /// Entry insertion order is preserved.
    Map_(Rc<RefCell<Vec<(Value, Value)>>>),
    /// Wrapped return value from a fallible fn: Ok(v) or Err(e).
    /// Unwrapped by `?` or by `match`.
    Result_(Rc<std::result::Result<Value, Value>>),
    Struct {
        type_name: String,
        // Stored as a Vec<(name, value)> rather than a HashMap so that
        // debug-print iteration order is the struct's *declared* field
        // order (not insertion order from the literal, and not
        // hash-randomised).  Lookup is O(n) but structs are small
        // (< ~20 fields), and this lets us match the C backend's
        // declared-order output without dragging the struct decl into
        // every `Value::display` call.  v0.1.29.
        fields: Vec<(String, Value)>,
    },
    Enum {
        type_name: String,
        variant: String,
        payload: Vec<Value>,
    },
    /// v0.2.1: typed optional value, returned by builtins that may
    /// produce *no* value (today: `map.get(k)` only).  `Opt(None)` matches
    /// the `none` pattern; `Opt(Some(v))` matches `some(x)` (binding x).
    /// Display: `none` for absent, the inner value's display for present —
    /// so `print(counts.get(k))` keeps the v0.1.x wire format.
    /// Distinct from `None_` (which is the "no return value" sentinel used
    /// internally — those continue to display as `none` too).
    Opt(Option<Box<Value>>),
    None_,
}

impl Value {
    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Bool(_) => "bool".into(),
            Value::Str(_) => "str".into(),
            Value::Range(_, _) => "range".into(),
            Value::Vec_(_) => "vec".into(),
            Value::Map_(_) => "map".into(),
            Value::Result_(_) => "result".into(),
            Value::Struct { type_name, .. } => type_name.clone(),
            Value::Enum { type_name, .. } => type_name.clone(),
            Value::Opt(_) => "opt".into(),
            Value::None_ => "none".into(),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Str(s) => s.clone(),
            Value::Range(a, b) => format!("{a}..{b}"),
            Value::Vec_(rc) => {
                let parts: Vec<String> = rc.borrow().iter().map(|v| v.display()).collect();
                format!("vec[{}]", parts.join(", "))
            }
            Value::Map_(rc) => {
                let parts: Vec<String> = rc.borrow().iter()
                    .map(|(k, v)| format!("{}: {}", k.display(), v.display())).collect();
                format!("map{{{}}}", parts.join(", "))
            }
            Value::Result_(r) => match r.as_ref() {
                Ok(v) => format!("ok({})", v.display()),
                Err(e) => format!("err({})", e.display()),
            },
            Value::Struct { type_name, fields } => {
                // v0.1.29: walk in declared order (the Vec preserves it).
                // Pre-v0.1.29 we used a HashMap + alphabetical sort, which
                // disagreed with the C backend's declared-order printout
                // (debug_print.lingo).
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display()))
                    .collect();
                format!("{type_name}{{{}}}", parts.join(", "))
            }
            Value::Enum { type_name, variant, payload } => {
                if payload.is_empty() {
                    format!("{type_name}.{variant}")
                } else {
                    let parts: Vec<String> = payload.iter().map(|v| v.display()).collect();
                    format!("{type_name}.{variant}({})", parts.join(", "))
                }
            }
            // v0.2.1: Opt's display matches the v0.1.x wire format —
            // `none` for absent, the inner value's display for present.
            // So `print(counts.get(k))` reads identically across versions
            // and across backends.  `Some(v)` / `None` wrapper text is
            // *never* part of the display (use `match` to discriminate).
            Value::Opt(None) => "none".into(),
            Value::Opt(Some(v)) => v.display(),
            Value::None_ => "none".into(),
        }
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

#[derive(Debug)]
struct Scope {
    bindings: HashMap<String, Binding>,
}

#[derive(Debug, Clone)]
struct Binding {
    value: Value,
    is_mut: bool,
}

#[derive(Debug)]
pub struct Interp {
    fns: HashMap<String, FnDecl>,
    consts: HashMap<String, Value>,
    structs: HashMap<String, StructDecl>,
    enums: HashMap<String, EnumDecl>,
    methods: HashMap<String, HashMap<String, FnDecl>>, // type -> method -> fn
    /// trait declarations by name.
    traits: HashMap<String, TraitDecl>,
    /// `type -> trait -> method -> fn` table.
    /// Looked up during method dispatch when no inherent method matches.
    trait_impls: HashMap<String, HashMap<String, HashMap<String, FnDecl>>>,
    /// v0.2.3 — `impl From[E1] for E2:` impls.  Looked up by `?` when the
    /// inner err's runtime type doesn't match the caller's `raises` type;
    /// the registered `from(e: E1) -> E2` wraps the err.  No `else`
    /// fallback needed when an impl is in scope.
    from_impls: HashMap<(String, String), FnDecl>,
    scopes: Vec<Scope>,
    /// Set by `?` when the inner result is Err. The next statement boundary
    /// converts this into a `Flow::Raise(...)` so it propagates to the caller.
    pending_raise: Option<Value>,
    /// Stack of the currently-executing fn's `raises` type name (e.g.
    /// `"ParseErr"`).  Pushed on fn entry if the fn has `! E`, popped on
    /// exit.  Used by `?` to look up From impls for type coercion.
    current_fn_raises_e: Vec<String>,
    /// Command-line args passed to the `lingo` binary, available via `args()`.
    argv: Vec<String>,
}

#[derive(Debug)]
enum Flow {
    Normal,
    Return(Value),
    Raise(Value),
    Break,
    Continue,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    pub fn new() -> Self {
        Self {
            fns: HashMap::new(),
            consts: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            methods: HashMap::new(),
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            from_impls: HashMap::new(),
            scopes: Vec::new(),
            pending_raise: None,
            current_fn_raises_e: Vec::new(),
            argv: Vec::new(),
        }
    }

    /// Provide command-line args (everything after the .lingo file path).
    /// These are visible to lingo code via the `args()` builtin.
    pub fn with_argv(mut self, argv: Vec<String>) -> Self {
        self.argv = argv;
        self
    }

    pub fn run_program(&mut self, prog: &Program) -> Result<Value, LingoError> {
        self.register_items(prog, false)?;
        let main = self
            .fns
            .get("main")
            .cloned()
            .ok_or_else(|| LingoError::new(Stage::Resolve, "no `main` function", Span::dummy()))?;
        if !main.params.is_empty() {
            return Err(LingoError::new(
                Stage::Resolve,
                "`fn main` must take no parameters in v0.1",
                main.span,
            ));
        }
        self.call_fn(&main, vec![])
    }

    /// Register every top-level item from `prog` into the interpreter's
    /// tables, running the same two-pass resolution that `run_program` does
    /// (types/sigs first, then methods/consts/trait impls).  When
    /// `allow_replace` is true, duplicate declarations silently replace the
    /// previous version — this is the REPL's "redefine on the fly" mode.
    /// When false, duplicates are rejected (the file/program mode).
    pub fn register_items(&mut self, prog: &Program, allow_replace: bool)
        -> Result<(), LingoError>
    {
        // first pass: register types and signatures
        for item in &prog.items {
            match item {
                Item::Fn(f) => {
                    if !allow_replace && self.fns.contains_key(&f.name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate function `{}`", f.name),
                            f.span,
                        ));
                    }
                    self.fns.insert(f.name.clone(), f.clone());
                }
                Item::Struct(s) => {
                    if !allow_replace && self.structs.contains_key(&s.name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate struct `{}`", s.name),
                            s.span,
                        ));
                    }
                    self.structs.insert(s.name.clone(), s.clone());
                    // Replacing a struct invalidates its methods & trait impls.
                    if allow_replace {
                        self.methods.remove(&s.name);
                        self.trait_impls.remove(&s.name);
                    }
                }
                Item::Enum(e) => {
                    if !allow_replace && self.enums.contains_key(&e.name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate enum `{}`", e.name),
                            e.span,
                        ));
                    }
                    self.enums.insert(e.name.clone(), e.clone());
                    if allow_replace {
                        self.methods.remove(&e.name);
                        self.trait_impls.remove(&e.name);
                    }
                }
                Item::Trait(t) => {
                    if !allow_replace && self.traits.contains_key(&t.name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate trait `{}`", t.name),
                            t.span,
                        ));
                    }
                    self.traits.insert(t.name.clone(), t.clone());
                }
                Item::Impl(_) | Item::Const(_) | Item::ImplTrait(_) => {}
                // v0.3.0 — the resolver flattens away every `import` before
                // the program reaches us; if one shows up here, that's a bug.
                Item::Import(_) => unreachable!(
                    "Item::Import must be stripped by the module resolver"
                ),
            }
        }
        // v0.2.5: if any `impl From[..] for ..:` block is present but the
        // user didn't declare `trait From[E]:`, synthesize the built-in
        // trait so the general impl-resolution path can validate it
        // uniformly with user-defined generic traits.  Note the synthetic
        // trait carries no method bodies — it exists only to drive
        // arity-checking and the `from_impls` denormalized view.
        let needs_synthetic_from = prog.items.iter().any(|it| matches!(
            it,
            Item::ImplTrait(b) if b.trait_name == "From"
        )) && !self.traits.contains_key("From");
        if needs_synthetic_from {
            self.traits.insert(
                "From".into(),
                TraitDecl {
                    name: "From".into(),
                    type_params: vec!["E".into()],
                    methods: Vec::new(),
                    span: Span::new(0, 0),
                },
            );
        }
        // second pass: register impl methods and evaluate consts
        for item in &prog.items {
            match item {
                Item::Impl(b) => {
                    if !self.structs.contains_key(&b.target) && !self.enums.contains_key(&b.target) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("`impl` for unknown type `{}`", b.target),
                            b.span,
                        ));
                    }
                    let entry = self.methods.entry(b.target.clone()).or_default();
                    for m in &b.methods {
                        if !allow_replace && entry.contains_key(&m.name) {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("duplicate method `{}.{}`", b.target, m.name),
                                m.span,
                            ));
                        }
                        entry.insert(m.name.clone(), m.clone());
                    }
                }
                Item::Const(c) => {
                    let v = self.eval_const(&c.value)?;
                    if !allow_replace && self.consts.contains_key(&c.name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate const `{}`", c.name),
                            c.span,
                        ));
                    }
                    self.consts.insert(c.name.clone(), v);
                }
                Item::ImplTrait(b) => {
                    // v0.2.5: every `impl Trait[..] for Target:` now goes
                    // through the general path.  The trait must exist
                    // (`From` is auto-synthesized above when needed) and
                    // the bracketed `trait_args` must match the trait's
                    // declared `type_params` arity.  After validation,
                    // `From`-impls additionally populate the denormalized
                    // `from_impls` lookup table the `?` operator consults
                    // when err types mismatch.
                    let trait_decl = self.traits.get(&b.trait_name).cloned().ok_or_else(|| {
                        LingoError::new(
                            Stage::Resolve,
                            format!("`impl {} for {}` refers to unknown trait `{}`",
                                    b.trait_name, b.target, b.trait_name),
                            b.span,
                        )
                    })?;
                    if b.trait_args.len() != trait_decl.type_params.len() {
                        let want = trait_decl.type_params.len();
                        let got = b.trait_args.len();
                        return Err(LingoError::new(
                            Stage::Resolve,
                            if want == 0 {
                                format!("trait `{}` takes no type parameters, but impl provided {} (`[{}]`)",
                                        b.trait_name, got, b.trait_args.join(", "))
                            } else {
                                format!("trait `{}` declares {} type parameter(s) ({}); impl provided {}",
                                        b.trait_name, want, trait_decl.type_params.join(", "), got)
                            },
                            b.span,
                        ));
                    }
                    // v0.2.5: `From`-specific bookkeeping piggybacks on the
                    // general path.  Validate the single method shape and
                    // register it into `from_impls` for `?` to find.
                    if b.trait_name == "From" {
                        let from_ty = b.trait_args[0].clone();
                        let to_ty = b.target.clone();
                        if b.methods.len() != 1 || b.methods[0].name != "from" {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                "`impl From[..] for ..` must contain exactly one method `fn from(e: <E1>) -> <E2>`",
                                b.span,
                            ));
                        }
                        if !allow_replace && self.from_impls.contains_key(&(from_ty.clone(), to_ty.clone())) {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("duplicate `impl From[{}] for {}`", from_ty, to_ty),
                                b.span,
                            ));
                        }
                        self.from_impls.insert((from_ty, to_ty), b.methods[0].clone());
                        continue;
                    }
                    // The target type must exist (struct or enum).
                    if !self.structs.contains_key(&b.target) && !self.enums.contains_key(&b.target) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("`impl {} for {}` refers to unknown type `{}`",
                                    b.trait_name, b.target, b.target),
                            b.span,
                        ));
                    }
                    // Check conformance: every required trait method must be implemented
                    // (or have a default in the trait). Impl methods must be declared on the trait.
                    let mut impl_by_name: HashMap<String, FnDecl> = HashMap::new();
                    for m in &b.methods {
                        if impl_by_name.insert(m.name.clone(), m.clone()).is_some() {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("duplicate method `{}.{}` in impl {} for {}",
                                        b.trait_name, m.name, b.trait_name, b.target),
                                m.span,
                            ));
                        }
                    }
                    // every impl method must be declared on the trait
                    for (mname, mdecl) in &impl_by_name {
                        if !trait_decl.methods.iter().any(|tm| &tm.decl.name == mname) {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("method `{}` is not part of trait `{}`",
                                        mname, b.trait_name),
                                mdecl.span,
                            ));
                        }
                    }
                    // every required (no-default) trait method must be implemented
                    //
                    // v0.2.6: also check that the impl method's
                    // signature matches the trait method's signature
                    // after substituting `type_params[i] ->
                    // trait_args[i]` and `Self -> target`.  Mirrors the
                    // identical check in the C backend.
                    let subst = crate::ast::build_trait_subst(
                        &trait_decl.type_params,
                        &b.trait_args,
                        &b.target,
                    );
                    let mut resolved: HashMap<String, FnDecl> = HashMap::new();
                    for tm in &trait_decl.methods {
                        if let Some(m) = impl_by_name.get(&tm.decl.name) {
                            if let Err(msg) = crate::ast::check_trait_method_sig(
                                &b.trait_name, &b.target, &tm.decl, m, &subst,
                            ) {
                                return Err(LingoError::new(Stage::Resolve, msg, m.span));
                            }
                            resolved.insert(tm.decl.name.clone(), m.clone());
                        } else if tm.has_default {
                            resolved.insert(tm.decl.name.clone(), tm.decl.clone());
                        } else {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("`impl {} for {}` missing required method `{}`",
                                        b.trait_name, b.target, tm.decl.name),
                                b.span,
                            ));
                        }
                    }
                    let entry = self.trait_impls
                        .entry(b.target.clone())
                        .or_default();
                    if !allow_replace && entry.contains_key(&b.trait_name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate `impl {} for {}`", b.trait_name, b.target),
                            b.span,
                        ));
                    }
                    entry.insert(b.trait_name.clone(), resolved);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// REPL entry: ensure there's a persistent root scope, then exec a
    /// single statement against it.  Used to evaluate top-level let/expr/print
    /// statements between REPL prompts.  Returns the value produced (for
    /// bare-expression statements that's the expression's value; everything
    /// else returns `Value::None_`).
    pub fn exec_top_stmt(&mut self, s: &Stmt) -> Result<Value, LingoError> {
        if self.scopes.is_empty() {
            self.scopes.push(Scope { bindings: HashMap::new() });
        }
        match self.exec_stmt(s)? {
            Flow::Normal => Ok(Value::None_),
            Flow::Return(v) => Ok(v),
            Flow::Break | Flow::Continue => Err(LingoError::new(
                Stage::Resolve,
                "`break` / `continue` not valid at REPL top level",
                Span::dummy(),
            )),
            Flow::Raise(e) => Err(LingoError::new(
                Stage::Runtime,
                format!("uncaught error at REPL top level: {}", e.display()),
                Span::dummy(),
            )),
        }
    }

    fn eval_const(&self, e: &Expr) -> Result<Value, LingoError> {
        match &e.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(f) => Ok(Value::Float(*f)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::None_ => Ok(Value::None_),
            ExprKind::Unary(UnOp::Neg, inner) => match self.eval_const(inner)? {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                v => Err(LingoError::new(
                    Stage::Resolve,
                    format!("cannot negate {} in const expression", v.type_name()),
                    e.span,
                )),
            },
            _ => Err(LingoError::new(
                Stage::Resolve,
                "const value must be a literal expression",
                e.span,
            )),
        }
    }

    fn call_fn(&mut self, decl: &FnDecl, args: Vec<Value>) -> Result<Value, LingoError> {
        if args.len() != decl.params.len() {
            return Err(LingoError::new(
                Stage::Runtime,
                format!(
                    "function `{}` expects {} arg(s), got {}",
                    decl.name,
                    decl.params.len(),
                    args.len()
                ),
                decl.span,
            ));
        }
        let saved = std::mem::take(&mut self.scopes);
        self.scopes.push(Scope { bindings: HashMap::new() });
        for (p, v) in decl.params.iter().zip(args) {
            self.scopes.last_mut().unwrap().bindings.insert(
                p.name.clone(),
                Binding { value: v, is_mut: p.name == "self" }, // self.field = ... allowed
            );
        }
        // Push the caller's raises type so `?` can find From impls.
        let pushed_raises = decl.raises.as_ref().map(|t| t.name.clone());
        if let Some(ref e) = pushed_raises {
            self.current_fn_raises_e.push(e.clone());
        }
        let flow = self.exec_block(&decl.body);
        if pushed_raises.is_some() {
            self.current_fn_raises_e.pop();
        }
        self.scopes = saved;
        let is_fallible = decl.raises.is_some();
        match flow? {
            Flow::Return(v) => {
                if is_fallible {
                    Ok(Value::Result_(Rc::new(Ok(v))))
                } else {
                    Ok(v)
                }
            }
            Flow::Normal => {
                if is_fallible {
                    Ok(Value::Result_(Rc::new(Ok(Value::None_))))
                } else {
                    Ok(Value::None_)
                }
            }
            Flow::Raise(e) => {
                if is_fallible {
                    Ok(Value::Result_(Rc::new(Err(e))))
                } else {
                    Err(LingoError::new(
                        Stage::Runtime,
                        format!(
                            "`raise` in non-fallible fn `{}` — declare `! ErrorType` after the return type",
                            decl.name
                        ),
                        decl.span,
                    ))
                }
            }
            Flow::Break => Err(LingoError::new(
                Stage::Runtime,
                "`break` outside loop",
                decl.span,
            )),
            Flow::Continue => Err(LingoError::new(
                Stage::Runtime,
                "`continue` outside loop",
                decl.span,
            )),
        }
    }

    fn exec_block(&mut self, block: &Block) -> Result<Flow, LingoError> {
        self.scopes.push(Scope { bindings: HashMap::new() });
        let mut flow = Flow::Normal;
        for s in &block.stmts {
            flow = self.exec_stmt(s)?;
            if !matches!(flow, Flow::Normal) {
                break;
            }
        }
        self.scopes.pop();
        Ok(flow)
    }

    /// Evaluate an expression and surface any `?`-triggered error as a Flow::Raise.
    /// Returns Ok(Ok(v)) for normal evaluation, Ok(Err(Flow::Raise(e))) when `?` fired.
    fn eval_stmt(&mut self, e: &Expr) -> Result<std::result::Result<Value, Flow>, LingoError> {
        let v = self.eval(e)?;
        if let Some(err) = self.pending_raise.take() {
            return Ok(Err(Flow::Raise(err)));
        }
        Ok(Ok(v))
    }

    fn exec_stmt(&mut self, s: &Stmt) -> Result<Flow, LingoError> {
        match s {
            Stmt::Let { is_mut, name, ty: _, value, span } => {
                let v = match self.eval_stmt(value)? { Ok(v) => v, Err(f) => return Ok(f) };
                for scope in self.scopes.iter().rev() {
                    if scope.bindings.contains_key(name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("`{}` already in scope (shadowing is forbidden)", name),
                            *span,
                        ));
                    }
                }
                if self.consts.contains_key(name) || self.fns.contains_key(name) {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("`{}` already declared at module scope", name),
                        *span,
                    ));
                }
                self.scopes
                    .last_mut()
                    .unwrap()
                    .bindings
                    .insert(name.clone(), Binding { value: v, is_mut: *is_mut });
                Ok(Flow::Normal)
            }
            Stmt::Assign { target, value, span } => {
                let v = match self.eval_stmt(value)? { Ok(v) => v, Err(f) => return Ok(f) };
                match target {
                    AssignTarget::Name(name) => {
                        for scope in self.scopes.iter_mut().rev() {
                            if let Some(b) = scope.bindings.get_mut(name) {
                                if !b.is_mut {
                                    return Err(LingoError::new(
                                        Stage::Runtime,
                                        format!("cannot assign to immutable `{}` (declare with `let mut`)", name),
                                        *span,
                                    ));
                                }
                                b.value = v;
                                return Ok(Flow::Normal);
                            }
                        }
                        Err(LingoError::new(
                            Stage::Runtime,
                            format!("`{}` is not defined", name),
                            *span,
                        ))
                    }
                    AssignTarget::Field(obj_expr, fname) => {
                        // only `self.field = ...` is supported in v0.1.1
                        if !matches!(obj_expr.kind, ExprKind::Self_) {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                "in v0.1.1, only `self.field = ...` is allowed (no struct mutation through other handles yet)",
                                *span,
                            ));
                        }
                        for scope in self.scopes.iter_mut().rev() {
                            if let Some(b) = scope.bindings.get_mut("self") {
                                if let Value::Struct { fields, .. } = &mut b.value {
                                    // v0.1.29: fields is now Vec<(String, Value)>, so
                                    // we linear-scan for the slot.  Structs are small.
                                    if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| k == fname) {
                                        *slot = v;
                                        return Ok(Flow::Normal);
                                    }
                                    return Err(LingoError::new(
                                        Stage::Runtime,
                                        format!("no field `{}` on this struct", fname),
                                        *span,
                                    ));
                                } else {
                                    return Err(LingoError::new(
                                        Stage::Runtime,
                                        "`self` is not a struct",
                                        *span,
                                    ));
                                }
                            }
                        }
                        Err(LingoError::new(
                            Stage::Runtime,
                            "`self` is not in scope",
                            *span,
                        ))
                    }
                }
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => match self.eval_stmt(e)? { Ok(v) => v, Err(f) => return Ok(f) },
                    None => Value::None_,
                };
                Ok(Flow::Return(v))
            }
            Stmt::Raise { value, .. } => {
                let v = match self.eval_stmt(value)? { Ok(v) => v, Err(f) => return Ok(f) };
                Ok(Flow::Raise(v))
            }
            Stmt::If { arms, else_block, .. } => {
                for (cond, block) in arms {
                    let c = match self.eval_stmt(cond)? { Ok(v) => v, Err(f) => return Ok(f) };
                    match c {
                        Value::Bool(true) => return self.exec_block(block),
                        Value::Bool(false) => continue,
                        v => {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                format!("`if` condition must be bool, got {}", v.type_name()),
                                cond.span,
                            ))
                        }
                    }
                }
                if let Some(b) = else_block {
                    self.exec_block(b)
                } else {
                    Ok(Flow::Normal)
                }
            }
            Stmt::For { var, iter, body, span: _ } => {
                // v0.1.28: the for-loop variable is a fresh binding — apply the
                // same no-shadowing rule as `let`.  `_` is the "don't bind" sigil
                // and is always allowed.  Pre-v0.1.28, `let i = 0; for i in 0..3`
                // silently shadowed `i` for the duration of the loop, which
                // disagreed with DECISIONS.md.
                if var != "_" {
                    for scope in self.scopes.iter().rev() {
                        if scope.bindings.contains_key(var) {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("`{}` already in scope (shadowing is forbidden)", var),
                                iter.span,
                            ));
                        }
                    }
                    if self.consts.contains_key(var) || self.fns.contains_key(var) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("`{}` already declared at module scope", var),
                            iter.span,
                        ));
                    }
                }
                // `for _ in forever:` — infinite loop. Only `_` is allowed as the
                // loop variable here; the iterable produces no value.
                if matches!(iter.kind, ExprKind::Forever) {
                    if var != "_" {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!(
                                "`for {} in forever:` is not allowed — the loop variable must be `_` because `forever` yields no value",
                                var
                            ),
                            iter.span,
                        ));
                    }
                    loop {
                        self.scopes.push(Scope { bindings: HashMap::new() });
                        let flow = self.exec_block_inline(body)?;
                        self.scopes.pop();
                        match flow {
                            Flow::Break => break,
                            Flow::Continue | Flow::Normal => continue,
                            Flow::Return(v) => return Ok(Flow::Return(v)),
                            Flow::Raise(e) => return Ok(Flow::Raise(e)),
                        }
                    }
                    return Ok(Flow::Normal);
                }
                let it = match self.eval_stmt(iter)? { Ok(v) => v, Err(f) => return Ok(f) };
                // collect the items to iterate (snapshot — no mutation of the source during the loop)
                let items: Vec<Value> = match it {
                    Value::Range(a, b) => (a..b).map(Value::Int).collect(),
                    Value::Vec_(rc) => rc.borrow().clone(),
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    v => {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("`for` needs a range, vec, or str, got {}", v.type_name()),
                            iter.span,
                        ))
                    }
                };
                for v in items {
                    self.scopes.push(Scope { bindings: HashMap::new() });
                    self.scopes.last_mut().unwrap().bindings.insert(
                        var.clone(),
                        Binding { value: v, is_mut: false },
                    );
                    let flow = self.exec_block_inline(body)?;
                    self.scopes.pop();
                    match flow {
                        Flow::Break => break,
                        Flow::Continue | Flow::Normal => continue,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Raise(e) => return Ok(Flow::Raise(e)),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Match { scrutinee, arms, span } => {
                let v = match self.eval_stmt(scrutinee)? { Ok(v) => v, Err(f) => return Ok(f) };
                for arm in arms {
                    self.scopes.push(Scope { bindings: HashMap::new() });
                    if self.pattern_match(&arm.pattern, &v)? {
                        let flow = self.exec_block_inline(&arm.body)?;
                        self.scopes.pop();
                        return Ok(flow);
                    }
                    self.scopes.pop();
                }
                Err(LingoError::new(
                    Stage::Runtime,
                    format!("no match arm matched value of type {}", v.type_name()),
                    *span,
                ))
            }
            Stmt::Break(_) => Ok(Flow::Break),
            Stmt::Continue(_) => Ok(Flow::Continue),
            Stmt::Expr(e) => {
                self.eval(e)?;
                if let Some(err) = self.pending_raise.take() {
                    return Ok(Flow::Raise(err));
                }
                Ok(Flow::Normal)
            }
        }
    }

    fn exec_block_inline(&mut self, block: &Block) -> Result<Flow, LingoError> {
        let mut flow = Flow::Normal;
        for s in &block.stmts {
            flow = self.exec_stmt(s)?;
            if !matches!(flow, Flow::Normal) {
                break;
            }
        }
        Ok(flow)
    }

    fn pattern_match(&mut self, pat: &Pattern, v: &Value) -> Result<bool, LingoError> {
        match pat {
            Pattern::Wildcard(_) => Ok(true),
            Pattern::Bind(name, span) => {
                // bind never fails; introduces a binding.  `_` is handled by
                // `Pattern::Wildcard` above — anything reaching here has a real
                // name we must record.
                //
                // The topmost scope is the per-arm scope pushed by `Stmt::Match`
                // before calling `pattern_match`.  Two checks:
                //   1. v0.1.28: the bind name mustn't shadow an enclosing-scope
                //      binding, a const, or a fn.  Pre-v0.1.28, a `let x = 1`
                //      followed by `match Opt.Some(42): Some(x): ...` silently
                //      shadowed `x` inside the arm, disagreeing with
                //      DECISIONS.md.
                //   2. The name mustn't be bound twice in the same pattern
                //      (`Pair(x, x)`).  Different diagnostic — matches the
                //      pre-v0.1.28 wording.
                let scopes_len = self.scopes.len();
                for scope in self.scopes[..scopes_len.saturating_sub(1)].iter().rev() {
                    if scope.bindings.contains_key(name) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("`{}` already in scope (shadowing is forbidden)", name),
                            *span,
                        ));
                    }
                }
                if self.consts.contains_key(name) || self.fns.contains_key(name) {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("`{}` already declared at module scope", name),
                        *span,
                    ));
                }
                let scope = self.scopes.last_mut().unwrap();
                if scope.bindings.contains_key(name) {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("`{}` already bound in this scope", name),
                        *span,
                    ));
                }
                scope.bindings.insert(
                    name.clone(),
                    Binding { value: v.clone(), is_mut: false },
                );
                Ok(true)
            }
            Pattern::Literal(lit, _) => Ok(match (lit, v) {
                (PatLit::Int(a), Value::Int(b)) => a == b,
                (PatLit::Bool(a), Value::Bool(b)) => a == b,
                (PatLit::Str(a), Value::Str(b)) => a == b,
                _ => false,
            }),
            Pattern::Variant { type_name, variant, sub, span } => {
                match v {
                    Value::Enum { type_name: tn, variant: var, payload } => {
                        if let Some(t) = type_name {
                            if t != tn {
                                return Ok(false);
                            }
                        }
                        if variant != var {
                            return Ok(false);
                        }
                        if sub.len() != payload.len() {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                format!(
                                    "variant `{}` expects {} field(s), pattern has {}",
                                    var, payload.len(), sub.len()
                                ),
                                *span,
                            ));
                        }
                        for (p, val) in sub.iter().zip(payload.iter()) {
                            if !self.pattern_match(p, val)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    Value::Result_(rc) => {
                        // built-in `ok(v)` / `err(e)` patterns on a fallible-fn result
                        if type_name.is_some() {
                            return Ok(false);
                        }
                        match (variant.as_str(), rc.as_ref()) {
                            ("ok", Ok(inner)) => {
                                if sub.len() != 1 {
                                    return Err(LingoError::new(Stage::Runtime,
                                        format!("`ok(...)` pattern needs exactly 1 field, got {}", sub.len()), *span));
                                }
                                self.pattern_match(&sub[0], inner)
                            }
                            ("err", Err(inner)) => {
                                if sub.len() != 1 {
                                    return Err(LingoError::new(Stage::Runtime,
                                        format!("`err(...)` pattern needs exactly 1 field, got {}", sub.len()), *span));
                                }
                                self.pattern_match(&sub[0], inner)
                            }
                            _ => Ok(false),
                        }
                    }
                    Value::None_ => Ok(type_name.is_none() && variant == "none" && sub.is_empty()),
                    Value::Opt(opt) => {
                        // v0.2.1: `none` and `some(...)` patterns on Opt[T].
                        // Bare-variant only (no `Opt.Some` / `Opt.None`
                        // namespacing today).
                        if type_name.is_some() {
                            return Ok(false);
                        }
                        match (variant.as_str(), opt) {
                            ("none", None) => Ok(sub.is_empty()),
                            ("some", Some(inner)) => {
                                if sub.len() != 1 {
                                    return Err(LingoError::new(
                                        Stage::Runtime,
                                        format!("`some(...)` pattern needs exactly 1 field, got {}", sub.len()),
                                        *span,
                                    ));
                                }
                                self.pattern_match(&sub[0], inner)
                            }
                            _ => Ok(false),
                        }
                    }
                    Value::Bool(b) => {
                        // allow `true`/`false` to also be matched as bare variants
                        if type_name.is_none()
                            && ((variant == "true" && *b) || (variant == "false" && !*b))
                        {
                            Ok(true)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(b) = scope.bindings.get(name) {
                return Some(b.value.clone());
            }
        }
        if let Some(v) = self.consts.get(name) {
            return Some(v.clone());
        }
        None
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, LingoError> {
        // short-circuit: a `?` already fired and we haven't reached a statement boundary yet
        if self.pending_raise.is_some() {
            return Ok(Value::None_);
        }
        match &e.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(f) => Ok(Value::Float(*f)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::None_ => Ok(Value::None_),
            ExprKind::Forever => Err(LingoError::new(
                Stage::Resolve,
                "`forever` is not a value — it can only be the iterable of `for _ in forever:`",
                e.span,
            )),
            ExprKind::Self_ => self.lookup("self").ok_or_else(|| {
                LingoError::new(Stage::Runtime, "`self` is not in scope", e.span)
            }),
            ExprKind::Ident(name) => self.lookup(name).ok_or_else(|| {
                LingoError::new(
                    Stage::Runtime,
                    format!("`{}` is not defined", name),
                    e.span,
                )
            }),
            ExprKind::PrintBuiltin => Err(LingoError::new(
                Stage::Runtime,
                "`print` can only be called, not used as a value",
                e.span,
            )),
            ExprKind::StructLit { name, fields } => {
                let decl = self.structs.get(name).cloned().ok_or_else(|| {
                    LingoError::new(
                        Stage::Runtime,
                        format!("unknown struct `{}`", name),
                        e.span,
                    )
                })?;
                let mut map = HashMap::new();
                // every declared field must be set exactly once
                for (fname, fexpr) in fields {
                    if !decl.fields.iter().any(|f| &f.name == fname) {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("struct `{}` has no field `{}`", name, fname),
                            fexpr.span,
                        ));
                    }
                    if map.contains_key(fname) {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("field `{}` set twice", fname),
                            fexpr.span,
                        ));
                    }
                    let val = self.eval(fexpr)?;
                    map.insert(fname.clone(), val);
                }
                for f in &decl.fields {
                    if !map.contains_key(&f.name) {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("missing field `{}` in struct literal", f.name),
                            e.span,
                        ));
                    }
                }
                // v0.1.29: materialize as a Vec in declared order so
                // debug-print iteration order matches the struct decl
                // (and the C backend).  `map` only existed to enforce
                // "every field set exactly once" — we drain it back out
                // in decl order here.
                let ordered: Vec<(String, Value)> = decl
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), map.remove(&f.name).expect("validated above")))
                    .collect();
                Ok(Value::Struct {
                    type_name: name.clone(),
                    fields: ordered,
                })
            }
            ExprKind::Field(lhs, name) => {
                // `Type.Variant` (nullary enum variant)
                if let ExprKind::Ident(type_name) = &lhs.kind {
                    if let Some(enum_decl) = self.enums.get(type_name).cloned() {
                        if enum_decl.variants.iter().any(|v| &v.name == name) {
                            return Ok(Value::Enum {
                                type_name: type_name.clone(),
                                variant: name.clone(),
                                payload: Vec::new(),
                            });
                        }
                        // fall through to error
                    }
                }
                let v = self.eval(lhs)?;
                match v {
                    Value::Struct { fields, .. } => fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).ok_or_else(|| {
                        LingoError::new(
                            Stage::Runtime,
                            format!("no field `{}`", name),
                            e.span,
                        )
                    }),
                    other => Err(LingoError::new(
                        Stage::Runtime,
                        format!("cannot read `.{}` on {}", name, other.type_name()),
                        e.span,
                    )),
                }
            }
            ExprKind::Unary(op, inner) => {
                let v = self.eval(inner)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
                    (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (op, v) => Err(LingoError::new(
                        Stage::Runtime,
                        format!("cannot apply {:?} to {}", op, v.type_name()),
                        e.span,
                    )),
                }
            }
            ExprKind::Binary(op, l, r) => {
                match op {
                    BinOp::And => {
                        let lv = self.eval(l)?;
                        return match lv {
                            Value::Bool(false) => Ok(Value::Bool(false)),
                            Value::Bool(true) => match self.eval(r)? {
                                Value::Bool(b) => Ok(Value::Bool(b)),
                                v => Err(LingoError::new(
                                    Stage::Runtime,
                                    format!("`and` requires bool, got {}", v.type_name()),
                                    r.span,
                                )),
                            },
                            v => Err(LingoError::new(
                                Stage::Runtime,
                                format!("`and` requires bool, got {}", v.type_name()),
                                l.span,
                            )),
                        };
                    }
                    BinOp::Or => {
                        let lv = self.eval(l)?;
                        return match lv {
                            Value::Bool(true) => Ok(Value::Bool(true)),
                            Value::Bool(false) => match self.eval(r)? {
                                Value::Bool(b) => Ok(Value::Bool(b)),
                                v => Err(LingoError::new(
                                    Stage::Runtime,
                                    format!("`or` requires bool, got {}", v.type_name()),
                                    r.span,
                                )),
                            },
                            v => Err(LingoError::new(
                                Stage::Runtime,
                                format!("`or` requires bool, got {}", v.type_name()),
                                l.span,
                            )),
                        };
                    }
                    _ => {}
                }
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                bin_op(*op, lv, rv, e.span)
            }
            ExprKind::Range(l, r) => {
                let a = match self.eval(l)? {
                    Value::Int(n) => n,
                    v => {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("range start must be int, got {}", v.type_name()),
                            l.span,
                        ))
                    }
                };
                let b = match self.eval(r)? {
                    Value::Int(n) => n,
                    v => {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("range end must be int, got {}", v.type_name()),
                            r.span,
                        ))
                    }
                };
                Ok(Value::Range(a, b))
            }
            ExprKind::Call(callee, args) => self.eval_call(callee, args, e.span),
            ExprKind::VecLit(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for it in items {
                    vals.push(self.eval(it)?);
                    if self.pending_raise.is_some() {
                        return Ok(Value::None_);
                    }
                }
                Ok(Value::Vec_(Rc::new(RefCell::new(vals))))
            }
            ExprKind::MapLit(entries) => {
                let mut out: Vec<(Value, Value)> = Vec::with_capacity(entries.len());
                for (ke, ve) in entries {
                    let k = self.eval(ke)?;
                    if self.pending_raise.is_some() { return Ok(Value::None_); }
                    let v = self.eval(ve)?;
                    if self.pending_raise.is_some() { return Ok(Value::None_); }
                    // de-dup: if the key already exists (by structural equality), overwrite.
                    let mut replaced = false;
                    for slot in out.iter_mut() {
                        if values_eq(&slot.0, &k) {
                            slot.1 = v.clone();
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        out.push((k, v));
                    }
                }
                Ok(Value::Map_(Rc::new(RefCell::new(out))))
            }
            ExprKind::FString(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        FStringPart::Lit(lit) => s.push_str(lit),
                        FStringPart::Expr(inner) => {
                            let v = self.eval(inner)?;
                            if self.pending_raise.is_some() {
                                return Ok(Value::None_);
                            }
                            s.push_str(&v.display());
                        }
                    }
                }
                Ok(Value::Str(s))
            }
            ExprKind::Try { inner, fallback } => {
                let v = self.eval(inner)?;
                if self.pending_raise.is_some() {
                    return Ok(Value::None_);
                }
                match v {
                    Value::Result_(rc) => match rc.as_ref() {
                        Ok(val) => Ok(val.clone()),
                        Err(err) => {
                            // Three error-coercion paths:
                            //   1. explicit `expr? else fb` (v0.2.2): use `fb`.
                            //   2. types already match — propagate as-is.
                            //   3. types differ — consult `from_impls`
                            //      registered by `impl From[E1] for E2:`
                            //      blocks (v0.2.3) and call the `from` fn
                            //      to wrap the err in the caller's type.
                            //      If no impl is in scope, propagate the
                            //      raw err (callers can still match on it
                            //      reflectively).
                            let raised = if let Some(fb) = fallback {
                                let v = self.eval(fb)?;
                                if self.pending_raise.is_some() {
                                    return Ok(Value::None_);
                                }
                                v
                            } else {
                                let err_ty = err.type_name();
                                let caller_e = self.current_fn_raises_e.last().cloned();
                                match caller_e {
                                    Some(target) if target != err_ty => {
                                        if let Some(from_fn) = self.from_impls
                                            .get(&(err_ty, target))
                                            .cloned()
                                        {
                                            self.call_fn(&from_fn, vec![err.clone()])?
                                        } else {
                                            err.clone()
                                        }
                                    }
                                    _ => err.clone(),
                                }
                            };
                            self.pending_raise = Some(raised);
                            Ok(Value::None_)
                        }
                    },
                    other => Err(LingoError::new(
                        Stage::Runtime,
                        format!(
                            "`?` requires a fallible result, got {}. did you forget `! E` on the called fn?",
                            other.type_name()
                        ),
                        e.span,
                    )),
                }
            }
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Arg], call_span: Span) -> Result<Value, LingoError> {
        // print builtin
        if matches!(callee.kind, ExprKind::PrintBuiltin) {
            let mut out = String::new();
            for (i, a) in args.iter().enumerate() {
                if a.name.is_some() {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        "`print` does not take keyword arguments",
                        a.span,
                    ));
                }
                if i > 0 {
                    out.push(' ');
                }
                let v = self.eval(&a.value)?;
                out.push_str(&v.display());
            }
            println!("{out}");
            return Ok(Value::None_);
        }

        // Type.Variant(args) — enum variant construction
        if let ExprKind::Field(lhs, vname) = &callee.kind {
            if let ExprKind::Ident(type_name) = &lhs.kind {
                if let Some(enum_decl) = self.enums.get(type_name).cloned() {
                    if let Some(variant) = enum_decl.variants.iter().find(|v| &v.name == vname) {
                        if args.iter().any(|a| a.name.is_some()) {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                "enum variant arguments must be positional",
                                call_span,
                            ));
                        }
                        if args.len() != variant.payload.len() {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                format!(
                                    "variant `{}.{}` expects {} value(s), got {}",
                                    type_name, vname, variant.payload.len(), args.len()
                                ),
                                call_span,
                            ));
                        }
                        let mut values = Vec::new();
                        for a in args {
                            values.push(self.eval(&a.value)?);
                        }
                        return Ok(Value::Enum {
                            type_name: type_name.clone(),
                            variant: vname.clone(),
                            payload: values,
                        });
                    }
                    // it's a static method on the enum type
                    if let Some(decl) = self.methods.get(type_name).and_then(|m| m.get(vname)).cloned() {
                        let values = self.resolve_args(&decl, args, call_span)?;
                        return self.call_fn(&decl, values);
                    }
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("no variant or method `{}` on enum `{}`", vname, type_name),
                        callee.span,
                    ));
                }
                if self.structs.contains_key(type_name) {
                    // static method: `Type.method(args)`
                    if let Some(decl) = self.methods.get(type_name).and_then(|m| m.get(vname)).cloned() {
                        let values = self.resolve_args(&decl, args, call_span)?;
                        return self.call_fn(&decl, values);
                    }
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `{}` on struct `{}`", vname, type_name),
                        callee.span,
                    ));
                }
            }
            // method call on a value: receiver.method(args)
            let receiver = self.eval(lhs)?;
            // builtin methods on `vec` and `str`
            if let Some(v) = self.call_builtin_method(&receiver, vname, args, call_span)? {
                return Ok(v);
            }
            let type_name = match &receiver {
                Value::Struct { type_name, .. } => type_name.clone(),
                Value::Enum { type_name, .. } => type_name.clone(),
                v => {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `{}` on {}", vname, v.type_name()),
                        callee.span,
                    ))
                }
            };
            // method resolution order:
            //   1. inherent `impl <Type>:` methods
            //   2. any `impl <Trait> for <Type>:` that defines this method
            //      (if multiple traits implement a method with the same name,
            //      we report an ambiguity error rather than silently picking one)
            let decl = if let Some(d) = self.methods.get(&type_name).and_then(|m| m.get(vname)).cloned() {
                d
            } else {
                let trait_table = self.trait_impls.get(&type_name);
                let matches: Vec<(String, FnDecl)> = trait_table
                    .map(|tt| {
                        tt.iter()
                            .filter_map(|(tname, ms)| ms.get(vname).map(|m| (tname.clone(), m.clone())))
                            .collect()
                    })
                    .unwrap_or_default();
                match matches.len() {
                    0 => return Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `{}` on `{}`", vname, type_name),
                        callee.span,
                    )),
                    1 => matches.into_iter().next().unwrap().1,
                    _ => {
                        let trait_names: Vec<String> = matches.iter().map(|(t, _)| t.clone()).collect();
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!(
                                "ambiguous method `{}.{}` — implemented by traits: {}. \
                                 use `<Trait>.{}(x, ...)` to disambiguate (not yet supported in v0.1.6)",
                                type_name, vname, trait_names.join(", "), vname
                            ),
                            callee.span,
                        ));
                    }
                }
            };
            // method must take self as first param
            if decl.params.first().map(|p| p.name.as_str()) != Some("self") {
                return Err(LingoError::new(
                    Stage::Runtime,
                    format!("`{}.{}` is a static method; call it as `{}.{}(...)` not `x.{}(...)`",
                            type_name, vname, type_name, vname, vname),
                    callee.span,
                ));
            }
            // resolve remaining args
            let rest_decl = FnDecl {
                params: decl.params[1..].to_vec(),
                ..decl.clone()
            };
            let mut values = vec![receiver];
            let rest_values = self.resolve_args(&rest_decl, args, call_span)?;
            values.extend(rest_values);
            return self.call_fn(&decl, values);
        }

        let name = match &callee.kind {
            ExprKind::Ident(s) => s.clone(),
            _ => {
                return Err(LingoError::new(
                    Stage::Runtime,
                    "can only call named functions in v0.1",
                    callee.span,
                ))
            }
        };
        // built-in free functions
        if let Some(v) = self.call_builtin_free(&name, args, call_span)? {
            return Ok(v);
        }
        let decl = self
            .fns
            .get(&name)
            .cloned()
            .ok_or_else(|| {
                LingoError::new(
                    Stage::Runtime,
                    format!("function `{}` is not defined", name),
                    callee.span,
                )
            })?;
        let values = self.resolve_args(&decl, args, call_span)?;
        self.call_fn(&decl, values)
    }

    /// Returns Ok(Some(v)) if `method` is a builtin on `receiver`, Ok(None) if not a builtin.
    /// Built-in free functions (io, env, conversions). Returns `Ok(None)`
    /// when the name isn't a builtin so the caller can fall through to user-defined fns.
    fn call_builtin_free(
        &mut self,
        name: &str,
        args: &[Arg],
        call_span: Span,
    ) -> Result<Option<Value>, LingoError> {
        // helper: enforce all-positional and eval args
        let positional = |this: &mut Interp| -> Result<Vec<Value>, LingoError> {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                if a.name.is_some() {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("builtin `{}` takes positional args only", name),
                        a.span,
                    ));
                }
                out.push(this.eval(&a.value)?);
                if this.pending_raise.is_some() {
                    return Ok(out);
                }
            }
            Ok(out)
        };

        match name {
            "read_file" => {
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if vals.len() != 1 {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`read_file(path)` expects 1 arg, got {}", vals.len()), call_span));
                }
                let path = match &vals[0] {
                    Value::Str(s) => s.clone(),
                    v => return Err(LingoError::new(Stage::Runtime,
                        format!("`read_file` expects str, got {}", v.type_name()), call_span)),
                };
                let result = match std::fs::read_to_string(&path) {
                    Ok(s) => Ok(Value::Str(s)),
                    Err(e) => Err(Value::Str(format!("read_file({}): {}", path, e))),
                };
                Ok(Some(Value::Result_(Rc::new(result))))
            }
            "write_file" => {
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if vals.len() != 2 {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`write_file(path, contents)` expects 2 args, got {}", vals.len()), call_span));
                }
                let path = match &vals[0] {
                    Value::Str(s) => s.clone(),
                    v => return Err(LingoError::new(Stage::Runtime,
                        format!("`write_file` path must be str, got {}", v.type_name()), call_span)),
                };
                let contents = match &vals[1] {
                    Value::Str(s) => s.clone(),
                    v => return Err(LingoError::new(Stage::Runtime,
                        format!("`write_file` contents must be str, got {}", v.type_name()), call_span)),
                };
                let result: std::result::Result<Value, Value> = match std::fs::write(&path, contents) {
                    Ok(()) => Ok(Value::None_),
                    Err(e) => Err(Value::Str(format!("write_file({}): {}", path, e))),
                };
                Ok(Some(Value::Result_(Rc::new(result))))
            }
            "args" => {
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if !vals.is_empty() {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`args()` takes no arguments, got {}", vals.len()), call_span));
                }
                let v: Vec<Value> = self.argv.iter().map(|s| Value::Str(s.clone())).collect();
                Ok(Some(Value::Vec_(Rc::new(RefCell::new(v)))))
            }
            "int" => {
                // int(str) -> int ! str   — parse a base-10 integer
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if vals.len() != 1 {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`int(x)` expects 1 arg, got {}", vals.len()), call_span));
                }
                let result: std::result::Result<Value, Value> = match &vals[0] {
                    Value::Int(n) => Ok(Value::Int(*n)),
                    Value::Float(f) => Ok(Value::Int(*f as i64)),
                    Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                    Value::Str(s) => match s.trim().parse::<i64>() {
                        Ok(n) => Ok(Value::Int(n)),
                        Err(_) => Err(Value::Str(format!("int: can't parse {:?}", s))),
                    },
                    v => Err(Value::Str(format!("int: can't convert {}", v.type_name()))),
                };
                Ok(Some(Value::Result_(Rc::new(result))))
            }
            "float" => {
                // v0.2.4: float(str) -> float ! str   — parse an f64
                //
                // Shape mirrors `int(x)` (v0.2.0): identity / int->float /
                // bool->float / str->parse.  `s.trim().parse::<f64>()`
                // accepts the same grammar rust accepts (`1.5`, `1e9`,
                // `-3`, `inf`, `nan`...).  Error string is byte-identical
                // to the C backend's `lingo_float_parse` failure message:
                //   `float: can't parse "<rust-debug-repr>"`
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if vals.len() != 1 {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`float(x)` expects 1 arg, got {}", vals.len()), call_span));
                }
                let result: std::result::Result<Value, Value> = match &vals[0] {
                    Value::Float(f) => Ok(Value::Float(*f)),
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
                    Value::Str(s) => match s.trim().parse::<f64>() {
                        Ok(f) => Ok(Value::Float(f)),
                        Err(_) => Err(Value::Str(format!("float: can't parse {:?}", s))),
                    },
                    v => Err(Value::Str(format!("float: can't convert {}", v.type_name()))),
                };
                Ok(Some(Value::Result_(Rc::new(result))))
            }
            "str" => {
                // str(x) -> str   — convert any value to its display form
                let vals = positional(self)?;
                if self.pending_raise.is_some() { return Ok(Some(Value::None_)); }
                if vals.len() != 1 {
                    return Err(LingoError::new(Stage::Runtime,
                        format!("`str(x)` expects 1 arg, got {}", vals.len()), call_span));
                }
                Ok(Some(Value::Str(vals[0].display())))
            }
            _ => Ok(None),
        }
    }

    fn call_builtin_method(
        &mut self,
        receiver: &Value,
        method: &str,
        args: &[Arg],
        call_span: Span,
    ) -> Result<Option<Value>, LingoError> {
        // helper: eval all args, reject keyword args
        let eval_positional = |this: &mut Self, args: &[Arg]| -> Result<Vec<Value>, LingoError> {
            let mut out = Vec::new();
            for a in args {
                if a.name.is_some() {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        "builtin methods don't take keyword arguments",
                        a.span,
                    ));
                }
                out.push(this.eval(&a.value)?);
            }
            Ok(out)
        };
        match receiver {
            Value::Vec_(rc) => {
                let vals = eval_positional(self, args)?;
                match (method, vals.len()) {
                    ("len", 0) => Ok(Some(Value::Int(rc.borrow().len() as i64))),
                    ("push", 1) => {
                        rc.borrow_mut().push(vals.into_iter().next().unwrap());
                        Ok(Some(Value::None_))
                    }
                    ("pop", 0) => {
                        let popped = rc.borrow_mut().pop();
                        Ok(Some(match popped {
                            Some(v) => v,
                            None => Value::None_,
                        }))
                    }
                    ("get", 1) => {
                        let idx = match &vals[0] {
                            Value::Int(n) => *n,
                            v => return Err(LingoError::new(Stage::Runtime,
                                format!("vec.get expects int, got {}", v.type_name()), call_span)),
                        };
                        let borrow = rc.borrow();
                        let len = borrow.len() as i64;
                        if idx < 0 || idx >= len {
                            return Err(LingoError::new(Stage::Runtime,
                                format!("vec.get index {} out of bounds (len {})", idx, len), call_span));
                        }
                        Ok(Some(borrow[idx as usize].clone()))
                    }
                    ("set", 2) => {
                        let idx = match &vals[0] {
                            Value::Int(n) => *n,
                            v => return Err(LingoError::new(Stage::Runtime,
                                format!("vec.set expects int index, got {}", v.type_name()), call_span)),
                        };
                        let mut borrow = rc.borrow_mut();
                        let len = borrow.len() as i64;
                        if idx < 0 || idx >= len {
                            return Err(LingoError::new(Stage::Runtime,
                                format!("vec.set index {} out of bounds (len {})", idx, len), call_span));
                        }
                        borrow[idx as usize] = vals.into_iter().nth(1).unwrap();
                        Ok(Some(Value::None_))
                    }
                    ("contains", 1) => {
                        let needle = &vals[0];
                        let found = rc.borrow().iter().any(|v| values_eq(v, needle));
                        Ok(Some(Value::Bool(found)))
                    }
                    ("clear", 0) => {
                        rc.borrow_mut().clear();
                        Ok(Some(Value::None_))
                    }
                    ("reverse", 0) => {
                        rc.borrow_mut().reverse();
                        Ok(Some(Value::None_))
                    }
                    (m, n) => Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `vec.{}` with {} arg(s) (known: len/push/pop/get/set/contains/clear/reverse)", m, n),
                        call_span,
                    )),
                }
            }
            Value::Str(s) => {
                let vals = eval_positional(self, args)?;
                let need_str = |v: &Value, n: &str| -> Result<String, LingoError> {
                    if let Value::Str(s) = v { Ok(s.clone()) } else {
                        Err(LingoError::new(Stage::Runtime,
                            format!("str.{} expects str, got {}", n, v.type_name()), call_span))
                    }
                };
                match (method, vals.len()) {
                    ("len", 0) => Ok(Some(Value::Int(s.chars().count() as i64))),
                    ("contains", 1) => {
                        let needle = need_str(&vals[0], "contains")?;
                        Ok(Some(Value::Bool(s.contains(&needle))))
                    }
                    ("starts_with", 1) => {
                        let n = need_str(&vals[0], "starts_with")?;
                        Ok(Some(Value::Bool(s.starts_with(&n))))
                    }
                    ("ends_with", 1) => {
                        let n = need_str(&vals[0], "ends_with")?;
                        Ok(Some(Value::Bool(s.ends_with(&n))))
                    }
                    ("to_lower", 0) => Ok(Some(Value::Str(s.to_lowercase()))),
                    ("to_upper", 0) => Ok(Some(Value::Str(s.to_uppercase()))),
                    ("trim", 0) => Ok(Some(Value::Str(s.trim().to_string()))),
                    ("split", 1) => {
                        let sep = need_str(&vals[0], "split")?;
                        let parts: Vec<Value> = if sep.is_empty() {
                            s.chars().map(|c| Value::Str(c.to_string())).collect()
                        } else {
                            s.split(&sep).map(|p| Value::Str(p.to_string())).collect()
                        };
                        Ok(Some(Value::Vec_(Rc::new(RefCell::new(parts)))))
                    }
                    ("replace", 2) => {
                        let from = need_str(&vals[0], "replace")?;
                        let to = need_str(&vals[1], "replace")?;
                        Ok(Some(Value::Str(s.replace(&from, &to))))
                    }
                    (m, n) => Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `str.{}` with {} arg(s) (known: len/contains/starts_with/ends_with/to_lower/to_upper/trim/split/replace)", m, n),
                        call_span,
                    )),
                }
            }
            Value::Map_(rc) => {
                let vals = eval_positional(self, args)?;
                match (method, vals.len()) {
                    ("len", 0) => Ok(Some(Value::Int(rc.borrow().len() as i64))),
                    ("has", 1) => {
                        let key = &vals[0];
                        let found = rc.borrow().iter().any(|(k, _)| values_eq(k, key));
                        Ok(Some(Value::Bool(found)))
                    }
                    ("get", 1) => {
                        // v0.2.1: `map.get(k)` now returns `Opt[V]` instead
                        // of the duck-typed "raw V or None_" of v0.1.x.
                        // Pattern-match with `some(v) / none` to discriminate;
                        // `print(counts.get(k))` keeps the same wire format
                        // because Opt's display equals the inner value's
                        // display (or "none" for absent).
                        let key = &vals[0];
                        let borrow = rc.borrow();
                        for (k, v) in borrow.iter() {
                            if values_eq(k, key) {
                                return Ok(Some(Value::Opt(Some(Box::new(v.clone())))));
                            }
                        }
                        Ok(Some(Value::Opt(None)))
                    }
                    ("set", 2) => {
                        let mut it = vals.into_iter();
                        let key = it.next().unwrap();
                        let val = it.next().unwrap();
                        let mut borrow = rc.borrow_mut();
                        for slot in borrow.iter_mut() {
                            if values_eq(&slot.0, &key) {
                                slot.1 = val;
                                return Ok(Some(Value::None_));
                            }
                        }
                        borrow.push((key, val));
                        Ok(Some(Value::None_))
                    }
                    ("remove", 1) => {
                        let key = &vals[0];
                        let mut borrow = rc.borrow_mut();
                        if let Some(pos) = borrow.iter().position(|(k, _)| values_eq(k, key)) {
                            borrow.remove(pos);
                            Ok(Some(Value::Bool(true)))
                        } else {
                            Ok(Some(Value::Bool(false)))
                        }
                    }
                    ("keys", 0) => {
                        let ks: Vec<Value> = rc.borrow().iter().map(|(k, _)| k.clone()).collect();
                        Ok(Some(Value::Vec_(Rc::new(RefCell::new(ks)))))
                    }
                    ("values", 0) => {
                        let vs: Vec<Value> = rc.borrow().iter().map(|(_, v)| v.clone()).collect();
                        Ok(Some(Value::Vec_(Rc::new(RefCell::new(vs)))))
                    }
                    ("clear", 0) => {
                        rc.borrow_mut().clear();
                        Ok(Some(Value::None_))
                    }
                    (m, n) => Err(LingoError::new(
                        Stage::Runtime,
                        format!("no method `map.{}` with {} arg(s) (known: len/has/get/set/remove/keys/values/clear)", m, n),
                        call_span,
                    )),
                }
            }
            _ => Ok(None),
        }
    }

    fn resolve_args(
        &mut self,
        decl: &FnDecl,
        args: &[Arg],
        call_span: Span,
    ) -> Result<Vec<Value>, LingoError> {
        if decl.params.len() > 2 && args.iter().any(|a| a.name.is_none()) {
            return Err(LingoError::new(
                Stage::Runtime,
                format!(
                    "function `{}` has {} params; pass all as keyword args (`name: value`)",
                    decl.name,
                    decl.params.len()
                ),
                call_span,
            ));
        }
        let mut resolved: Vec<Option<Value>> = vec![None; decl.params.len()];
        let mut positional_idx = 0usize;
        let arg_count = args.len();
        for a in args {
            let v = self.eval(&a.value)?;
            if let Some(n) = &a.name {
                let idx = decl.params.iter().position(|p| &p.name == n).ok_or_else(|| {
                    LingoError::new(
                        Stage::Runtime,
                        format!("`{}` has no parameter `{}`", decl.name, n),
                        a.span,
                    )
                })?;
                if resolved[idx].is_some() {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("argument `{}` passed twice", n),
                        a.span,
                    ));
                }
                resolved[idx] = Some(v);
            } else {
                if positional_idx >= decl.params.len() {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("too many positional arguments to `{}`", decl.name),
                        a.span,
                    ));
                }
                resolved[positional_idx] = Some(v);
                positional_idx += 1;
            }
        }
        if let Some((i, _)) = resolved.iter().enumerate().find(|(_, v)| v.is_none()) {
            return Err(LingoError::new(
                Stage::Runtime,
                format!(
                    "missing argument `{}` in call to `{}` ({} param(s), {} given)",
                    decl.params[i].name, decl.name, decl.params.len(), arg_count
                ),
                call_span,
            ));
        }
        Ok(resolved.into_iter().map(|v| v.unwrap()).collect())
    }
}

fn bin_op(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, LingoError> {
    use BinOp::*;
    use Value::*;
    let promote = matches!(&l, Float(_)) || matches!(&r, Float(_));
    if promote {
        let lf = match &l {
            Int(n) => *n as f64,
            Float(f) => *f,
            _ => return type_err(op, &l, &r, span),
        };
        let rf = match &r {
            Int(n) => *n as f64,
            Float(f) => *f,
            _ => return type_err(op, &l, &r, span),
        };
        return Ok(match op {
            Add => Float(lf + rf),
            Sub => Float(lf - rf),
            Mul => Float(lf * rf),
            Div => Float(lf / rf),
            Mod => Float(lf % rf),
            Pow => Float(lf.powf(rf)),
            Eq => Bool(lf == rf),
            Ne => Bool(lf != rf),
            Lt => Bool(lf < rf),
            Le => Bool(lf <= rf),
            Gt => Bool(lf > rf),
            Ge => Bool(lf >= rf),
            And | Or => unreachable!(),
        });
    }
    Ok(match (op, l, r) {
        (Add, Int(a), Int(b)) => Int(a + b),
        (Sub, Int(a), Int(b)) => Int(a - b),
        (Mul, Int(a), Int(b)) => Int(a * b),
        (Div, Int(a), Int(b)) => {
            if b == 0 {
                return Err(LingoError::new(Stage::Runtime, "integer division by zero", span));
            }
            Int(a / b)
        }
        (Mod, Int(a), Int(b)) => {
            if b == 0 {
                return Err(LingoError::new(Stage::Runtime, "integer modulo by zero", span));
            }
            Int(a % b)
        }
        (Pow, Int(a), Int(b)) => {
            if b < 0 {
                Float((a as f64).powi(b as i32))
            } else {
                Int(a.pow(b as u32))
            }
        }
        (Eq, Int(a), Int(b)) => Bool(a == b),
        (Ne, Int(a), Int(b)) => Bool(a != b),
        (Lt, Int(a), Int(b)) => Bool(a < b),
        (Le, Int(a), Int(b)) => Bool(a <= b),
        (Gt, Int(a), Int(b)) => Bool(a > b),
        (Ge, Int(a), Int(b)) => Bool(a >= b),
        (Eq, Bool(a), Bool(b)) => Bool(a == b),
        (Ne, Bool(a), Bool(b)) => Bool(a != b),
        (Eq, Str(a), Str(b)) => Bool(a == b),
        (Ne, Str(a), Str(b)) => Bool(a != b),
        (Add, Str(a), Str(b)) => Str(a + &b),
        (op, l, r) => return type_err(op, &l, &r, span),
    })
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::None_, Value::None_) => true,
        (Value::Enum { type_name: ta, variant: va, payload: pa },
         Value::Enum { type_name: tb, variant: vb, payload: pb }) => {
            ta == tb && va == vb && pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| values_eq(x, y))
        }
        _ => false,
    }
}

fn type_err(op: BinOp, l: &Value, r: &Value, span: Span) -> Result<Value, LingoError> {
    Err(LingoError::new(
        Stage::Runtime,
        format!(
            "cannot apply {:?} to {} and {} (no implicit conversions)",
            op,
            l.type_name(),
            r.type_name()
        ),
        span,
    ))
}
