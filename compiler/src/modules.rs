//! Module resolution (v0.3.0).
//!
//! Reads `import foo.bar` declarations, walks the filesystem to load every
//! transitively-reachable `.lingo` file, detects cycles, and *flattens*
//! the whole thing into one big [`Program`] before handing off to the
//! interpreter or the C backend.
//!
//! Design notes
//! ============
//!
//! - **Resolver is the only place that touches the filesystem.**  The parser
//!   stays string-in / AST-out; the interpreter and the C backend never
//!   see a path.  This keeps each layer testable in isolation.
//!
//! - **Flattening, not separate compilation units.**  After the resolver
//!   runs there is exactly one [`Program`].  Every top-level declaration
//!   in every non-entry module gets renamed with a per-module prefix
//!   (`lm{i}__`).  Every `Ident(x)` / `TypeRef { name: x }` inside that
//!   module that refers to one of *that module's* top-level names gets
//!   the same prefix applied.  Every `alias.name` access in any module
//!   gets rewritten to the bare prefixed name in the target module.
//!
//! - **The entry module keeps its original names**, so `fn main()` is
//!   still callable by both backends without any extra wiring, and so
//!   the generated C is byte-identical to v0.2.7 for any single-file
//!   program.
//!
//! - **Name collisions** between (a) two top-level decls inside the entry
//!   module or (b) an entry-module decl and any prefixed import are
//!   impossible by construction.  Inside a non-entry module they're
//!   caught later by the interp / codegen duplicate-symbol checks, the
//!   same as for single-file programs.
//!
//! Limitations (deliberately deferred past v0.3.0)
//! -----------------------------------------------
//! - No cross-module type references in `TypeRef` positions
//!   (`fn f() -> bar.Point` is a parse error today).  Workaround: write
//!   a function in `bar` that returns a value of the type, and call
//!   it from the entry module.
//! - No cross-module struct literals (`bar.Point{...}`) — the parser
//!   doesn't currently produce a `StructLit` from a `Field` expression.
//! - No re-exports, no privacy, no `import *`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::error::{LingoError, Span, Stage};
use crate::{lexer, parser};

/// One loaded `.lingo` file.  Held only during resolution; the public
/// output of this module is a flattened [`Program`].
struct Module {
    /// Canonical filesystem path (after `canonicalize`).  Used as the
    /// graph key.
    canonical: PathBuf,
    /// Source text — kept so error rendering can point at it later.
    source: String,
    /// Parsed AST for this file (still contains `Item::Import` items).
    program: Program,
    /// `lm0__`, `lm1__`, … — entry module gets `""`.
    prefix: String,
    /// alias-as-written → canonical path of the imported module.
    imports: HashMap<String, PathBuf>,
    /// Top-level names declared *in this file* (pre-prefix).  Used by the
    /// rewrite pass to decide which `Ident(x)` / `TypeRef { name: x }`
    /// occurrences refer to one of this module's own decls (and thus
    /// need the prefix).
    own_names: HashSet<String>,
}

/// The result of resolving an entry file plus everything it transitively
/// imports.  Carries the original sources keyed by filename so error
/// rendering at later stages can still point at the right file.
pub struct ResolvedProgram {
    pub program: Program,
    pub sources: HashMap<String, String>,
    /// The filename that owns the entry point (used for error rendering
    /// of issues that don't carry a clear source themselves).
    pub entry_filename: String,
}

/// Top-level entry point.  Reads `entry_path` from disk, walks every
/// `import`, and returns the flattened program ready to feed to the
/// interpreter or the C backend.
///
/// `entry_path` may be relative — it's resolved against the current
/// working directory, same as `lingo file.lingo` already does.
pub fn resolve_from_path(entry_path: &str) -> Result<ResolvedProgram, String> {
    let entry_abs = std::fs::canonicalize(entry_path)
        .map_err(|e| format!("cannot open `{}`: {}", entry_path, e))?;
    let entry_source = std::fs::read_to_string(&entry_abs)
        .map_err(|e| format!("cannot read `{}`: {}", entry_path, e))?;
    resolve(entry_path.to_string(), entry_abs, entry_source)
}

