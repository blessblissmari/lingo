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
#[derive(Debug, Clone, PartialEq, Eq)]
enum CType {
    I64,
    U64,
    Bool,
    Str, // const char*
    Void,
    /// User-defined struct type. The `String` is its lingo name, which is
    /// also the typedef'd C name (no mangling needed — lingo names are
    /// already valid C identifiers).
    Struct(String),
}

impl CType {
    fn c_decl(&self) -> String {
        match self {
            CType::I64 => "int64_t".into(),
            CType::U64 => "uint64_t".into(),
            CType::Bool => "bool".into(),
            CType::Str => "const char*".into(),
            CType::Void => "void".into(),
            CType::Struct(name) => name.clone(),
        }
    }

    /// printf format specifier for this type. We splice these in as C
    /// `PRId64`/`PRIu64` macros (from <inttypes.h>) so the format
    /// stays correct on both 32- and 64-bit platforms.
    fn printf_fmt(&self) -> &'static str {
        match self {
            CType::I64 => "%\" PRId64 \"",
            CType::U64 => "%\" PRIu64 \"",
            CType::Bool => "%s", // we print "true"/"false"
            CType::Str => "%s",
            CType::Void => "",
            CType::Struct(_) => "<struct>", // not directly printable yet
        }
    }
}

/// Lower a lingo type reference to a `CType`.
/// `known_structs` lets us recognize user-defined struct names; we keep it
/// optional so call sites that only care about primitives can pass `None`.
fn map_type_with(
    t: &TypeRef,
    span: Span,
    known_structs: Option<&HashMap<String, Vec<(String, CType)>>>,
) -> Result<CType, LingoError> {
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
            if let Some(map) = known_structs {
                if map.contains_key(other) {
                    return Ok(CType::Struct(other.to_string()));
                }
            }
            return Err(LingoError::new(
                Stage::Resolve,
                format!("C backend: type `{}` is not supported yet", other),
                span,
            ));
        }
    })
}

fn map_type(t: &TypeRef, span: Span) -> Result<CType, LingoError> {
    map_type_with(t, span, None)
}

