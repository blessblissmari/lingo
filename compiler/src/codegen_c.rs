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
    F64,
    Bool,
    Str, // const char*
    Void,
    /// User-defined struct type. The `String` is its lingo name, which is
    /// also the typedef'd C name (no mangling needed — lingo names are
    /// already valid C identifiers).
    Struct(String),
    /// User-defined tagged-union (lingo `enum`). Same naming convention:
    /// the C type is a typedef of the same lingo name.
    Enum(String),
    /// `vec[T]` — a small POD struct holding a `const T*` plus length.
    /// Read-only for now; literals lower to C99 compound array literals so
    /// the backing array's lifetime is the enclosing C block.  Mutation
    /// (`push`/`pop`) waits on the allocator story.
    ///
    /// Element type is restricted to a small fixed set (i64 / f64 / str)
    /// because we hand-emit per-element-type runtime structs and helpers
    /// instead of doing full monomorphization.  Once generics land we'll
    /// collapse those down to a single template.
    Vec(Box<CType>),
}

impl CType {
    fn c_decl(&self) -> String {
        match self {
            CType::I64 => "int64_t".into(),
            CType::U64 => "uint64_t".into(),
            CType::F64 => "double".into(),
            CType::Bool => "bool".into(),
            CType::Str => "const char*".into(),
            CType::Void => "void".into(),
            CType::Struct(name) => name.clone(),
            CType::Enum(name) => name.clone(),
            CType::Vec(inner) => match inner.as_ref() {
                CType::I64 => "lingo_vec_i64_t".into(),
                CType::F64 => "lingo_vec_f64_t".into(),
                CType::Str => "lingo_vec_str_t".into(),
                // Unsupported inner types are caught at `map_type_with`, so
                // hitting this branch means the compiler has a bug.
                other => panic!("unsupported vec element type in codegen: {:?}", other),
            },
        }
    }

    /// printf format specifier for this type. We splice these in as C
    /// `PRId64`/`PRIu64` macros (from <inttypes.h>) so the format
    /// stays correct on both 32- and 64-bit platforms.
    fn printf_fmt(&self) -> &'static str {
        match self {
            CType::I64 => "%\" PRId64 \"",
            CType::U64 => "%\" PRIu64 \"",
            CType::F64 => "%g",
            CType::Bool => "%s", // we print "true"/"false"
            CType::Str => "%s",
            CType::Void => "",
            CType::Struct(_) => "<struct>", // not directly printable yet
            CType::Enum(_) => "<enum>",     // not directly printable yet
            CType::Vec(_) => "<vec>",        // printed via emit_print special-case
        }
    }
}

