//! Tiny C backend — emits portable C99 from a *subset* of lingo.
//!
//! This is the first step toward the v0.2 LLVM backend.  The
//! generated C is meant to be fed straight to `gcc -O2 -std=c99`
//! and produces a native binary with zero runtime overhead.
//!
//! **Supported subset (v0.1.7)**
//!   - `fn` declarations with typed params + return
//!   - `let` / `let mut` / assignment
//!   - `if` / `elif` / `else`
//!   - `for x in start..end`  (counted loop, exclusive upper bound)
//!   - `return`
//!   - `print(...)` with int / bool / str args
//!   - int (i64 / u64 / int), bool, str literals
//!   - arithmetic, comparison, boolean ops
//!   - calls to user-defined functions (incl. recursion)
//!
//! **Not yet:** vec, map, structs, enums, traits, error types, `?`,
//! f-strings, match, io builtins, generics, allocators.
//!
//! When this file grows past ~1000 lines it should be split into
//! `lower.rs` (lingo AST → mid-IR) + `emit_c.rs` (mid-IR → C).

use std::collections::HashMap;
use std::fmt::Write;

use crate::ast::*;
use crate::error::{LingoError, Span, Stage};

/// The C type of an expression. Kept narrow on purpose — anything richer
/// belongs to the runtime story (vec/map/structs) which the C backend
/// won't tackle until after we have a real type checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CType {
    I64,
    U64,
    Bool,
    Str, // const char*
    Void,
}

impl CType {
    fn c_decl(self) -> &'static str {
        match self {
            CType::I64 => "int64_t",
            CType::U64 => "uint64_t",
            CType::Bool => "bool",
            CType::Str => "const char*",
            CType::Void => "void",
        }
    }

    /// printf format specifier for this type. We splice these in as C
    /// `PRId64`/`PRIu64` macros (from <inttypes.h>) so the format
    /// stays correct on both 32- and 64-bit platforms.
    fn printf_fmt(self) -> &'static str {
        match self {
            CType::I64 => "%\" PRId64 \"",
            CType::U64 => "%\" PRIu64 \"",
            CType::Bool => "%s", // we print "true"/"false"
            CType::Str => "%s",
            CType::Void => "",
        }
    }
}

fn map_type(t: &TypeRef, span: Span) -> Result<CType, LingoError> {
    if !t.type_args.is_empty() {
        return Err(LingoError::new(
            Stage::Resolve,
            format!("C backend: generic type `{}` is not supported yet", t.name),
            span,
        ));
    }
    Ok(match t.name.as_str() {
        "int" | "i64" => CType::I64,
        "u64" => CType::U64,
        "bool" => CType::Bool,
        "str" => CType::Str,
        other => {
            return Err(LingoError::new(
                Stage::Resolve,
                format!("C backend: type `{}` is not supported in v0.1.7", other),
                span,
            ));
        }
    })
}

