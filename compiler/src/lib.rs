//! lingoc — the bootstrap compiler for the lingo language.
//!
//! Currently exposes a tree-walking interpreter only.  The path to a real
//! native compiler runs through this same crate:
//!
//!   v0.1.x: lexer + parser + interpreter   ← we are here
//!   v0.2.x: mid-ir (ssa) + llvm backend
//!   v0.3.x: stdlib
//!   v0.4.x: tooling (fmt, lsp, test)
//!   v1.0.x: self-host
//!
//! see ROADMAP.md and docs/DECISIONS.md in the repo root.

pub mod ast;
pub mod codegen_c;
pub mod error;
pub mod interp;
pub mod lexer;
pub mod modules;
pub mod parser;
pub mod repl;

pub use error::{LingoError, Stage};

/// Run a lingo source string as if it were the whole program.
/// `filename` is only used for error messages.
///
/// This is the "in-memory" entry point: nothing is read from disk and
/// `import` declarations are not resolved.  For real multi-file
/// programs, use [`run_path`].
pub fn run(source: &str, filename: &str) -> Result<(), String> {
    run_with_argv(source, filename, Vec::new())
}

/// Same as `run` but also exposes `argv` to lingo code via the `args()` builtin.
pub fn run_with_argv(source: &str, filename: &str, argv: Vec<String>) -> Result<(), String> {
    let tokens = lexer::lex(source).map_err(|e| e.render(source, filename))?;
    let program = parser::parse(tokens).map_err(|e| e.render(source, filename))?;
    if has_imports(&program) {
        return Err(format!(
            "{}: `import` requires a file path — use `lingo run file.lingo` (the CLI entry point already does this for you)",
            filename
        ));
    }
    let mut interp = interp::Interp::new().with_argv(argv);
    interp.run_program(&program).map_err(|e| e.render(source, filename))?;
    Ok(())
}

/// Run a lingo program from its entry file, resolving every `import`
/// transitively before evaluation.  This is what the `lingo` CLI uses.
pub fn run_path(entry_path: &str, argv: Vec<String>) -> Result<(), String> {
    let resolved = modules::resolve_from_path(entry_path)?;
    let entry_filename = resolved.entry_filename.clone();
    let entry_source = resolved.sources.get(&entry_filename).cloned().unwrap_or_default();
    let mut interp = interp::Interp::new().with_argv(argv);
    interp
        .run_program(&resolved.program)
        .map_err(|e| e.render(&entry_source, &entry_filename))?;
    Ok(())
}

/// Lower a lingo program to a self-contained C source string.
/// In-memory variant — no imports allowed.  Multi-file programs go
/// through [`emit_c_path`].
pub fn emit_c(source: &str, filename: &str) -> Result<String, String> {
    let tokens = lexer::lex(source).map_err(|e| e.render(source, filename))?;
    let program = parser::parse(tokens).map_err(|e| e.render(source, filename))?;
    if has_imports(&program) {
        return Err(format!(
            "{}: `import` requires a file path — use `lingo build file.lingo`",
            filename
        ));
    }
    codegen_c::emit(&program).map_err(|e| e.render(source, filename))
}

/// File-aware variant: resolves imports starting from `entry_path` and
/// emits one self-contained C source string.
pub fn emit_c_path(entry_path: &str) -> Result<String, String> {
    let resolved = modules::resolve_from_path(entry_path)?;
    let entry_filename = resolved.entry_filename.clone();
    let entry_source = resolved.sources.get(&entry_filename).cloned().unwrap_or_default();
    codegen_c::emit(&resolved.program).map_err(|e| e.render(&entry_source, &entry_filename))
}

fn has_imports(p: &ast::Program) -> bool {
    p.items.iter().any(|i| matches!(i, ast::Item::Import(_)))
}