/// Same as [`resolve_from_path`] but the entry file's bytes are passed
/// in directly.  Used by the tests and by callers that already have
/// the entry source in memory.
pub fn resolve(
    entry_display: String,
    entry_abs: PathBuf,
    entry_source: String,
) -> Result<ResolvedProgram, String> {
    let mut modules: HashMap<PathBuf, Module> = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    sources.insert(entry_display.clone(), entry_source.clone());

    // BFS load.  `pending` is (canonical_path, display_path, source_or_None).
    // We carry the source for the entry up front; everything else we
    // read from disk on first sight.
    let mut to_load: Vec<(PathBuf, String, Option<String>)> =
        vec![(entry_abs.clone(), entry_display.clone(), Some(entry_source))];
    let mut next_prefix_id: usize = 0;

    while let Some((path, display, maybe_src)) = to_load.pop() {
        if modules.contains_key(&path) {
            continue;
        }
        let src = match maybe_src {
            Some(s) => s,
            None => std::fs::read_to_string(&path).map_err(|e| {
                format!("cannot read `{}`: {}", path.display(), e)
            })?,
        };
        sources.insert(display.clone(), src.clone());

        let toks = lexer::lex(&src).map_err(|e| e.render(&src, &display))?;
        let program = parser::parse(toks).map_err(|e| e.render(&src, &display))?;

        // Pick a prefix.  The entry file (the first thing we load) gets
        // no prefix, everything else gets `lm{i}__`.
        let prefix = if path == entry_abs {
            String::new()
        } else {
            let p = format!("lm{}__", next_prefix_id);
            next_prefix_id += 1;
            p
        };

        // Resolve this module's `import` items into a (alias -> canonical
        // path) map and enqueue each unloaded target.
        let mut imports: HashMap<String, PathBuf> = HashMap::new();
        let module_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        for item in &program.items {
            if let Item::Import(imp) = item {
                let alias = imp
                    .alias
                    .clone()
                    .unwrap_or_else(|| imp.path.last().cloned().unwrap_or_default());
                if imports.contains_key(&alias) {
                    return Err(
                        LingoError::new(
                            Stage::Resolve,
                            format!("duplicate import alias `{}` in `{}`", alias, display),
                            imp.span,
                        )
                        .render(&src, &display),
                    );
                }
                let mut target = module_dir.clone();
                for (i, seg) in imp.path.iter().enumerate() {
                    if i + 1 == imp.path.len() {
                        target.push(format!("{}.lingo", seg));
                    } else {
                        target.push(seg);
                    }
                }
                let target_canonical = std::fs::canonicalize(&target).map_err(|_| {
                    LingoError::new(
                        Stage::Resolve,
                        format!(
                            "cannot resolve `import {}`: file `{}` not found",
                            imp.path.join("."),
                            target.display()
                        ),
                        imp.span,
                    )
                    .render(&src, &display)
                })?;
                imports.insert(alias, target_canonical.clone());
                if !modules.contains_key(&target_canonical) {
                    let target_display = target.to_string_lossy().into_owned();
                    to_load.push((target_canonical, target_display, None));
                }
            }
        }

        // Top-level names declared in this file (pre-prefix).
        let own_names = collect_own_names(&program);

        modules.insert(
            path.clone(),
            Module {
                canonical: path,
                source: src,
                program,
                prefix,
                imports,
                own_names,
            },
        );
    }

    // Cycle detection — a topological check over the import graph.
    detect_cycles(&modules, &entry_abs)?;

    // Build (canonical_path -> prefix) so the rewrite pass can map an
    // alias's target module to its mangling prefix without keeping the
    // whole module map around.
    let prefix_by_canonical: HashMap<PathBuf, String> = modules
        .values()
        .map(|m| (m.canonical.clone(), m.prefix.clone()))
        .collect();

    // Rewrite + flatten.  We pull each module out of the map, rewrite its
    // items, and push them into one shared item list.  Entry module
    // goes first so its `fn main()` (if any) is registered before
    // anything else — matches v0.2.x ordering for single-file programs.
    let mut flat_items: Vec<Item> = Vec::new();
    let mut module_list: Vec<Module> = modules.into_values().collect();
    module_list.sort_by(|a, b| {
        // Entry module first (empty prefix), then by prefix for
        // determinism so the generated C is reproducible run-to-run.
        match (a.prefix.is_empty(), b.prefix.is_empty()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.prefix.cmp(&b.prefix),
        }
    });

    for module in &module_list {
        rewrite_into(module, &prefix_by_canonical, &mut flat_items)
            .map_err(|e| e.render(&module.source, &module.canonical.to_string_lossy()))?;
    }

    // Re-bucket items so that, across modules, every type declaration
    // comes before every constant, which comes before every function /
    // impl block.  The C backend (codegen_c::emit) processes items in
    // source order during its "pass 3" body emission — a function in
    // the entry module that references `math.PI` would otherwise see
    // `lm0__PI` as undefined because the const is declared further
    // down in `math.lingo`.  Within a category we preserve the
    // resolver's deterministic order (entry first, then modules sorted
    // by prefix).  Single-file programs are unaffected: source order
    // for them already has types-then-consts-then-fns and this pass
    // is a stable no-op.
    let mut types: Vec<Item> = Vec::new();
    let mut consts: Vec<Item> = Vec::new();
    let mut rest: Vec<Item> = Vec::new();
    for item in flat_items {
        match &item {
            Item::Struct(_) | Item::Enum(_) | Item::Trait(_) => types.push(item),
            Item::Const(_) => consts.push(item),
            _ => rest.push(item),
        }
    }
    let mut reordered = Vec::with_capacity(types.len() + consts.len() + rest.len());
    reordered.append(&mut types);
    reordered.append(&mut consts);
    reordered.append(&mut rest);

    Ok(ResolvedProgram {
        program: Program { items: reordered },
        sources,
        entry_filename: entry_display,
    })
}