pub struct Codegen {
    /// Accumulated function bodies (after the prelude).
    body: String,
    /// Forward-declared function prototypes (so call order doesn't matter).
    protos: String,
    /// Function signatures, looked up to type return values.
    fn_sigs: HashMap<String, (Vec<CType>, CType)>,
    /// Stack of local-scope variable types. Top frame is the active scope.
    scopes: Vec<HashMap<String, CType>>,
    /// How deep are we indented in the current C function body?
    indent: usize,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            body: String::new(),
            protos: String::new(),
            fn_sigs: HashMap::new(),
            scopes: Vec::new(),
            indent: 0,
        }
    }

    /// Compile a whole program to a self-contained C99 source file.
    pub fn gen_program(mut self, prog: &Program) -> Result<String, LingoError> {
        // Bail fast on anything the C backend can't handle.
        for item in &prog.items {
            match item {
                Item::Fn(_) | Item::Const(_) => {}
                _ => {
                    let span = match item {
                        Item::Struct(s) => s.span,
                        Item::Enum(e) => e.span,
                        Item::Impl(b) => b.span,
                        Item::Trait(t) => t.span,
                        Item::ImplTrait(b) => b.span,
                        Item::Fn(_) | Item::Const(_) => Span::dummy(),
                    };
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: only `fn` and `const` are supported in v0.1.7. \
                         structs, enums, impls, and traits land with the LLVM backend in v0.2.",
                        span,
                    ));
                }
            }
        }

        // First pass: collect signatures so calls can be type-checked.
        for item in &prog.items {
            if let Item::Fn(f) = item {
                let mut params = Vec::with_capacity(f.params.len());
                for p in &f.params {
                    params.push(map_type(&p.ty, p.span)?);
                }
                let ret = match &f.return_type {
                    Some(t) => map_type(t, f.span)?,
                    None => CType::Void,
                };
                if f.raises.is_some() {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: fallible fns (`! E`) need the v0.2 result lowering",
                        f.span,
                    ));
                }
                self.fn_sigs.insert(f.name.clone(), (params, ret));
            }
        }

        // Second pass: emit prototypes and bodies.
        for item in &prog.items {
            match item {
                Item::Fn(f) => {
                    let proto = self.fn_proto(f)?;
                    writeln!(self.protos, "{};", proto).unwrap();
                    self.emit_fn(f)?;
                }
                Item::Const(c) => {
                    let (code, ty) = self.gen_expr(&c.value)?;
                    writeln!(self.protos, "static {} {} = {};", ty.c_decl(), c.name, code).unwrap();
                    // make const visible to other fns (top-level scope)
                    self.scopes.last_mut().map(|s| s.insert(c.name.clone(), ty));
                }
                _ => unreachable!(),
            }
        }

        // Compose final source.
        let mut out = String::new();
        out.push_str("/* auto-generated by lingoc C backend — do not edit */\n");
        out.push_str("#include <stdio.h>\n");
        out.push_str("#include <stdint.h>\n");
        out.push_str("#include <inttypes.h>\n");
        out.push_str("#include <stdbool.h>\n");
        out.push_str("\n");
        out.push_str(&self.protos);
        out.push_str("\n");
        out.push_str(&self.body);
        Ok(out)
    }

    fn fn_proto(&self, f: &FnDecl) -> Result<String, LingoError> {
        let (params, ret) = self
            .fn_sigs
            .get(&f.name)
            .expect("signature must be registered before emit");
        let mut s = String::new();
        // C's `main` must return int — even though lingo's main returns nothing.
        let c_ret = if f.name == "main" { "int" } else { ret.c_decl() };
        write!(s, "{} {}(", c_ret, f.name).unwrap();
        if f.params.is_empty() {
            s.push_str("void");
        } else {
            for (i, p) in f.params.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                write!(s, "{} {}", params[i].c_decl(), p.name).unwrap();
            }
        }
        s.push(')');
        Ok(s)
    }

    fn emit_fn(&mut self, f: &FnDecl) -> Result<(), LingoError> {
        let proto = self.fn_proto(f)?;
        writeln!(self.body, "{} {{", proto).unwrap();
        self.indent = 1;
        self.scopes.push(HashMap::new());
        let (params, _) = self.fn_sigs.get(&f.name).cloned().unwrap();
        for (i, p) in f.params.iter().enumerate() {
            self.scopes.last_mut().unwrap().insert(p.name.clone(), params[i]);
        }
        for s in &f.body.stmts {
            self.emit_stmt(s)?;
        }
        // For `main`, always finish with `return 0;` (C requires an int return).
        if f.name == "main" {
            writeln!(self.body, "{}return 0;", self.pad()).unwrap();
        }
        self.scopes.pop();
        writeln!(self.body, "}}\n").unwrap();
        Ok(())
    }

    fn pad(&self) -> String {
        "    ".repeat(self.indent)
    }

    fn emit_stmt(&mut self, s: &Stmt) -> Result<(), LingoError> {
        match s {
            Stmt::Let { is_mut: _, name, ty, value, span } => {
                let (code, val_ty) = self.gen_expr(value)?;
                let decl_ty = match ty {
                    Some(t) => map_type(t, *span)?,
                    None => val_ty,
                };
                if decl_ty != val_ty && !(decl_ty == CType::I64 && val_ty == CType::U64)
                    && !(decl_ty == CType::U64 && val_ty == CType::I64)
                {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("C backend: let `{}` declared {:?} but rhs is {:?}",
                                name, decl_ty, val_ty),
                        *span,
                    ));
                }
                writeln!(self.body, "{}{} {} = {};", self.pad(), decl_ty.c_decl(), name, code).unwrap();
                self.scopes.last_mut().unwrap().insert(name.clone(), decl_ty);
            }
            Stmt::Assign { target, value, span } => {
                let name = match target {
                    AssignTarget::Name(n) => n.clone(),
                    AssignTarget::Field(_, _) => {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            "C backend: field assignment lands with structs in v0.2",
                            *span,
                        ));
                    }
                };
                let (code, _) = self.gen_expr(value)?;
                writeln!(self.body, "{}{} = {};", self.pad(), name, code).unwrap();
            }
            Stmt::Return { value, span: _ } => {
                if let Some(e) = value {
                    let (code, _) = self.gen_expr(e)?;
                    writeln!(self.body, "{}return {};", self.pad(), code).unwrap();
                } else {
                    writeln!(self.body, "{}return;", self.pad()).unwrap();
                }
            }
            Stmt::If { arms, else_block, span: _ } => {
                for (i, (cond, block)) in arms.iter().enumerate() {
                    let (code, _) = self.gen_expr(cond)?;
                    let kw = if i == 0 { "if" } else { "else if" };
                    writeln!(self.body, "{}{} ({}) {{", self.pad(), kw, code).unwrap();
                    self.indent += 1;
                    self.scopes.push(HashMap::new());
                    for s in &block.stmts {
                        self.emit_stmt(s)?;
                    }
                    self.scopes.pop();
                    self.indent -= 1;
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                }
                if let Some(b) = else_block {
                    writeln!(self.body, "{}else {{", self.pad()).unwrap();
                    self.indent += 1;
                    self.scopes.push(HashMap::new());
                    for s in &b.stmts {
                        self.emit_stmt(s)?;
                    }
                    self.scopes.pop();
                    self.indent -= 1;
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                }
            }
            Stmt::For { var, iter, body, span } => {
                // only `start..end` ranges are supported in v0.1.7
                let (lo, hi) = match &iter.kind {
                    ExprKind::Range(a, b) => (a.as_ref(), b.as_ref()),
                    _ => {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            "C backend: `for` only supports `start..end` ranges in v0.1.7",
                            *span,
                        ));
                    }
                };
                let (lo_code, _) = self.gen_expr(lo)?;
                let (hi_code, _) = self.gen_expr(hi)?;
                writeln!(self.body, "{}for (int64_t {var} = {lo_code}; {var} < {hi_code}; ++{var}) {{",
                         self.pad()).unwrap();
                self.indent += 1;
                self.scopes.push(HashMap::new());
                self.scopes.last_mut().unwrap().insert(var.clone(), CType::I64);
                for s in &body.stmts {
                    self.emit_stmt(s)?;
                }
                self.scopes.pop();
                self.indent -= 1;
                writeln!(self.body, "{}}}", self.pad()).unwrap();
            }
            Stmt::Break(_) => {
                writeln!(self.body, "{}break;", self.pad()).unwrap();
            }
            Stmt::Continue(_) => {
                writeln!(self.body, "{}continue;", self.pad()).unwrap();
            }
            Stmt::Expr(e) => {
                // a bare call (typically `print(...)`)
                if let ExprKind::Call(callee, args) = &e.kind {
                    if matches!(callee.kind, ExprKind::PrintBuiltin) {
                        self.emit_print(args, e.span)?;
                        return Ok(());
                    }
                }
                let (code, _) = self.gen_expr(e)?;
                writeln!(self.body, "{}{};", self.pad(), code).unwrap();
            }
            other => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    format!("C backend: unsupported statement {:?}", std::mem::discriminant(other)),
                    Span::dummy(),
                ));
            }
        }
        Ok(())
    }

    fn emit_print(&mut self, args: &[Arg], span: Span) -> Result<(), LingoError> {
        // Build a single printf("fmt", ...). Multiple args separated by spaces,
        // newline at end. Bool values are converted to "true"/"false" strings.
        let mut fmt = String::new();
        let mut vals: Vec<String> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if a.name.is_some() {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: `print` takes positional args only",
                    a.span,
                ));
            }
            if i > 0 {
                fmt.push(' ');
            }
            let (code, ty) = self.gen_expr(&a.value)?;
            fmt.push_str(ty.printf_fmt());
            match ty {
                CType::Bool => vals.push(format!("(({}) ? \"true\" : \"false\")", code)),
                _ => vals.push(code),
            }
        }
        fmt.push_str("\\n");
        let _ = span;
        if vals.is_empty() {
            writeln!(self.body, "{}printf(\"{}\");", self.pad(), fmt).unwrap();
        } else {
            writeln!(self.body, "{}printf(\"{}\", {});", self.pad(), fmt, vals.join(", ")).unwrap();
        }
        Ok(())
    }

    fn lookup_var(&self, name: &str) -> Option<CType> {
        for s in self.scopes.iter().rev() {
            if let Some(t) = s.get(name) {
                return Some(*t);
            }
        }
        None
    }

    fn gen_expr(&mut self, e: &Expr) -> Result<(String, CType), LingoError> {
        Ok(match &e.kind {
            ExprKind::Int(n) => (format!("((int64_t){}LL)", n), CType::I64),
            ExprKind::Bool(b) => ((if *b { "true" } else { "false" }).to_string(), CType::Bool),
            ExprKind::Str(s) => (format!("\"{}\"", escape_c(s)), CType::Str),
            ExprKind::Float(_) => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: floats land in v0.2",
                    e.span,
                ));
            }
            ExprKind::None_ => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: `none` lands in v0.2 with options",
                    e.span,
                ));
            }
            ExprKind::Ident(name) => {
                let ty = self.lookup_var(name).ok_or_else(|| {
                    LingoError::new(
                        Stage::Resolve,
                        format!("C backend: `{}` is not in scope", name),
                        e.span,
                    )
                })?;
                (name.clone(), ty)
            }
            ExprKind::Unary(op, x) => {
                let (code, ty) = self.gen_expr(x)?;
                match op {
                    UnOp::Neg => (format!("(-{})", code), ty),
                    UnOp::Not => (format!("(!{})", code), CType::Bool),
                }
            }
            ExprKind::Binary(op, a, b) => self.gen_binop(*op, a, b)?,
            ExprKind::Call(callee, args) => self.gen_call(callee, args, e.span)?,
            ExprKind::Range(_, _) => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: ranges only appear inside `for` headers",
                    e.span,
                ));
            }
            other => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    format!("C backend: unsupported expression {:?}",
                            std::mem::discriminant(other)),
                    e.span,
                ));
            }
        })
    }

    fn gen_binop(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Result<(String, CType), LingoError> {
        let (a_code, a_ty) = self.gen_expr(a)?;
        let (b_code, b_ty) = self.gen_expr(b)?;
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let ty = if a_ty == CType::U64 || b_ty == CType::U64 {
                    CType::U64
                } else {
                    CType::I64
                };
                let sym = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    _ => unreachable!(),
                };
                Ok((format!("({} {} {})", a_code, sym, b_code), ty))
            }
            BinOp::Pow => {
                // integer pow — keep it simple, generate a runtime helper inline
                // 2 ** n with i64 / u64. (we punt on overflow; that's a v0.2 problem.)
                let ty = if a_ty == CType::U64 || b_ty == CType::U64 {
                    CType::U64
                } else {
                    CType::I64
                };
                // emit a one-shot helper using a comma expression + statement expression
                // would be GCC-only; safer to require pow to be lifted later.
                // For now, use a fixed __builtin call via repeated multiplication helper.
                Ok((format!("lingo_ipow({}, {})", a_code, b_code), ty))
            }
            BinOp::Eq => Ok((format!("({} == {})", a_code, b_code), CType::Bool)),
            BinOp::Ne => Ok((format!("({} != {})", a_code, b_code), CType::Bool)),
            BinOp::Lt => Ok((format!("({} <  {})", a_code, b_code), CType::Bool)),
            BinOp::Le => Ok((format!("({} <= {})", a_code, b_code), CType::Bool)),
            BinOp::Gt => Ok((format!("({} >  {})", a_code, b_code), CType::Bool)),
            BinOp::Ge => Ok((format!("({} >= {})", a_code, b_code), CType::Bool)),
            BinOp::And => Ok((format!("({} && {})", a_code, b_code), CType::Bool)),
            BinOp::Or => Ok((format!("({} || {})", a_code, b_code), CType::Bool)),
        }
    }

    fn gen_call(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<(String, CType), LingoError> {
        let name = match &callee.kind {
            ExprKind::Ident(s) => s.clone(),
            _ => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: only named function calls are supported in v0.1.7",
                    span,
                ));
            }
        };
        let (param_tys, ret) = self.fn_sigs.get(&name).cloned().ok_or_else(|| {
            LingoError::new(
                Stage::Resolve,
                format!("C backend: function `{}` is not defined", name),
                span,
            )
        })?;
        if args.len() != param_tys.len() {
            return Err(LingoError::new(
                Stage::Resolve,
                format!("`{}` expects {} args, got {}", name, param_tys.len(), args.len()),
                span,
            ));
        }
        let mut parts = Vec::with_capacity(args.len());
        for a in args {
            if a.name.is_some() {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: keyword args land in v0.2",
                    a.span,
                ));
            }
            let (code, _) = self.gen_expr(&a.value)?;
            parts.push(code);
        }
        Ok((format!("{}({})", name, parts.join(", ")), ret))
    }
}

fn escape_c(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Build a full C source file from a lingo program.
/// The result already includes the prelude (`#include`s) plus a tiny
/// `lingo_ipow` runtime helper used by integer `**`.
pub fn emit(prog: &Program) -> Result<String, LingoError> {
    let core = Codegen::new().gen_program(prog)?;
    // splice the helper just after the includes
    let helper = "\
__attribute__((unused))
static int64_t lingo_ipow(int64_t base, int64_t exp) {
    int64_t r = 1;
    if (exp < 0) return 0;
    while (exp > 0) {
        if (exp & 1) r *= base;
        base *= base;
        exp >>= 1;
    }
    return r;
}

";
    // Insert helper right before the protos section.
    // (Protos always start after the three #include lines + blank.)
    let marker = "#include <stdbool.h>\n\n";
    Ok(match core.find(marker) {
        Some(idx) => {
            let split = idx + marker.len();
            let mut s = String::with_capacity(core.len() + helper.len());
            s.push_str(&core[..split]);
            s.push_str(helper);
            s.push_str(&core[split..]);
            s
        }
        None => format!("{}{}", helper, core),
    })
}
