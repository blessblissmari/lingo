//! Tree-walking interpreter for lingo v0.1.
//!
//! Scope rules:
//!   - lexical scopes, no closures (yet).
//!   - `let` introduces a new binding.  shadowing in the same scope is a
//!     hard error at runtime (until the resolver moves it earlier).
//!   - `let mut` allows later assignment via `name = expr`.
//!
//! Control flow uses a small `Flow` enum threaded through statement
//! evaluation.  Errors are surfaced as `LingoError` with `Stage::Runtime`.

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
    None_,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Str(_) => "str",
            Value::Range(_, _) => "range",
            Value::None_ => "none",
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Str(s) => s.clone(),
            Value::Range(a, b) => format!("{a}..{b}"),
            Value::None_ => "none".to_string(),
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
            scopes: Vec::new(),
        }
    }

    pub fn run_program(&mut self, prog: &Program) -> Result<Value, LingoError> {
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
        // each function call gets its own scope stack
        let saved = std::mem::take(&mut self.scopes);
        self.scopes.push(Scope { bindings: HashMap::new() });
        for (p, v) in decl.params.iter().zip(args.into_iter()) {
            self.scopes.last_mut().unwrap().bindings.insert(
                p.name.clone(),
                Binding { value: v, is_mut: false },
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
                // no shadowing
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
                for scope in self.scopes.iter_mut().rev() {
                    if let Some(b) = scope.bindings.get_mut(target) {
                        if !b.is_mut {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                format!("cannot assign to immutable `{}` (declare with `let mut`)", target),
                                *span,
                            ));
                        }
                        b.value = v;
                        return Ok(Flow::Normal);
                    }
                }
                Err(LingoError::new(
                    Stage::Runtime,
                    format!("`{}` is not defined", target),
                    *span,
                ))
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
            Stmt::For { var, iter, body, span } => {
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
                let _ = span;
                Ok(Flow::Normal)
            }
            Stmt::Break(_) => Ok(Flow::Break),
            Stmt::Continue(_) => Ok(Flow::Continue),
            Stmt::Expr(e) => {
                self.eval(e)?;
                Ok(Flow::Normal)
            }
        }
    }

    // exec a block without creating a new scope (caller already pushed one)
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
                // short-circuit and / or
                match op {
                    BinOp::And => {
                        let lv = self.eval(l)?;
                        match lv {
                            Value::Bool(false) => return Ok(Value::Bool(false)),
                            Value::Bool(true) => {
                                let rv = self.eval(r)?;
                                if let Value::Bool(b) = rv {
                                    return Ok(Value::Bool(b));
                                }
                                return Err(LingoError::new(
                                    Stage::Runtime,
                                    "`and` requires bool on both sides",
                                    r.span,
                                ));
                            }
                            v => {
                                return Err(LingoError::new(
                                    Stage::Runtime,
                                    format!("`and` requires bool, got {}", v.type_name()),
                                    l.span,
                                ))
                            }
                        }
                    }
                    BinOp::Or => {
                        let lv = self.eval(l)?;
                        match lv {
                            Value::Bool(true) => return Ok(Value::Bool(true)),
                            Value::Bool(false) => {
                                let rv = self.eval(r)?;
                                if let Value::Bool(b) = rv {
                                    return Ok(Value::Bool(b));
                                }
                                return Err(LingoError::new(
                                    Stage::Runtime,
                                    "`or` requires bool on both sides",
                                    r.span,
                                ));
                            }
                            v => {
                                return Err(LingoError::new(
                                    Stage::Runtime,
                                    format!("`or` requires bool, got {}", v.type_name()),
                                    l.span,
                                ))
                            }
                        }
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
            ExprKind::Call(callee, args) => {
                // evaluate args
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
                // resolve args: support positional and keyword
                let mut resolved: Vec<Option<Value>> = vec![None; decl.params.len()];
                let mut positional_idx = 0usize;
                let arg_count = args.len();
                // keyword-required-when->2 rule:
                if decl.params.len() > 2 {
                    if args.iter().any(|a| a.name.is_none()) {
                        return Err(LingoError::new(
                            Stage::Runtime,
                            format!(
                                "function `{}` has {} params; pass all as keyword args (`name: value`)",
                                name,
                                decl.params.len()
                            ),
                            e.span,
                        ));
                    }
                }
                for a in args {
                    let v = self.eval(&a.value)?;
                    if let Some(n) = &a.name {
                        let idx = decl.params.iter().position(|p| &p.name == n).ok_or_else(|| {
                            LingoError::new(
                                Stage::Runtime,
                                format!("`{}` has no parameter `{}`", name, n),
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
                                format!("too many positional arguments to `{}`", name),
                                a.span,
                            ));
                        }
                        if resolved[positional_idx].is_some() {
                            return Err(LingoError::new(
                                Stage::Runtime,
                                format!(
                                    "positional argument collides with a keyword for `{}`",
                                    decl.params[positional_idx].name
                                ),
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
                            decl.params[i].name, name, decl.params.len(), arg_count
                        ),
                        e.span,
                    ));
                }
                let values: Vec<Value> = resolved.into_iter().map(|v| v.unwrap()).collect();
                self.call_fn(&decl, values)
            }
        }
    }
}

fn bin_op(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, LingoError> {
    use BinOp::*;
    use Value::*;
    // promote int->float when one side is float
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