/// Collect the names declared at the top of one file (functions, consts,
/// structs, enums, traits).  Used by the rewrite pass to decide which
/// `Ident(x)` / `TypeRef { name: x }` references inside this file refer
/// to its own decls.
fn collect_own_names(program: &Program) -> HashSet<String> {
    let mut s = HashSet::new();
    for item in &program.items {
        match item {
            Item::Fn(f) => {
                s.insert(f.name.clone());
            }
            Item::Const(c) => {
                s.insert(c.name.clone());
            }
            Item::Struct(st) => {
                s.insert(st.name.clone());
            }
            Item::Enum(e) => {
                s.insert(e.name.clone());
            }
            Item::Trait(t) => {
                s.insert(t.name.clone());
            }
            Item::Impl(_) | Item::ImplTrait(_) | Item::Import(_) => {}
        }
    }
    s
}

/// Walks the import graph from the entry module looking for a back edge.
/// Reports the smallest cycle as a chain of canonical paths.
fn detect_cycles(
    modules: &HashMap<PathBuf, Module>,
    entry: &Path,
) -> Result<(), String> {
    // Iterative DFS with an explicit stack so we can build a readable
    // cycle path on detection.
    fn dfs(
        modules: &HashMap<PathBuf, Module>,
        current: &Path,
        stack: &mut Vec<PathBuf>,
        visited: &mut HashSet<PathBuf>,
    ) -> Result<(), Vec<PathBuf>> {
        if let Some(pos) = stack.iter().position(|p| p == current) {
            let mut cycle = stack[pos..].to_vec();
            cycle.push(current.to_path_buf());
            return Err(cycle);
        }
        if visited.contains(current) {
            return Ok(());
        }
        stack.push(current.to_path_buf());
        if let Some(m) = modules.get(current) {
            for target in m.imports.values() {
                dfs(modules, target, stack, visited)?;
            }
        }
        stack.pop();
        visited.insert(current.to_path_buf());
        Ok(())
    }

    let mut stack = Vec::new();
    let mut visited = HashSet::new();
    if let Err(cycle) = dfs(modules, entry, &mut stack, &mut visited) {
        let chain = cycle
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("cyclic import: {}", chain));
    }
    Ok(())
}

