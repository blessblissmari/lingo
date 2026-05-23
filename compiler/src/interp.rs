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
        fields: HashMap<String, Value>,
    },
    Enum {
        type_name: String,
        variant: String,
        payload: Vec<Value>,
    },
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
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display()))
                    .collect();
                parts.sort();
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
    scopes: Vec<Scope>,
    /// Set by `?` when the inner result is Err. The next statement boundary
    /// converts this into a `Flow::Raise(...)` so it propagates to the caller.
    pending_raise: Option<Value>,
}

#[derive(Debug)]
enum Flow {
    Normal,
    Return(Value),
    Raise(Value),
    Break,
    Continue,
}

impl Interp {
    pub fn new() -> Self {
        Self {
            fns: HashMap::new(),
            consts: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            methods: HashMap::new(),
            scopes: Vec::new(),
            pending_raise: None,
        }
    }

    pub fn run_program(&mut self, prog: &Program) -> Result<Value, LingoError> {
        // first pass: register types and signatures
        for item in &prog.items {
            match item {
                Item::Fn(f) => {
                    if self.fns.insert(f.name.clone(), f.clone()).is_some() {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate function `{}`", f.name),
                            f.span,
                        ));
                    }
                }
                Item::Struct(s) => {
                    if self.structs.insert(s.name.clone(), s.clone()).is_some() {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate struct `{}`", s.name),
                            s.span,
                        ));
                    }
                }
                Item::Enum(e) => {
                    if self.enums.insert(e.name.clone(), e.clone()).is_some() {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate enum `{}`", e.name),
                            e.span,
                        ));
                    }
                }
                Item::Impl(_) | Item::Const(_) => {}
            }
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
                        if entry.insert(m.name.clone(), m.clone()).is_some() {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("duplicate method `{}.{}`", b.target, m.name),
                                m.span,
                            ));
                        }
                    }
                }
                Item::Const(c) => {
                    let v = self.eval_const(&c.value)?;
                    if self.consts.insert(c.name.clone(), v).is_some() {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("duplicate const `{}`", c.name),
                            c.span,
                        ));
                    }
                }
                _ => {}
            }
        }

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
        for (p, v) in decl.params.iter().zip(args.into_iter()) {
            self.scopes.last_mut().unwrap().bindings.insert(
                p.name.clone(),
                Binding { value: v, is_mut: p.name == "self" }, // self.field = ... allowed
            );
        }
        let flow = self.exec_block(&decl.body);
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
                                    if let Some(slot) = fields.get_mut(fname) {
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
                // bind never fails; introduces a binding
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
                Ok(Value::Struct {
                    type_name: name.clone(),
                    fields: map,
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
                    Value::Struct { fields, .. } => fields.get(name).cloned().ok_or_else(|| {
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
            ExprKind::Try(inner) => {
                let v = self.eval(inner)?;
                if self.pending_raise.is_some() {
                    return Ok(Value::None_);
                }
                match v {
                    Value::Result_(rc) => match rc.as_ref() {
                        Ok(val) => Ok(val.clone()),
                        Err(err) => {
                            self.pending_raise = Some(err.clone());
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
            let decl = self
                .methods
                .get(&type_name)
                .and_then(|m| m.get(vname))
                .cloned()
                .ok_or_else(|| {
                    LingoError::new(
                        Stage::Runtime,
                        format!("no method `{}` on `{}`", vname, type_name),
                        callee.span,
                    )
                })?;
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
                        let key = &vals[0];
                        let borrow = rc.borrow();
                        for (k, v) in borrow.iter() {
                            if values_eq(k, key) {
                                return Ok(Some(v.clone()));
                            }
                        }
                        Ok(Some(Value::None_))
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