/// Lower a lingo type reference to a `CType`.
/// `known_structs` lets us recognize user-defined struct names; we keep it
/// optional so call sites that only care about primitives can pass `None`.
fn map_type_with(
    t: &TypeRef,
    span: Span,
    structs: Option<&HashMap<String, Vec<(String, CType)>>>,
    enums: Option<&HashMap<String, EnumDecl>>,
) -> Result<CType, LingoError> {
    // `vec[i64]` is the one generic shape we recognize so far — everything
    // else with type args bails out until we have monomorphization.
    if t.name == "vec" {
        if t.type_args.len() != 1 {
            return Err(LingoError::new(
                Stage::Resolve,
                "C backend: `vec` needs exactly one type argument, e.g. `vec[i64]`",
                span,
            ));
        }
        let inner = map_type_with(&t.type_args[0], span, structs, enums)?;
        match &inner {
            CType::I64 | CType::F64 | CType::Str => {}
            other => return Err(LingoError::new(
                Stage::Resolve,
                format!("C backend: only `vec[i64]`, `vec[f64]`, `vec[str]` are supported in v0.1.14 (got `vec[{}]`)",
                        other.c_decl()),
                span,
            )),
        }
        return Ok(CType::Vec(Box::new(inner)));
    }
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
        "f64" | "float" => CType::F64,
        "bool" => CType::Bool,
        "str" => CType::Str,
        other => {
            if let Some(map) = structs {
                if map.contains_key(other) {
                    return Ok(CType::Struct(other.to_string()));
                }
            }
            if let Some(map) = enums {
                if map.contains_key(other) {
                    return Ok(CType::Enum(other.to_string()));
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
    map_type_with(t, span, None, None)
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
    /// Parameter names parallel to `fn_sigs`.  Stored separately so the rest of
    /// the codegen doesn't have to thread name-typed tuples around.  Used by
    /// `gen_call` to resolve keyword arguments to positional slots.
    fn_param_names: HashMap<String, Vec<String>>,
    /// `struct_name -> [(field_name, field_type)]`, in declared order.
    structs: HashMap<String, Vec<(String, CType)>>,
    /// `enum_name -> EnumDecl`, kept around for variant lookup.
    enums: HashMap<String, EnumDecl>,
    /// Stack of local-scope variable types. Top frame is the active scope.
    scopes: Vec<HashMap<String, CType>>,
    /// How deep are we indented in the current C function body?
    indent: usize,
    /// Monotonically increasing counter for synthesized temporaries
    /// (`__pr_<N>` in debug prints, `__match_<N>` in match lowering).
    /// Reset per function in `emit_fn_body` to keep names short and readable.
    tmp_counter: usize,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            body: String::new(),
            protos: String::new(),
            type_defs: String::new(),
            fn_sigs: HashMap::new(),
            fn_param_names: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            scopes: Vec::new(),
            indent: 0,
            tmp_counter: 0,
        }
    }

    /// Compile a whole program to a self-contained C99 source file.
    pub fn gen_program(mut self, prog: &Program) -> Result<String, LingoError> {
        // Bail fast on anything the C backend can't handle.
        for item in &prog.items {
            match item {
                Item::Fn(_) | Item::Const(_) | Item::Struct(_) | Item::Impl(_) | Item::Enum(_) => {}
                _ => {
                    let span = match item {
                        Item::Trait(t) => t.span,
                        Item::ImplTrait(b) => b.span,
                        _ => Span::dummy(),
                    };
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: `trait` and `impl Trait for Type` need \
                         v0.2 vtable lowering. Use the interpreter for now.",
                        span,
                    ));
                }
            }
        }

        // Pass 0: register enums (names only) so struct fields / fn params /
        // return types can reference them.
        for item in &prog.items {
            if let Item::Enum(e) = item {
                self.enums.insert(e.name.clone(), e.clone());
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
                    let ty = map_type_with(&f.ty, f.span, Some(&self.structs), Some(&self.enums))?;
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

        // Pass 1d: emit tagged-union typedef for each enum.
        // The shape is:
        //     typedef enum { T_V1_TAG, T_V2_TAG, ... } T_Tag;
        //     typedef struct T {
        //         T_Tag tag;
        //         union {
        //             struct { /* payload fields _0, _1, ... */ } V1;
        //             struct { } V2;
        //             ...
        //         } as;
        //     } T;
        // Nullary variants get an empty struct (allowed in GNU/clang C; for
        // strict C99 we keep at least a dummy field).
        let enum_decls: Vec<EnumDecl> = prog.items.iter().filter_map(|it| {
            if let Item::Enum(e) = it { Some(e.clone()) } else { None }
        }).collect();
        for e in &enum_decls {
            writeln!(self.type_defs, "typedef enum {{").unwrap();
            for v in &e.variants {
                writeln!(self.type_defs, "    {}_{}_TAG,", e.name, v.name).unwrap();
            }
            writeln!(self.type_defs, "}} {}_Tag;", e.name).unwrap();
            writeln!(self.type_defs, "typedef struct {} {{", e.name).unwrap();
            writeln!(self.type_defs, "    {}_Tag tag;", e.name).unwrap();
            writeln!(self.type_defs, "    union {{").unwrap();
            for v in &e.variants {
                if v.payload.is_empty() {
                    writeln!(self.type_defs, "        struct {{ char _dummy; }} {};", v.name).unwrap();
                } else {
                    writeln!(self.type_defs, "        struct {{").unwrap();
                    for (i, p) in v.payload.iter().enumerate() {
                        let ty = map_type_with(p, v.span, Some(&self.structs), Some(&self.enums))?;
                        writeln!(self.type_defs, "            {} _{};", ty.c_decl(), i).unwrap();
                    }
                    writeln!(self.type_defs, "        }} {};", v.name).unwrap();
                }
            }
            writeln!(self.type_defs, "    }} as;").unwrap();
            writeln!(self.type_defs, "}} {};\n", e.name).unwrap();
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
                Item::Enum(_) => {}   // already emitted in pass 1d
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
        out.push_str("#include <math.h>\n"); // for `pow`, `sqrt`, etc. when f64 lands
        out.push_str("#include <stddef.h>\n"); // size_t, NULL
        out.push_str("#include <string.h>\n"); // strlen/strcmp/strstr/memcpy (str runtime, v0.1.13)
        out.push_str("#include <stdlib.h>\n"); // malloc (str runtime, v0.1.13)
        out.push_str("#include <stdarg.h>\n"); // va_list (lingo_fmt_alloc, v0.1.13)
        out.push_str("\n");
        // Tiny built-in runtime types — typedef'd up front so user code and
        // generated method calls can refer to them by name.  Read-only vec
        // for v0.1.12: data lifetime tracks the enclosing C block.  When the
        // allocator story lands, this grows to an owning vector with cap +
        // realloc, without changing the public API (`len`/`get`).
        out.push_str("typedef struct { const int64_t* data; int64_t len; } lingo_vec_i64_t;\n");
        out.push_str("typedef struct { const double*  data; int64_t len; } lingo_vec_f64_t;\n");
        out.push_str("typedef struct { const char**   data; int64_t len; } lingo_vec_str_t;\n\n");
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
                params.push(map_type_with(&p.ty, p.span, Some(&self.structs), Some(&self.enums))?);
            }
        }
        let ret = match &f.return_type {
            Some(t) => map_type_with(t, f.span, Some(&self.structs), Some(&self.enums))?,
            None => CType::Void,
        };
        let name = match impl_target {
            Some(t) => format!("{}_{}", t, f.name),
            None => f.name.clone(),
        };
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        self.fn_param_names.insert(name.clone(), param_names);
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
        self.tmp_counter = 0;
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
                // Two shapes supported:
                //   `for i in lo..hi`  — integer range
                //   `for x in vec_i64` — iterate a vec[i64]
                match &iter.kind {
                    ExprKind::Range(a, b) => {
                        let (lo_code, _) = self.gen_expr(a)?;
                        let (hi_code, _) = self.gen_expr(b)?;
                        writeln!(self.body,
                            "{}for (int64_t {var} = {lo_code}; {var} < {hi_code}; ++{var}) {{",
                            self.pad()).unwrap();
                        self.indent += 1;
                        self.scopes.push(HashMap::new());
                        self.scopes.last_mut().unwrap().insert(var.clone(), CType::I64);
                        for s in &body.stmts { self.emit_stmt(s)?; }
                        self.scopes.pop();
                        self.indent -= 1;
                        writeln!(self.body, "{}}}", self.pad()).unwrap();
                    }
                    _ => {
                        // Bind the iterable to a temp so we evaluate once even if it's
                        // a compound literal like `vec[1,2,3]`.
                        let (iter_code, iter_ty) = self.gen_expr(iter)?;
                        let elem_ty = match &iter_ty {
                            CType::Vec(inner) => (**inner).clone(),
                            other => {
                                return Err(LingoError::new(
                                    Stage::Resolve,
                                    format!("C backend: `for` needs a range or vec, got `{}`",
                                            other.c_decl()),
                                    *span,
                                ));
                            }
                        };
                        let vec_c = iter_ty.c_decl();
                        let elem_c = elem_ty.c_decl();
                        let tmp = format!("__it_{}", self.tmp_counter);
                        self.tmp_counter += 1;
                        writeln!(self.body, "{}{} {} = {};",
                                 self.pad(), vec_c, tmp, iter_code).unwrap();
                        let ix = self.tmp_counter;
                        self.tmp_counter += 1;
                        writeln!(self.body,
                            "{}for (int64_t __ix_{ix} = 0; __ix_{ix} < {tmp}.len; ++__ix_{ix}) {{",
                            self.pad(), ix = ix).unwrap();
                        self.indent += 1;
                        writeln!(self.body,
                            "{}{} {var} = {tmp}.data[__ix_{ix}];",
                            self.pad(), elem_c, ix = ix).unwrap();
                        self.scopes.push(HashMap::new());
                        self.scopes.last_mut().unwrap().insert(var.clone(), elem_ty);
                        for s in &body.stmts { self.emit_stmt(s)?; }
                        self.scopes.pop();
                        self.indent -= 1;
                        writeln!(self.body, "{}}}", self.pad()).unwrap();
                    }
                }
            }
            Stmt::Break(_) => {
                writeln!(self.body, "{}break;", self.pad()).unwrap();
            }
            Stmt::Continue(_) => {
                writeln!(self.body, "{}continue;", self.pad()).unwrap();
            }
            Stmt::Match { scrutinee, arms, span } => {
                self.emit_match(scrutinee, arms, *span)?;
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

    /// Lower a `match` statement to a C `switch`.
    /// Supports enum-variant patterns + a single wildcard / bind catch-all.
    /// Literal patterns are out of scope for now.
    fn emit_match(&mut self, scrut: &Expr, arms: &[MatchArm], span: Span) -> Result<(), LingoError> {
        let (scrut_code, scrut_ty) = self.gen_expr(scrut)?;
        let enum_name = match &scrut_ty {
            CType::Enum(n) => n.clone(),
            _ => {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "C backend: `match` only supports enum scrutinees in v0.1.9",
                    span,
                ));
            }
        };
        // Stash scrutinee into a local so subpattern bindings can reference it.
        let tmp = format!("__match_{}", self.tmp_counter);
        self.tmp_counter += 1;
        writeln!(self.body, "{}{} {} = {};", self.pad(), enum_name, tmp, scrut_code).unwrap();
        writeln!(self.body, "{}switch ({}.tag) {{", self.pad(), tmp).unwrap();
        let mut had_default = false;
        let decl = self.enums.get(&enum_name).cloned().unwrap();
        for arm in arms {
            match &arm.pattern {
                Pattern::Variant { type_name, variant, sub, span: pat_span } => {
                    // sanity: type_name (if given) must match the scrutinee.
                    if let Some(tn) = type_name {
                        if tn != &enum_name {
                            return Err(LingoError::new(
                                Stage::Resolve,
                                format!("pattern type `{}` doesn't match scrutinee type `{}`", tn, enum_name),
                                *pat_span,
                            ));
                        }
                    }
                    let v = decl.variants.iter().find(|v| v.name == *variant).ok_or_else(|| {
                        LingoError::new(
                            Stage::Resolve,
                            format!("`{}` has no variant `{}`", enum_name, variant),
                            *pat_span,
                        )
                    })?;
                    if sub.len() != v.payload.len() {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("variant `{}.{}` binds {} values, pattern has {}",
                                    enum_name, variant, v.payload.len(), sub.len()),
                            *pat_span,
                        ));
                    }
                    writeln!(self.body, "{}case {}_{}_TAG: {{",
                             self.pad(), enum_name, variant).unwrap();
                    self.indent += 1;
                    self.scopes.push(HashMap::new());
                    // bind payload subpatterns: only `Bind(name)` and `Wildcard` allowed.
                    for (i, sp) in sub.iter().enumerate() {
                        match sp {
                            Pattern::Wildcard(_) => {}
                            Pattern::Bind(name, sp_span) => {
                                let ty = map_type_with(
                                    &v.payload[i],
                                    *sp_span,
                                    Some(&self.structs),
                                    Some(&self.enums),
                                )?;
                                writeln!(self.body, "{}{} {} = {}.as.{}._{};",
                                         self.pad(), ty.c_decl(), name, tmp, variant, i).unwrap();
                                self.scopes.last_mut().unwrap().insert(name.clone(), ty);
                            }
                            _ => {
                                return Err(LingoError::new(
                                    Stage::Resolve,
                                    "C backend: nested patterns aren't supported in v0.1.9 \
                                     (only `name` or `_` inside variants)",
                                    *pat_span,
                                ));
                            }
                        }
                    }
                    for s in &arm.body.stmts {
                        self.emit_stmt(s)?;
                    }
                    writeln!(self.body, "{}break;", self.pad()).unwrap();
                    self.scopes.pop();
                    self.indent -= 1;
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                }
                Pattern::Wildcard(_) => {
                    writeln!(self.body, "{}default: {{", self.pad()).unwrap();
                    self.indent += 1;
                    self.scopes.push(HashMap::new());
                    for s in &arm.body.stmts {
                        self.emit_stmt(s)?;
                    }
                    writeln!(self.body, "{}break;", self.pad()).unwrap();
                    self.scopes.pop();
                    self.indent -= 1;
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                    had_default = true;
                }
                Pattern::Bind(name, _) => {
                    writeln!(self.body, "{}default: {{", self.pad()).unwrap();
                    self.indent += 1;
                    self.scopes.push(HashMap::new());
                    writeln!(self.body, "{}{} {} = {};",
                             self.pad(), enum_name, name, tmp).unwrap();
                    self.scopes.last_mut().unwrap().insert(name.clone(), CType::Enum(enum_name.clone()));
                    for s in &arm.body.stmts {
                        self.emit_stmt(s)?;
                    }
                    writeln!(self.body, "{}break;", self.pad()).unwrap();
                    self.scopes.pop();
                    self.indent -= 1;
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                    had_default = true;
                }
                Pattern::Literal(_, lit_span) => {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: literal patterns land in v0.2 (need int/bool scrutinees)",
                        *lit_span,
                    ));
                }
            }
        }
        // If no default arm was provided, guarantee the switch is total
        // (otherwise some C compilers warn).  We don't try to be smart
        // about exhaustiveness — that's the type checker's job.
        if !had_default {
            // We tell the C compiler the switch is total. If a future variant
            // is added without updating the match, we hit UB at runtime —
            // that's the trade-off for getting clean warnings today. A real
            // exhaustiveness checker (Phase 1.5) will catch this at compile time.
            writeln!(self.body, "{}default: __builtin_unreachable();", self.pad()).unwrap();
        }
        writeln!(self.body, "{}}}", self.pad()).unwrap();
        Ok(())
    }

    fn emit_print(&mut self, args: &[Arg], span: Span) -> Result<(), LingoError> {
        // Build a single printf("fmt", ...). Multiple args separated by spaces,
        // newline at end. Bool values are converted to "true"/"false" strings.
        // Struct and enum values get auto-generated debug formats so
        // `print(point)` Just Works — same intent as Rust's `{:?}`.
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
            match &ty {
                CType::Bool => {
                    fmt.push_str("%s");
                    vals.push(format!("(({}) ? \"true\" : \"false\")", code));
                }
                CType::Struct(name) => {
                    let fields = self.structs.get(name).cloned().ok_or_else(|| {
                        LingoError::new(Stage::Resolve,
                            format!("C backend: struct `{}` not registered", name), a.span)
                    })?;
                    // `Name{f1=<fmt>, f2=<fmt>}` — bind the struct to a temp so
                    // we evaluate the expression exactly once even if it has side effects.
                    let tmp = format!("__pr_{}_{}", name, self.tmp_counter);
                    self.tmp_counter += 1;
                    writeln!(self.body, "{}{} {} = {};", self.pad(), name, tmp, code).unwrap();
                    fmt.push_str(name);
                    fmt.push('{');
                    for (fi, (fname, fty)) in fields.iter().enumerate() {
                        if fi > 0 { fmt.push_str(", "); }
                        fmt.push_str(fname);
                        fmt.push_str(": ");
                        fmt.push_str(&debug_fmt_for(fty));
                        vals.push(debug_val_for(fty, &format!("{}.{}", tmp, fname)));
                    }
                    fmt.push('}');
                }
                CType::Enum(name) => {
                    let decl = self.enums.get(name).cloned().ok_or_else(|| {
                        LingoError::new(Stage::Resolve,
                            format!("C backend: enum `{}` not registered", name), a.span)
                    })?;
                    // Switch on the tag at print time; emit a small helper expression
                    // via a comma'd block using a `__match` temp.  We unroll into the
                    // printf by emitting it now and finishing the rest of the format
                    // separately — so the enum becomes a self-contained printf line.
                    if !fmt.is_empty() {
                        // flush the partial line so the enum can stand alone
                        // (keeps the printf logic linear)
                        if vals.is_empty() {
                            writeln!(self.body, "{}printf(\"{}\");", self.pad(), fmt).unwrap();
                        } else {
                            writeln!(self.body, "{}printf(\"{}\", {});",
                                     self.pad(), fmt, vals.join(", ")).unwrap();
                        }
                        fmt.clear();
                        vals.clear();
                    }
                    let tmp = format!("__pr_{}_{}", name, self.tmp_counter);
                    self.tmp_counter += 1;
                    writeln!(self.body, "{}{} {} = {};", self.pad(), name, tmp, code).unwrap();
                    writeln!(self.body, "{}switch ({}.tag) {{", self.pad(), tmp).unwrap();
                    for v in &decl.variants {
                        writeln!(self.body, "{}    case {}_{}_TAG: {{",
                                 self.pad(), name, v.name).unwrap();
                        let mut inner_fmt = format!("{}.{}", name, v.name);
                        let mut inner_vals: Vec<String> = Vec::new();
                        if !v.payload.is_empty() {
                            inner_fmt.push('(');
                            for (pi, p) in v.payload.iter().enumerate() {
                                if pi > 0 { inner_fmt.push_str(", "); }
                                let pty = map_type_with(p, v.span, Some(&self.structs), Some(&self.enums))?;
                                inner_fmt.push_str(&debug_fmt_for(&pty));
                                inner_vals.push(debug_val_for(&pty, &format!("{}.as.{}._{}", tmp, v.name, pi)));
                            }
                            inner_fmt.push(')');
                        }
                        // Trailing space if this isn't the last printed arg; newline
                        // at the end is added below when the loop finishes.
                        if i + 1 < args.len() {
                            inner_fmt.push(' ');
                        }
                        if inner_vals.is_empty() {
                            writeln!(self.body, "{}        printf(\"{}\");",
                                     self.pad(), inner_fmt).unwrap();
                        } else {
                            writeln!(self.body, "{}        printf(\"{}\", {});",
                                     self.pad(), inner_fmt, inner_vals.join(", ")).unwrap();
                        }
                        writeln!(self.body, "{}        break;", self.pad()).unwrap();
                        writeln!(self.body, "{}    }}", self.pad()).unwrap();
                    }
                    writeln!(self.body, "{}    default: __builtin_unreachable();", self.pad()).unwrap();
                    writeln!(self.body, "{}}}", self.pad()).unwrap();
                }
                _ => {
                    fmt.push_str(ty.printf_fmt());
                    vals.push(code);
                }
            }
        }
        // Trailing newline for whatever's left in the buffer.
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
                // Bare nullary variant reference: `Foo.Bar` (no Call wrapping).
                if let ExprKind::Ident(id) = &receiver.kind {
                    if let Some(decl) = self.enums.get(id).cloned() {
                        if let Some(v) = decl.variants.iter().find(|v| v.name == *name) {
                            if !v.payload.is_empty() {
                                return Err(LingoError::new(
                                    Stage::Resolve,
                                    format!("variant `{}.{}` needs arguments", id, name),
                                    e.span,
                                ));
                            }
                            return Ok((
                                format!("(({}){{ .tag = {}_{}_TAG }})", id, id, name),
                                CType::Enum(id.clone()),
                            ));
                        }
                    }
                }
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
            ExprKind::Float(f) => {
                // {:?} on f64 always includes a decimal point ("1" -> "1.0"), so the
                // emitted token is unambiguously a C double literal.  We don't try
                // to round-trip-perfectly; -O2 will fold these to identical IEEE bits.
                let s = if f.is_nan() {
                    "(0.0/0.0)".to_string()
                } else if f.is_infinite() {
                    if *f > 0.0 { "(1.0/0.0)".to_string() } else { "(-1.0/0.0)".to_string() }
                } else {
                    format!("{:?}", f)
                };
                (s, CType::F64)
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
            ExprKind::FString(parts) => {
                // Lower `f"hello, {name}, you are {age}"` to a snprintf call:
                //   - assemble a format string with `%s` for each interpolation
                //   - build the args list using printf_fmt() for each value
                //   - two-pass snprintf to size and fill a fresh malloc'd buffer
                // We lift the work into an expression by emitting a statement-expr
                // helper.  Since C doesn't have stmt-exprs portably, we instead emit
                // a runtime function `lingo_fmt_alloc` for the simple case where
                // every interpolation is `%s`/`%lld`/etc., and synthesize a printf-
                // shaped call.
                let mut fmt = String::new();
                let mut vals: Vec<String> = Vec::new();
                for p in parts {
                    match p {
                        FStringPart::Lit(s) => {
                            // Escape `%` to avoid it being treated as a fmt spec,
                            // and escape C string-literal specials (`"`, `\`, etc.).
                            for ch in s.chars() {
                                if ch == '%' { fmt.push_str("%%"); }
                                else { fmt.push_str(&escape_c(&ch.to_string())); }
                            }
                        }
                        FStringPart::Expr(ex) => {
                            let (code, ty) = self.gen_expr(ex)?;
                            match &ty {
                                CType::Bool => {
                                    fmt.push_str("%s");
                                    vals.push(format!("(({}) ? \"true\" : \"false\")", code));
                                }
                                CType::Str => {
                                    fmt.push_str("%s");
                                    vals.push(code);
                                }
                                CType::I64 | CType::U64 | CType::F64 => {
                                    fmt.push_str(ty.printf_fmt());
                                    vals.push(code);
                                }
                                other => {
                                    return Err(LingoError::new(
                                        Stage::Resolve,
                                        format!("C backend: f-string can't interpolate `{}` yet \
                                                 (only primitives in v0.1.13)",
                                                other.c_decl()),
                                        ex.span,
                                    ));
                                }
                            }
                        }
                    }
                }
                // The varargs list and format are baked into a single call.
                // `lingo_fmt_alloc` does the two-pass snprintf for us — see the
                // runtime helper below.
                let args = if vals.is_empty() {
                    String::new()
                } else {
                    format!(", {}", vals.join(", "))
                };
                (
                    format!("lingo_fmt_alloc(\"{}\"{})", fmt, args),
                    CType::Str,
                )
            }
            ExprKind::VecLit(items) => {
                // Lowers to a C99 compound literal:
                //   ((lingo_vec_<T>_t){ .data = (<C-T>[]){ a, b, c }, .len = N })
                // The inner array's lifetime is the enclosing block (C99 §6.5.2.5),
                // so the vec is valid as long as we're inside that block — fine for
                // reads/iteration; mutation lands when we wire up an allocator.
                if items.is_empty() {
                    // Empty literal — we can't infer the element type; default to
                    // i64 for now.  Once we have a type checker it can fix this
                    // from context (e.g. variable annotation).
                    return Ok((
                        "((lingo_vec_i64_t){ .data = NULL, .len = 0 })".to_string(),
                        CType::Vec(Box::new(CType::I64)),
                    ));
                }
                // Infer element type from the first item; require the rest to match.
                let (first_code, first_ty) = self.gen_expr(&items[0])?;
                match first_ty {
                    CType::I64 | CType::F64 | CType::Str => {}
                    ref other => return Err(LingoError::new(
                        Stage::Resolve,
                        format!("C backend: vec element type `{}` not supported (i64/f64/str only)",
                                other.c_decl()),
                        items[0].span,
                    )),
                }
                let mut parts = Vec::with_capacity(items.len());
                parts.push(first_code);
                for it in &items[1..] {
                    let (code, ty) = self.gen_expr(it)?;
                    if ty != first_ty {
                        return Err(LingoError::new(
                            Stage::Resolve,
                            format!("C backend: vec literal mixed types — first was `{}`, got `{}`",
                                    first_ty.c_decl(), ty.c_decl()),
                            it.span,
                        ));
                    }
                    parts.push(code);
                }
                let vec_ty = CType::Vec(Box::new(first_ty.clone()));
                let elem_c = first_ty.c_decl();
                (
                    format!(
                        "(({}){{ .data = ({}[]){{ {} }}, .len = {} }})",
                        vec_ty.c_decl(),
                        elem_c,
                        parts.join(", "),
                        items.len()
                    ),
                    vec_ty,
                )
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
                // String concat is the one non-numeric `+` we recognize.  Lowers to
                // a runtime helper that mallocs a fresh buffer and copies both
                // halves.  v0.1.x leaks; we'll thread an allocator through when
                // `defer` lands.
                if op == BinOp::Add && a_ty == CType::Str && b_ty == CType::Str {
                    return Ok((
                        format!("lingo_str_concat({}, {})", a_code, b_code),
                        CType::Str,
                    ));
                }
                if op == BinOp::Add && (a_ty == CType::Str || b_ty == CType::Str) {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: `+` between str and non-str — use f-strings or `str(x)` to convert first",
                        a.span,
                    ));
                }
                // Float-aware numeric op.  We don't do implicit numeric promotion in
                // lingo, but if either operand is f64 we treat the whole expression
                // as f64 and let C upcast the int side (which is exactly what lingo's
                // type checker will require at the boundary).
                let is_float = a_ty == CType::F64 || b_ty == CType::F64;
                if op == BinOp::Mod && is_float {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: `%` on f64 isn't supported (use `fmod` in v0.2)",
                        a.span,
                    ));
                }
                let ty = if is_float {
                    CType::F64
                } else if a_ty == CType::U64 || b_ty == CType::U64 {
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
                // For floats we lower to `pow()` from <math.h>.  For integers we keep
                // the original repeated-multiplication helper.
                if a_ty == CType::F64 || b_ty == CType::F64 {
                    let a_cast = if a_ty == CType::F64 { a_code } else { format!("(double)({})", a_code) };
                    let b_cast = if b_ty == CType::F64 { b_code } else { format!("(double)({})", b_code) };
                    return Ok((format!("pow({}, {})", a_cast, b_cast), CType::F64));
                }
                let ty = if a_ty == CType::U64 || b_ty == CType::U64 {
                    CType::U64
                } else {
                    CType::I64
                };
                Ok((format!("lingo_ipow({}, {})", a_code, b_code), ty))
            }
            BinOp::Eq => {
                if a_ty == CType::Str && b_ty == CType::Str {
                    return Ok((format!("(strcmp({}, {}) == 0)", a_code, b_code), CType::Bool));
                }
                Ok((format!("({} == {})", a_code, b_code), CType::Bool))
            }
            BinOp::Ne => {
                if a_ty == CType::Str && b_ty == CType::Str {
                    return Ok((format!("(strcmp({}, {}) != 0)", a_code, b_code), CType::Bool));
                }
                Ok((format!("({} != {})", a_code, b_code), CType::Bool))
            }
            BinOp::Lt => Ok((format!("({} <  {})", a_code, b_code), CType::Bool)),
            BinOp::Le => Ok((format!("({} <= {})", a_code, b_code), CType::Bool)),
            BinOp::Gt => Ok((format!("({} >  {})", a_code, b_code), CType::Bool)),
            BinOp::Ge => Ok((format!("({} >= {})", a_code, b_code), CType::Bool)),
            BinOp::And => Ok((format!("({} && {})", a_code, b_code), CType::Bool)),
            BinOp::Or => Ok((format!("({} || {})", a_code, b_code), CType::Bool)),
        }
    }

    /// Construct an enum value: `Foo.Bar(x, y)` → `(Foo){ .tag = Foo_Bar_TAG, .as.Bar = { ._0 = x, ._1 = y } }`.
    fn gen_enum_ctor(
        &mut self,
        type_name: &str,
        decl: &EnumDecl,
        variant: &str,
        args: &[Arg],
        span: Span,
    ) -> Result<(String, CType), LingoError> {
        let v = decl.variants.iter().find(|v| v.name == variant).ok_or_else(|| {
            LingoError::new(
                Stage::Resolve,
                format!("`{}` has no variant `{}`", type_name, variant),
                span,
            )
        })?;
        if args.len() != v.payload.len() {
            return Err(LingoError::new(
                Stage::Resolve,
                format!("variant `{}.{}` expects {} payload value(s), got {}",
                        type_name, variant, v.payload.len(), args.len()),
                span,
            ));
        }
        let tag = format!("{}_{}_TAG", type_name, variant);
        if args.is_empty() {
            return Ok((
                format!("(({}){{ .tag = {} }})", type_name, tag),
                CType::Enum(type_name.to_string()),
            ));
        }
        let mut parts = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            if a.name.is_some() {
                return Err(LingoError::new(
                    Stage::Resolve,
                    "variant payload must be positional",
                    a.span,
                ));
            }
            let (code, _) = self.gen_expr(&a.value)?;
            parts.push(format!("._{} = {}", i, code));
        }
        Ok((
            format!("(({}){{ .tag = {}, .as.{} = {{ {} }} }})",
                    type_name, tag, variant, parts.join(", ")),
            CType::Enum(type_name.to_string()),
        ))
    }

    /// Built-in methods on `str` (= `const char*` in C).  v0.1.13 subset:
    ///   - `s.len()`         -> `((int64_t)strlen(s))`        (byte count!)
    ///   - `s.contains(t)`   -> `(strstr(s, t) != NULL)`
    ///   - `s.starts_with(t)`-> `lingo_str_starts_with(s, t)`
    ///   - `s.ends_with(t)`  -> `lingo_str_ends_with(s, t)`
    ///
    /// NOTE: `len` returns *bytes*, not Unicode chars, to keep the runtime
    /// dependency-free.  The interp returns chars.  Plain ASCII matches;
    /// non-ASCII content diverges.  Pinned per test until we ship a real
    /// UTF-8 string runtime.
    fn gen_str_method(
        &mut self,
        recv_code: &str,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Result<(String, CType), LingoError> {
        let arg_str = |this: &mut Self, n: usize| -> Result<String, LingoError> {
            let a = &args[n];
            if a.name.is_some() {
                return Err(LingoError::new(
                    Stage::Resolve,
                    format!("C backend: `str.{}` takes positional args", method),
                    a.span,
                ));
            }
            let (c, t) = this.gen_expr(&a.value)?;
            if t != CType::Str {
                return Err(LingoError::new(
                    Stage::Resolve,
                    format!("C backend: `str.{}` expects str arg, got `{}`", method, t.c_decl()),
                    a.span,
                ));
            }
            Ok(c)
        };
        match (method, args.len()) {
            ("len", 0) => Ok((format!("((int64_t)strlen({}))", recv_code), CType::I64)),
            ("contains", 1) => {
                let needle = arg_str(self, 0)?;
                Ok((format!("(strstr({}, {}) != NULL)", recv_code, needle), CType::Bool))
            }
            ("starts_with", 1) => {
                let needle = arg_str(self, 0)?;
                Ok((format!("lingo_str_starts_with({}, {})", recv_code, needle), CType::Bool))
            }
            ("ends_with", 1) => {
                let needle = arg_str(self, 0)?;
                Ok((format!("lingo_str_ends_with({}, {})", recv_code, needle), CType::Bool))
            }
            ("split", 1) => {
                // `s.split(sep)` returns `vec[str]`.  The runtime helper allocs
                // both the backing array of `const char*` and each piece.
                let sep = arg_str(self, 0)?;
                Ok((
                    format!("lingo_str_split({}, {})", recv_code, sep),
                    CType::Vec(Box::new(CType::Str)),
                ))
            }
            (m, n) => Err(LingoError::new(
                Stage::Resolve,
                format!("C backend: `str.{}` with {} arg(s) is not supported yet \
                         (have: len/0, contains/1, starts_with/1, ends_with/1)", m, n),
                span,
            )),
        }
    }

    /// Built-in methods on `vec[T]` for T ∈ {i64, f64, str}.  Subset for v0.1.14:
    ///   - `v.len()`     -> `(v).len`              : i64
    ///   - `v.get(i)`    -> `(v).data[i]`          : T   (no bounds check yet)
    ///   - `v.push/.pop/.set` not yet — need an allocator
    fn gen_vec_method(
        &mut self,
        recv_code: &str,
        recv_ty: &CType,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Result<(String, CType), LingoError> {
        let elem_ty = match recv_ty {
            CType::Vec(inner) => (**inner).clone(),
            _ => unreachable!("gen_vec_method called with non-vec receiver"),
        };
        match (method, args.len()) {
            ("len", 0) => Ok((format!("({}).len", recv_code), CType::I64)),
            ("get", 1) => {
                if args[0].name.is_some() {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "C backend: `vec.get` takes a positional index",
                        args[0].span,
                    ));
                }
                let (i_code, i_ty) = self.gen_expr(&args[0].value)?;
                if i_ty != CType::I64 {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("C backend: `vec.get` index must be i64, got `{}`", i_ty.c_decl()),
                        args[0].span,
                    ));
                }
                Ok((format!("({}).data[(size_t)({})]", recv_code, i_code), elem_ty))
            }
            (m, n) => Err(LingoError::new(
                Stage::Resolve,
                format!("C backend: `vec.{}` with {} arg(s) is not supported yet \
                         (have: len/0, get/1)", m, n),
                span,
            )),
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
                // Enum variant constructor: `Type.Variant(args)`
                if let ExprKind::Ident(id) = &receiver.kind {
                    if let Some(enum_decl) = self.enums.get(id).cloned() {
                        return self.gen_enum_ctor(id, &enum_decl, method, args, span);
                    }
                }
                // Built-in vec methods.  Only probe when the receiver could
                // plausibly *be* a vec value — never when it's a bare type
                // name like `Point` (that would be a static method call,
                // handled below, and `gen_expr` would fail with "not in
                // scope" because struct/enum names aren't bound as values).
                let receiver_is_type_name = matches!(&receiver.kind, ExprKind::Ident(id)
                    if self.structs.contains_key(id) || self.enums.contains_key(id));
                if !receiver_is_type_name {
                    let probe = self.gen_expr(receiver)?;
                    if matches!(probe.1, CType::Vec(_)) {
                        return self.gen_vec_method(&probe.0, &probe.1, method, args, span);
                    }
                    if probe.1 == CType::Str {
                        return self.gen_str_method(&probe.0, method, args, span);
                    }
                }
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
        // Resolve a mix of positional + keyword args to a fully-positional
        // C-call list.  Lingo's rule (>2 params requires keywords) is enforced
        // at parse time, so by the time we're here we only need to (a) gather
        // names if present and (b) reject collisions / unknown names.
        let mut parts: Vec<String> = Vec::with_capacity(total);
        let self_count = if prepend_self_code.is_some() { 1 } else { 0 };
        if let Some(s) = prepend_self_code {
            parts.push(s);
        }
        let param_names = self.fn_param_names.get(&mangled).cloned().unwrap_or_default();
        // For static methods (`Type.method`) param_names already starts after `self`.
        // For instance methods we registered `self` as param[0], so skip it here.
        let param_name_slice: &[String] = if !param_names.is_empty()
            && self_count == 1
            && param_names[0] == "self"
        {
            &param_names[1..]
        } else {
            &param_names[..]
        };
        let expected = param_name_slice.len();
        // Fill from positional args first.
        let mut chosen: Vec<Option<String>> = vec![None; expected];
        let mut next_positional = 0usize;
        for a in args {
            if let Some(name) = &a.name {
                let Some(idx) = param_name_slice.iter().position(|p| p == name) else {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("`{}` has no parameter `{}`", mangled, name),
                        a.span,
                    ));
                };
                if chosen[idx].is_some() {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("parameter `{}` set twice in call to `{}`", name, mangled),
                        a.span,
                    ));
                }
                let (code, _) = self.gen_expr(&a.value)?;
                chosen[idx] = Some(code);
            } else {
                if next_positional >= expected {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        format!("too many positional args in call to `{}`", mangled),
                        a.span,
                    ));
                }
                if chosen[next_positional].is_some() {
                    return Err(LingoError::new(
                        Stage::Resolve,
                        "positional arg follows a keyword arg for the same slot",
                        a.span,
                    ));
                }
                let (code, _) = self.gen_expr(&a.value)?;
                chosen[next_positional] = Some(code);
                next_positional += 1;
            }
        }
        for (i, slot) in chosen.into_iter().enumerate() {
            let code = slot.ok_or_else(|| LingoError::new(
                Stage::Resolve,
                format!("missing arg `{}` in call to `{}`", param_name_slice[i], mangled),
                span,
            ))?;
            parts.push(code);
        }
        Ok((format!("{}({})", mangled, parts.join(", ")), ret))
    }
}