/// Rewrite one module's items in place and append them to `out`.
fn rewrite_into(
    module: &Module,
    prefix_by_canonical: &HashMap<PathBuf, String>,
    out: &mut Vec<Item>,
) -> Result<(), LingoError> {
    let ctx = RewriteCtx {
        prefix: module.prefix.clone(),
        own_names: &module.own_names,
        imports: &module.imports,
        prefix_by_canonical,
        errors: std::cell::RefCell::new(Vec::new()),
    };
    for item in &module.program.items {
        match item {
            Item::Import(_) => { /* swallowed by the resolver */ }
            Item::Fn(f) => out.push(Item::Fn(ctx.rewrite_fn(f.clone())?)),
            Item::Const(c) => {
                let mut c2 = c.clone();
                c2.name = ctx.prefix_name(&c2.name);
                c2.ty = c2.ty.map(|t| ctx.rewrite_typeref(t));
                c2.value = ctx.rewrite_expr(c2.value)?;
                out.push(Item::Const(c2));
            }
            Item::Struct(s) => {
                let mut s2 = s.clone();
                s2.name = ctx.prefix_name(&s2.name);
                for fd in &mut s2.fields {
                    fd.ty = ctx.rewrite_typeref(fd.ty.clone());
                }
                out.push(Item::Struct(s2));
            }
            Item::Enum(e) => {
                let mut e2 = e.clone();
                e2.name = ctx.prefix_name(&e2.name);
                for v in &mut e2.variants {
                    for ty in &mut v.payload {
                        *ty = ctx.rewrite_typeref(ty.clone());
                    }
                }
                out.push(Item::Enum(e2));
            }
            Item::Trait(t) => {
                let mut t2 = t.clone();
                t2.name = ctx.prefix_name(&t2.name);
                let methods = std::mem::take(&mut t2.methods);
                t2.methods = methods
                    .into_iter()
                    .map(|m| {
                        let TraitMethod { decl, has_default } = m;
                        Ok::<_, LingoError>(TraitMethod {
                            decl: ctx.rewrite_fn(decl)?,
                            has_default,
                        })
                    })
                    .collect::<Result<_, _>>()?;
                out.push(Item::Trait(t2));
            }
            Item::Impl(b) => {
                let mut b2 = b.clone();
                // Note: `impl Target:` blocks attach methods to `Target`
                // even when `Target` isn't declared in this file (e.g. a
                // type defined in another module — disallowed today but
                // we still respect the local-only rename rule).
                b2.target = ctx.maybe_prefix_typename(&b2.target);
                let methods = std::mem::take(&mut b2.methods);
                b2.methods = methods
                    .into_iter()
                    .map(|m| ctx.rewrite_fn(m))
                    .collect::<Result<_, _>>()?;
                out.push(Item::Impl(b2));
            }
            Item::ImplTrait(b) => {
                let mut b2 = b.clone();
                // The trait name might be a user-defined trait (rename
                // if local to this module) or a built-in name like `From`
                // (leave it alone — built-in trait names are global).
                b2.trait_name = ctx.maybe_prefix_typename(&b2.trait_name);
                b2.target = ctx.maybe_prefix_typename(&b2.target);
                // trait_args is Vec<String> — each one is a type name.
                b2.trait_args = b2
                    .trait_args
                    .into_iter()
                    .map(|n| ctx.maybe_prefix_typename(&n))
                    .collect();
                let methods = std::mem::take(&mut b2.methods);
                b2.methods = methods
                    .into_iter()
                    .map(|m| ctx.rewrite_fn(m))
                    .collect::<Result<_, _>>()?;
                out.push(Item::ImplTrait(b2));
            }
        }
    }
    // Surface the first error gathered during the rewrite (e.g. a
    // dotted type ref whose alias isn't an import in this module).
    // We stop at the first one so the diagnostic stream stays
    // focused on the root cause rather than the cascade.
    if let Some(first) = ctx.errors.into_inner().into_iter().next() {
        return Err(first);
    }
    Ok(())
}

