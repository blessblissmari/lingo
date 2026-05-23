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

use std::collections::HashMap;

use crate::ast::*;
use crate::error::{LingoError, Span, Stage};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Range(i64, i64),
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
}

#[derive(Debug)]
enum Flow {
    Normal,
    Return(Value),
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
        match flow? {
            Flow::Return(v) => Ok(v),
            Flow::Normal => Ok(Value::None_),
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

    fn exec_stmt(&mut self, s: &Stmt) -> Result<Flow, LingoError> {
        match s {
            Stmt::Let { is_mut, name, ty: _, value, span } => {
                let v = self.eval(value)?;
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
                let v = self.eval(value)?;
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
                    Some(e) => self.eval(e)?,
                    None => Value::None_,
                };
                Ok(Flow::Return(v))
            }
            Stmt::If { arms, else_block, .. } => {
                for (cond, block) in arms {
                    let c = self.eval(cond)?;
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
                let it = self.eval(iter)?;
                let (lo, hi) = match it {
                    Value::Range(a, b) => (a, b),
                    v => {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!("`for` needs a range, got {}", v.type_name()),
                            iter.span,
                        ))
                    }
                };
                for n in lo..hi {
                    self.scopes.push(Scope { bindings: HashMap::new() });
                    self.scopes.last_mut().unwrap().bindings.insert(
                        var.clone(),
                        Binding { value: Value::Int(n), is_mut: false },
                    );
                    let flow = self.exec_block_inline(body)?;
                    self.scopes.pop();
                    match flow {
                        Flow::Break => break,
                        Flow::Continue | Flow::Normal => continue,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Match { scrutinee, arms, span } => {
                let v = self.eval(scrutinee)?;
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
            let type_name = match &receiver {
                Value::Struct { type_name, .. } => type_name.clone(),
                Value::Enum { type_name, .. } => type_name.clone(),
                v => {
                    return Err(LingoError::new(
                        Stage::Runtime,
                        format!("cannot call methods on {}", v.type_name()),
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