pub struct Codegen {
    /// Accumulated function bodies (after the prelude).
    body: String,
    /// Forward-declared function prototypes (so call order doesn't matter).
    protos: String,
    /// Typedef'd struct definitions (top of the file, before protos).
    type_defs: String,
    /// Function signatures, looked up to type return values.
    /// For impl methods we register `Type_method` with `self` as the first param.
    fn_sigs: HashMap<String, (Vec<CType>, CType)>,
    /// `struct_name -> [(field_name, field_type)]`, in declared order.
    structs: HashMap<String, Vec<(String, CType)>>,
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
            type_defs: String::new(),
            fn_sigs: HashMap::new(),
            structs: HashMap::new(),
            scopes: Vec::new(),
            indent: 0,
        }
    }

    /// Compile a whole program to a self-contained C99 source file.
    pub fn gen_program(mut self, prog: &Program) -> Result<String, LingoError> {
        // Bail fast on anything the C backend can't handle.
        for item in &prog.items {
            match item {
                Item::Fn(_) | Item::Const(_) | Item::Struct(_) | Item::Impl(_) => {}
                _ => {
                    let span = match item {
                        Item::Enum(e) => e.span,
                        Item::Trait(t) => t.span,
                        Item::ImplTrait(b) => b.span,
                        _ => Span::dummy(),
                    };
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: `enum`, `trait`, and `impl Trait for Type` need \
                         v0.2 lowering (tagged unions + vtables). \
                         Use the interpreter for now.",
                        span,
                    ));
                }
            }
        }

        // Pass 1a: register struct shapes (forward declaration of fields).
        // We do this *first* so subsequent passes can recognize struct types.
        for item in &prog.items {
            if let Item::Struct(s) = item {
                let mut fields = Vec::with_capacity(s.fields.len());
                for f in &s.fields {
                    // field types may reference *other* structs — fine because we
                    // collect names first, then types in a second sub-pass.
                    fields.push((f.name.clone(), CType::Void)); // placeholder
                }
                self.structs.insert(s.name.clone(), fields);
            }
        }
        // Pass 1b: now resolve field types (every struct name is known).
        for item in &prog.items {
            if let Item::Struct(s) = item {
                let mut fields = Vec::with_capacity(s.fields.len());
                for f in &s.fields {
                    let ty = map_type_with(&f.ty, f.span, Some(&self.structs))?;
                    fields.push((f.name.clone(), ty));
                }
                self.structs.insert(s.name.clone(), fields);
            }
        }
        // Pass 1c: emit `typedef struct { ... } Name;` for each struct.
        for item in &prog.items {
            if let Item::Struct(s) = item {
                writeln!(self.type_defs, "typedef struct {} {{", s.name).unwrap();
                for (fname, fty) in &self.structs[&s.name] {
                    writeln!(self.type_defs, "    {} {};", fty.c_decl(), fname).unwrap();
                }
                writeln!(self.type_defs, "}} {};\n", s.name).unwrap();
            }
        }

        // Pass 2: collect function signatures (free + impl methods).
        for item in &prog.items {
            match item {
                Item::Fn(f) => self.register_fn_sig(f, None)?,
                Item::Impl(blk) => {
                    if !self.structs.contains_key(&blk.target) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("C backend: impl target `{}` must be a struct", blk.target),
                            blk.span,
                        ));
                    }
                    for m in &blk.methods {
                        self.register_fn_sig(m, Some(&blk.target))?;
                    }
                }
                _ => {}
            }
        }

        // Pass 3: emit prototypes and bodies.
        for item in &prog.items {
            match item {
                Item::Fn(f) => {
                    let proto = self.fn_proto(&f.name, f)?;
                    writeln!(self.protos, "{};", proto).unwrap();
                    self.emit_fn_body(&f.name, f, None)?;
                }
                Item::Impl(blk) => {
                    for m in &blk.methods {
                        let mangled = format!("{}_{}", blk.target, m.name);
                        let proto = self.fn_proto(&mangled, m)?;
                        writeln!(self.protos, "{};", proto).unwrap();
                        self.emit_fn_body(&mangled, m, Some(&blk.target))?;
                    }
                }
                Item::Const(c) => {
                    let (code, ty) = self.gen_expr(&c.value)?;
                    writeln!(self.protos, "static {} {} = {};", ty.c_decl(), c.name, code).unwrap();
                    // top-level const becomes visible to scoped lookups
                    self.scopes.last_mut().map(|s| s.insert(c.name.clone(), ty));
                }
                Item::Struct(_) => {} // already emitted in pass 1c
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
        if !self.type_defs.is_empty() {
            out.push_str(&self.type_defs);
        }
        out.push_str(&self.protos);
        out.push_str("\n");
        out.push_str(&self.body);
        Ok(out)
    }

    /// Register a function signature in `self.fn_sigs`.
    /// `impl_target` is `Some(struct_name)` if this is an `impl Type:` method,
    /// in which case the mangled name `<Type>_<fn>` is used and `self: Self` is
    /// substituted to `self: Type`.
    fn register_fn_sig(&mut self, f: &FnDecl, impl_target: Option<&str>) -> Result<(), LingoError> {
        if f.raises.is_some() {
            return Err(LingoError::new(
                Stage::Resolve,
                "C backend: fallible fns (`! E`) need the v0.2 result lowering",
                f.span,
            ));
        }
        let mut params = Vec::with_capacity(f.params.len());
        for p in &f.params {
            if p.name == "self" {
                let target = impl_target.ok_or_else(|| LingoError::new(
                    Stage::Resolve,
                    "C backend: `self` only allowed inside `impl Type:` blocks",
                    p.span,
                ))?;
                params.push(CType::Struct(target.to_string()));
            } else {
                params.push(map_type_with(&p.ty, p.span, Some(&self.structs))?);
            }
        }
        let ret = match &f.return_type {
            Some(t) => map_type_with(t, f.span, Some(&self.structs))?,
            None => CType::Void,
        };
        let name = match impl_target {
            Some(t) => format!("{}_{}", t, f.name),
            None => f.name.clone(),
        };
        self.fn_sigs.insert(name, (params, ret));
        Ok(())
    }

    /// Build a C-style prototype `RetType name(T0 p0, T1 p1, ...)`.
    /// `c_name` is the mangled C function name (e.g. `Point_dist_sq`).
    fn fn_proto(&self, c_name: &str, f: &FnDecl) -> Result<String, LingoError> {
        let (params, ret) = self
            .fn_sigs
            .get(c_name)
            .expect("signature must be registered before emit");
        let mut s = String::new();
        // C's `main` must return int — even though lingo's main returns nothing.
        let c_ret: String = if c_name == "main" { "int".into() } else { ret.c_decl() };
        write!(s, "{} {}(", c_ret, c_name).unwrap();
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

    fn emit_fn_body(
        &mut self,
        c_name: &str,
        f: &FnDecl,
        _impl_target: Option<&str>,
    ) -> Result<(), LingoError> {
        let proto = self.fn_proto(c_name, f)?;
        writeln!(self.body, "{} {{", proto).unwrap();
        self.indent = 1;
        self.scopes.push(HashMap::new());
        let (params, _) = self.fn_sigs.get(c_name).cloned().unwrap();
        for (i, p) in f.params.iter().enumerate() {
            self.scopes.last_mut().unwrap().insert(p.name.clone(), params[i].clone());
        }
        for s in &f.body.stmts {
            self.emit_stmt(s)?;
        }
        // For `main`, always finish with `return 0;` (C requires an int return).
        if c_name == "main" {
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
                    None => val_ty.clone(),
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
                return Some(t.clone());
            }
        }
        None
    }

    fn gen_expr(&mut self, e: &Expr) -> Result<(String, CType), LingoError> {
        Ok(match &e.kind {
            ExprKind::Int(n) => (format!("((int64_t){}LL)", n), CType::I64),
            ExprKind::Bool(b) => ((if *b { "true" } else { "false" }).to_string(), CType::Bool),
            ExprKind::Str(s) => (format!("\"{}\"", escape_c(s)), CType::Str),
            ExprKind::Self_ => {
                let ty = self.lookup_var("self").ok_or_else(|| LingoError::new(
                    Stage::Resolve,
                    "C backend: `self` used outside an impl method",
                    e.span,
                ))?;
                ("self".to_string(), ty)
            }
            ExprKind::Field(receiver, name) => {
                // Plain field read: lower to `receiver.field`.
                // We disallow this on type names (those are static-method refs,
                // which can only appear inside a Call — handled in gen_call).
                if let ExprKind::Ident(id) = &receiver.kind {
                    if self.structs.contains_key(id) {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("C backend: bare reference to `{}.{}` is not a value (call it as a function)", id, name),
                            e.span,
                        ));
                    }
                }
                let (r_code, r_ty) = self.gen_expr(receiver)?;
                let struct_name = match &r_ty {
                    CType::Struct(n) => n.clone(),
                    _ => {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("C backend: cannot read field `{}` on non-struct value", name),
                            e.span,
                        ));
                    }
                };
                let fty = self.structs[&struct_name].iter()
                    .find(|(fname, _)| fname == name)
                    .map(|(_, t)| t.clone())
                    .ok_or_else(|| LingoError::new(
                        Stage::Resolve,
                        format!("C backend: `{}` has no field `{}`", struct_name, name),
                        e.span,
                    ))?;
                (format!("({}).{}", r_code, name), fty)
            }
            ExprKind::StructLit { name, fields } => {
                if !self.structs.contains_key(name) {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("C backend: unknown struct `{}`", name),
                        e.span,
                    ));
                }
                let mut parts = Vec::with_capacity(fields.len());
                for (fname, fexpr) in fields {
                    let (code, _) = self.gen_expr(fexpr)?;
                    parts.push(format!(".{} = {}", fname, code));
                }
                (
                    format!("(({}){{ {} }})", name, parts.join(", ")),
                    CType::Struct(name.clone()),
                )
            }
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
        // Three call shapes we recognize:
        //   1. `foo(args)`            — free function
        //   2. `Type.method(args)`    — static method on a struct  -> `Type_method(args)`
        //   3. `obj.method(args)`     — instance method            -> `Type_method(obj, args)`
        let (mangled, prepend_self_code) = match &callee.kind {
            ExprKind::Ident(s) => (s.clone(), None),
            ExprKind::Field(receiver, method) => {
                // Static call when the receiver is a known struct name.
                if let ExprKind::Ident(id) = &receiver.kind {
                    if self.structs.contains_key(id) {
                        (format!("{}_{}", id, method), None)
                    } else {
                        // free-fn-like field call would be a value method
                        let (r_code, r_ty) = self.gen_expr(receiver)?;
                        let struct_name = match &r_ty {
                            CType::Struct(n) => n.clone(),
                            _ => {
                                return Err(LingoError::new(
                                    Stage::Resolve,
                                    format!("C backend: method `{}` on non-struct receiver", method),
                                    span,
                                ));
                            }
                        };
                        (format!("{}_{}", struct_name, method), Some(r_code))
                    }
                } else {
                    let (r_code, r_ty) = self.gen_expr(receiver)?;
                    let struct_name = match &r_ty {
                        CType::Struct(n) => n.clone(),
                        _ => {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("C backend: method `{}` on non-struct receiver", method),
                                span,
                            ));
                        }
                    };
                    (format!("{}_{}", struct_name, method), Some(r_code))
                }
            }
            _ => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: only named function calls are supported",
                    span,
                ));
            }
        };
        let (param_tys, ret) = self.fn_sigs.get(&mangled).cloned().ok_or_else(|| {
            LingoError::new(
                Stage::Resolve,
                format!("C backend: function `{}` is not defined", mangled),
                span,
            )
        })?;
        // Total args we'll pass to the C function:
        let total = args.len() + if prepend_self_code.is_some() { 1 } else { 0 };
        if total != param_tys.len() {
            return Err(LingoError::new(
                Stage::Resolve,
                format!("`{}` expects {} args, got {}", mangled, param_tys.len(), total),
                span,
            ));
        }
        let mut parts: Vec<String> = Vec::with_capacity(total);
        if let Some(s) = prepend_self_code {
            parts.push(s);
        }
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
        Ok((format!("{}({})", mangled, parts.join(", ")), ret))
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