struct RewriteCtx<'a> {
    prefix: String,
    own_names: &'a HashSet<String>,
    imports: &'a HashMap<String, PathBuf>,
    prefix_by_canonical: &'a HashMap<PathBuf, String>,
    // Errors gathered during the otherwise-infallible rewrite (e.g. a
    // dotted type ref whose alias isn't an import).  Threaded through
    // `RefCell` so the existing rewrite signatures keep returning
    // plain values; the resolver checks this after the pass and
    // surfaces the first error.
    errors: std::cell::RefCell<Vec<LingoError>>,
}

impl<'a> RewriteCtx<'a> {
    /// Add this module's prefix unconditionally — used when emitting a
    /// new top-level name (`fn foo` → `lm0__foo`).
    fn prefix_name(&self, name: &str) -> String {
        format!("{}{}", self.prefix, name)
    }

    /// Add this module's prefix *only if* `name` is one of this module's
    /// own top-level names — used when rewriting a reference that may
    /// or may not point at a local decl.
    ///
    /// v0.3.1: `name` may also be a dotted cross-module reference like
    /// `bar.Point`.  In that case `bar` is looked up in this module's
    /// imports, the imported module's prefix is fetched, and the
    /// resolved flat name `lm{i}__Point` is returned.  The interp and
    /// the C backend never see a dotted name — by the time they run,
    /// every reference is a regular flat ident.
    fn maybe_prefix_typename(&self, name: &str) -> String {
        if let Some((alias, last)) = name.split_once('.') {
            if let Some(target) = self.imports.get(alias) {
                if let Some(prefix) = self.prefix_by_canonical.get(target) {
                    return format!("{prefix}{last}");
                }
            }
            // Alias isn't an import in this module — record an error
            // here so the resolver fails clean instead of leaving a
            // dotted name to confuse the downstream backends.  Use the
            // dummy span (0..0) because TypeRef-style call sites don't
            // thread a span into this helper; the diagnostic body is
            // unambiguous on its own.
            self.errors.borrow_mut().push(LingoError::new(
                Stage::Parse,
                format!(
                    "cannot resolve `{name}`: `{alias}` is not an import in this module"
                ),
                Span::new(0, 0),
            ));
            return name.to_string();
        }
        if self.own_names.contains(name) {
            self.prefix_name(name)
        } else {
            name.to_string()
        }
    }

    fn rewrite_typeref(&self, mut ty: TypeRef) -> TypeRef {
        ty.name = self.maybe_prefix_typename(&ty.name);
        ty.type_args = ty.type_args.into_iter().map(|t| self.rewrite_typeref(t)).collect();
        ty
    }

    fn rewrite_fn(&self, mut f: FnDecl) -> Result<FnDecl, LingoError> {
        f.name = self.prefix_name(&f.name);
        for p in &mut f.params {
            p.ty = self.rewrite_typeref(p.ty.clone());
        }
        if let Some(rt) = f.return_type.take() {
            f.return_type = Some(self.rewrite_typeref(rt));
        }
        if let Some(rs) = f.raises.take() {
            f.raises = Some(self.rewrite_typeref(rs));
        }
        f.body = self.rewrite_block(f.body)?;
        Ok(f)
    }

