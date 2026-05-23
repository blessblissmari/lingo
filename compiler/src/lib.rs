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
pub mod parser;

pub use error::{LingoError, Stage};

/// Run a lingo source string as if it were the whole program.
/// `filename` is only used for error messages.
pub fn run(source: &str, filename: &str) -> Result<(), String> {
    run_with_argv(source, filename, Vec::new())
}

/// Same as `run` but also exposes `argv` to lingo code via the `args()` builtin.
pub fn run_with_argv(source: &str, filename: &str, argv: Vec<String>) -> Result<(), String> {
    let tokens = lexer::lex(source).map_err(|e| e.render(source, filename))?;
    let program = parser::parse(tokens).map_err(|e| e.render(source, filename))?;
    let mut interp = interp::Interp::new().with_argv(argv);
    interp.run_program(&program).map_err(|e| e.render(source, filename))?;
    Ok(())
}

/// Lower a lingo program to a self-contained C source string.
/// (Subset of the language only — see `src/codegen_c.rs`.)
pub fn emit_c(source: &str, filename: &str) -> Result<String, String> {
    let tokens = lexer::lex(source).map_err(|e| e.render(source, filename))?;
    let program = parser::parse(tokens).map_err(|e| e.render(source, filename))?;
    codegen_c::emit(&program).map_err(|e| e.render(source, filename))
}