/// printf format specifier we use when debug-printing a value inside a struct
/// or enum payload — same idea as `CType::printf_fmt` but always quoting strings
/// (we want `Pt{name="foo"}`, not `Pt{name=foo}`).
fn debug_fmt_for(t: &CType) -> String {
    match t {
        CType::I64 => "%\" PRId64 \"".into(),
        CType::U64 => "%\" PRIu64 \"".into(),
        CType::F64 => "%g".into(),
        CType::Bool => "%s".into(),
        CType::Str => "\\\"%s\\\"".into(),
        CType::Void => "".into(),
        CType::Struct(_) => "<struct>".into(),
        CType::Enum(_) => "<enum>".into(),
        CType::Vec(_) => "<vec>".into(),
    }
}

fn debug_val_for(t: &CType, code: &str) -> String {
    match t {
        CType::Bool => format!("(({}) ? \"true\" : \"false\")", code),
        _ => code.to_string(),
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

/* === str runtime (v0.1.13) ===
 * Tiny, deps-free, *leaking* string helpers.  Buffers are malloc'd and never
 * freed; once we ship an allocator + `defer`, these will route through it
 * instead.  Until then, lingo programs leak proportional to their string
 * activity, which is fine for batch programs and well-known to users.
 */
__attribute__((unused))
static const char* lingo_str_concat(const char* a, const char* b) {
    size_t la = strlen(a), lb = strlen(b);
    char* out = (char*)malloc(la + lb + 1);
    if (!out) { fprintf(stderr, \"lingo: out of memory in str_concat\\n\"); exit(1); }
    memcpy(out, a, la);
    memcpy(out + la, b, lb);
    out[la + lb] = '\\0';
    return out;
}

__attribute__((unused))
static bool lingo_str_starts_with(const char* s, const char* prefix) {
    size_t lp = strlen(prefix);
    return strncmp(s, prefix, lp) == 0;
}

__attribute__((unused))
static bool lingo_str_ends_with(const char* s, const char* suffix) {
    size_t ls = strlen(s), lsuf = strlen(suffix);
    if (lsuf > ls) return false;
    return memcmp(s + (ls - lsuf), suffix, lsuf) == 0;
}

/* Split `s` by non-empty `sep` and return a `lingo_vec_str_t` of malloc'd
 * pieces.  Two passes: first counts, second copies.  Empty `sep` is rejected
 * (interp splits by codepoint there; we'll add that once we have UTF-8). */
__attribute__((unused))
static lingo_vec_str_t lingo_str_split(const char* s, const char* sep) {
    size_t sep_len = strlen(sep);
    if (sep_len == 0) {
        fprintf(stderr, \"lingo: str.split: empty separator not supported yet\\n\");
        exit(1);
    }
    size_t count = 1;
    const char* p = s;
    while ((p = strstr(p, sep)) != NULL) { count++; p += sep_len; }
    const char** arr = (const char**)malloc(count * sizeof(const char*));
    if (!arr) { fprintf(stderr, \"lingo: oom in str_split\\n\"); exit(1); }
    const char* start = s;
    size_t i = 0;
    while (1) {
        const char* end = strstr(start, sep);
        size_t len = end ? (size_t)(end - start) : strlen(start);
        char* piece = (char*)malloc(len + 1);
        if (!piece) { fprintf(stderr, \"lingo: oom in str_split\\n\"); exit(1); }
        memcpy(piece, start, len);
        piece[len] = '\\0';
        arr[i++] = piece;
        if (!end) break;
        start = end + sep_len;
    }
    return (lingo_vec_str_t){ .data = arr, .len = (int64_t)count };
}

/* Two-pass snprintf into a fresh heap buffer.  Returned `const char*` leaks
 * on purpose (see note above).  Used by the f-string lowering. */
__attribute__((unused))
__attribute__((format(printf, 1, 2)))
static const char* lingo_fmt_alloc(const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    va_list ap2;
    va_copy(ap2, ap);
    int n = vsnprintf(NULL, 0, fmt, ap);
    va_end(ap);
    if (n < 0) { fprintf(stderr, \"lingo: vsnprintf failed in fmt_alloc\\n\"); exit(1); }
    char* out = (char*)malloc((size_t)n + 1);
    if (!out) { fprintf(stderr, \"lingo: out of memory in fmt_alloc\\n\"); exit(1); }
    vsnprintf(out, (size_t)n + 1, fmt, ap2);
    va_end(ap2);
    return out;
}

";
    // Insert helper right before the protos section.
    // (Protos always start after the three #include lines + blank.)
    // Insertion point is right after the prelude block.  The last include
    // we emit is <math.h>; if that changes, update this marker too.
    // Marker must match the *exact* trailing chunk of the prelude in `gen_program`.
    // If you add/reorder #includes there, update this string too.
    // Marker is the chunk at the *end* of the prelude block, right after which
    // the helper splice goes — and crucially, the splice goes *after* the
    // `lingo_vec_<T>_t` typedefs (because the helpers reference them).
    let marker = "typedef struct { const int64_t* data; int64_t len; } lingo_vec_i64_t;\ntypedef struct { const double*  data; int64_t len; } lingo_vec_f64_t;\ntypedef struct { const char**   data; int64_t len; } lingo_vec_str_t;\n\n";
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