    fn rewrite_block(&self, mut b: Block) -> Result<Block, LingoError> {
        let stmts = std::mem::take(&mut b.stmts);
        b.stmts = stmts
            .into_iter()
            .map(|s| self.rewrite_stmt(s))
            .collect::<Result<_, _>>()?;
        Ok(b)
    }

    fn rewrite_stmt(&self, stmt: Stmt) -> Result<Stmt, LingoError> {
        Ok(match stmt {
            Stmt::Let { is_mut, name, ty, value, span } => Stmt::Let {
                is_mut,
                name,
                ty: ty.map(|t| self.rewrite_typeref(t)),
                value: self.rewrite_expr(value)?,
                span,
            },
            Stmt::Assign { target, value, span } => Stmt::Assign {
                target: self.rewrite_assign_target(target)?,
                value: self.rewrite_expr(value)?,
                span,
            },
            Stmt::Expr(e) => Stmt::Expr(self.rewrite_expr(e)?),
            Stmt::Return { value, span } => Stmt::Return {
                value: value.map(|e| self.rewrite_expr(e)).transpose()?,
                span,
            },
            Stmt::Raise { value, span } => Stmt::Raise {
                value: self.rewrite_expr(value)?,
                span,
            },
            Stmt::If { arms, else_block, span } => Stmt::If {
                arms: arms
                    .into_iter()
                    .map(|(cond, body)| {
                        Ok::<_, LingoError>((self.rewrite_expr(cond)?, self.rewrite_block(body)?))
                    })
                    .collect::<Result<_, _>>()?,
                else_block: else_block.map(|b| self.rewrite_block(b)).transpose()?,
                span,
            },
            Stmt::For { var, iter, body, span } => Stmt::For {
                var,
                iter: self.rewrite_expr(iter)?,
                body: self.rewrite_block(body)?,
                span,
            },
            Stmt::Match { scrutinee, arms, span } => Stmt::Match {
                scrutinee: self.rewrite_expr(scrutinee)?,
                arms: arms
                    .into_iter()
                    .map(|arm| {
                        Ok::<_, LingoError>(MatchArm {
                            pattern: self.rewrite_pattern(arm.pattern)?,
                            body: self.rewrite_block(arm.body)?,
                            span: arm.span,
                        })
                    })
                    .collect::<Result<_, _>>()?,
                span,
            },
            Stmt::Break(s) => Stmt::Break(s),
            Stmt::Continue(s) => Stmt::Continue(s),
        })
    }

    fn rewrite_assign_target(&self, t: AssignTarget) -> Result<AssignTarget, LingoError> {
        Ok(match t {
            AssignTarget::Name(n) => AssignTarget::Name(n),
            AssignTarget::Field(recv, name) => {
                AssignTarget::Field(Box::new(self.rewrite_expr(*recv)?), name)
            }
        })
    }

    fn rewrite_pattern(&self, p: Pattern) -> Result<Pattern, LingoError> {
        Ok(match p {
            Pattern::Wildcard(s) => Pattern::Wildcard(s),
            Pattern::Bind(n, s) => Pattern::Bind(n, s),
            Pattern::Literal(lit, s) => Pattern::Literal(lit, s),
            Pattern::Variant { type_name, variant, sub, span } => Pattern::Variant {
                // `match x: Foo.A(p): ...` — `type_name` is `Foo`, which
                // we rename if it's a local enum.  Bare-variant patterns
                // (`some(x)`, `none`) have `type_name == None` and are
                // left untouched.
                type_name: type_name.map(|t| self.maybe_prefix_typename(&t)),
                variant,
                sub: sub
                    .into_iter()
                    .map(|p| self.rewrite_pattern(p))
                    .collect::<Result<_, _>>()?,
                span,
            },
        })
    }

    fn rewrite_expr(&self, mut e: Expr) -> Result<Expr, LingoError> {
        e.kind = self.rewrite_exprkind(e.kind, e.span)?;
        Ok(e)
    }

    fn rewrite_exprkind(&self, k: ExprKind, span: Span) -> Result<ExprKind, LingoError> {
        Ok(match k {
            ExprKind::Ident(name) => {
                // Bare identifier: only rewrite if it matches a local
                // top-level decl.  Otherwise it could be a parameter,
                // a let-binding, a builtin like `print`, or an unknown
                // (caught later by interp/codegen).
                if self.own_names.contains(&name) {
                    ExprKind::Ident(self.prefix_name(&name))
                } else {
                    ExprKind::Ident(name)
                }
            }
            ExprKind::Field(recv, name) => {
                // The one cross-module hook: `alias.thing`.  If `alias`
                // is one of this file's imports we rewrite the whole
                // `Field(Ident(alias), thing)` to `Ident(target_prefix +
                // thing)`.  Otherwise it's a normal field/method access
                // and we leave it alone (recursing into the receiver).
                if let ExprKind::Ident(alias) = &recv.kind {
                    if !self.own_names.contains(alias) {
                        if let Some(target) = self.imports.get(alias) {
                            let target_prefix =
                                self.prefix_by_canonical.get(target).cloned().unwrap_or_default();
                            return Ok(ExprKind::Ident(format!("{}{}", target_prefix, name)));
                        }
                    }
                }
                ExprKind::Field(Box::new(self.rewrite_expr(*recv)?), name)
            }
            ExprKind::StructLit { name, fields } => ExprKind::StructLit {
                name: self.maybe_prefix_typename(&name),
                fields: fields
                    .into_iter()
                    .map(|(n, v)| Ok::<_, LingoError>((n, self.rewrite_expr(v)?)))
                    .collect::<Result<_, _>>()?,
            },
            ExprKind::Call(callee, args) => ExprKind::Call(
                Box::new(self.rewrite_expr(*callee)?),
                args.into_iter()
                    .map(|a| {
                        Ok::<_, LingoError>(Arg {
                            name: a.name,
                            value: self.rewrite_expr(a.value)?,
                            span: a.span,
                        })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            ExprKind::Unary(op, x) => ExprKind::Unary(op, Box::new(self.rewrite_expr(*x)?)),
            ExprKind::Binary(op, a, b) => ExprKind::Binary(
                op,
                Box::new(self.rewrite_expr(*a)?),
                Box::new(self.rewrite_expr(*b)?),
            ),
            ExprKind::Range(a, b) => ExprKind::Range(
                Box::new(self.rewrite_expr(*a)?),
                Box::new(self.rewrite_expr(*b)?),
            ),
            ExprKind::VecLit(items) => ExprKind::VecLit(
                items.into_iter().map(|e| self.rewrite_expr(e)).collect::<Result<_, _>>()?,
            ),
            ExprKind::MapLit(pairs) => ExprKind::MapLit(
                pairs
                    .into_iter()
                    .map(|(k, v)| Ok::<_, LingoError>((self.rewrite_expr(k)?, self.rewrite_expr(v)?)))
                    .collect::<Result<_, _>>()?,
            ),
            ExprKind::FString(parts) => ExprKind::FString(
                parts
                    .into_iter()
                    .map(|p| {
                        Ok::<_, LingoError>(match p {
                            FStringPart::Lit(s) => FStringPart::Lit(s),
                            FStringPart::Expr(e) => FStringPart::Expr(self.rewrite_expr(e)?),
                        })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            ExprKind::Try { inner, fallback } => ExprKind::Try {
                inner: Box::new(self.rewrite_expr(*inner)?),
                fallback: fallback
                    .map(|f| self.rewrite_expr(*f).map(Box::new))
                    .transpose()?,
            },
            // Leaves — no recursion needed.
            other @ (ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None_
            | ExprKind::Self_
            | ExprKind::PrintBuiltin
            | ExprKind::Forever) => {
                let _ = span;
                other
            }
        })
    }
}
