//! Interactive REPL for lingo (v0.1.16).
//!
//! ## How it works
//!
//! The REPL holds *one* persistent `Interp` instance.  Every chunk you type
//! is parsed and routed by its shape:
//!
//!   * top-level items (`fn`, `struct`, `enum`, `impl`, `trait`, `const`) are
//!     registered into the interpreter's tables.  Re-definitions silently
//!     replace the previous version — the convenience of a REPL outweighs the
//!     strictness of file mode here.
//!
//!   * everything else (`let`, expressions, calls, `for`/`if`/...) is exec'd
//!     against a persistent root scope, so `let x = 5` survives between
//!     prompts.
//!
//! ## Multi-line input
//!
//! A chunk ends when we see an *empty* line *and* the next line isn't
//! indented — i.e. we treat empty lines inside an indented block as
//! continuations.  Or: just press Enter twice to submit.
//!
//! ## Status of `main`
//!
//! Defining `fn main()` works; `run main` calls it.  `main` itself does NOT
//! run automatically — the REPL is interactive, not a file driver.

use std::io::{self, BufRead, Write};

use crate::ast::{Item, Program, Stmt};
use crate::error::{LingoError, Span, Stage};
use crate::interp::Interp;
use crate::{lexer, parser};

/// Run the REPL on stdin/stdout.  Returns once stdin hits EOF.
pub fn run() -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "lingo {} — interactive REPL", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "type a function or statement; blank line submits; `:help` for tips, `:quit` or Ctrl-D to exit.")?;
    out.flush()?;

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    let mut interp = Interp::new();
    let mut buf: Vec<String> = Vec::new();

    // Print prompt helper.
    let prompt = |out: &mut io::StdoutLock<'_>, continuing: bool| -> io::Result<()> {
        write!(out, "{}", if continuing { "... " } else { ">>> " })?;
        out.flush()
    };

    prompt(&mut out, false)?;
    loop {
        let line = match lines.next() {
            Some(Ok(l)) => l,
            Some(Err(e)) => return Err(e),
            None => {
                writeln!(out)?;
                break;
            }
        };

        // Meta commands work only on a fresh line (not mid-chunk).
        if buf.is_empty() {
            let trimmed = line.trim();
            match trimmed {
                ":quit" | ":q" => break,
                ":help" | ":h" => {
                    writeln!(out, "  type a `fn`/`struct`/`enum`/`impl`/`trait`/`const` to add it to the session.")?;
                    writeln!(out, "  type any statement (`let x = 1`, `print(x)`, `for i in 0..3:`) to execute it.")?;
                    writeln!(out, "  indented continuation lines: press Enter once after each, blank line submits.")?;
                    writeln!(out, "  `:clear`  — wipe the session and start fresh")?;
                    writeln!(out, "  `:quit`   — exit (or Ctrl-D)")?;
                    out.flush()?;
                    prompt(&mut out, false)?;
                    continue;
                }
                ":clear" => {
                    interp = Interp::new();
                    writeln!(out, "session cleared.")?;
                    prompt(&mut out, false)?;
                    continue;
                }
                "" => {
                    prompt(&mut out, false)?;
                    continue;
                }
                _ => {}
            }
        }

        // Track whether we're inside a multi-line chunk.  A chunk ends when
        // the user hits an empty line — but only if the previous line wasn't
        // itself a block opener (`:` at end-of-line meaning "expect indented
        // body").  This matches what people expect from a Python-ish REPL.
        let is_blank = line.trim().is_empty();
        if is_blank && !buf.is_empty() {
            // submit
        } else {
            buf.push(line);
            let needs_more = chunk_wants_more(&buf);
            if needs_more {
                prompt(&mut out, true)?;
                continue;
            }
            // single-line chunks fall through immediately.
        }

        let source = buf.join("\n") + "\n";
        buf.clear();

        match dispatch(&mut interp, &source) {
            Ok(()) => {}
            Err(msg) => {
                eprintln!("{msg}");
            }
        }
        prompt(&mut out, false)?;
    }
    Ok(())
}

/// Heuristic: do we need more input lines before submitting?
///
/// Yes if any non-blank line ends with `:`, or if the most recent non-blank
/// line is indented (we're inside a body).  This keeps the REPL friendly for
/// `fn foo():` / `if ...:` / `for ...:` chunks where the body follows on
/// subsequent lines.
fn chunk_wants_more(buf: &[String]) -> bool {
    // Find the last non-blank line.
    let last = buf.iter().rev().find(|l| !l.trim().is_empty());
    let Some(last) = last else { return false };
    let trimmed = last.trim_end();
    if trimmed.ends_with(':') {
        return true;
    }
    // If buf has more than one line and the last line is indented, we're
    // probably mid-body — let the user finish before we try to parse.
    if buf.len() > 1 && last.starts_with([' ', '\t']) {
        return true;
    }
    false
}

/// Parse the chunk, then either register decls or exec it as statements.
fn dispatch(interp: &mut Interp, source: &str) -> Result<(), String> {
    // Try parsing as a program (item-level).
    let tokens = lexer::lex(source).map_err(|e| e.render(source, "<repl>"))?;
    // We always try the program parser first.  If it succeeds AND yields at
    // least one Item, we register; if it succeeds and is empty, we re-parse
    // as a wrapped statement.  If the program parse fails AND the chunk
    // doesn't look like a decl, we fall back to statement-wrap mode.
    let looks_like_decl = source
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| {
            let t = l.trim_start();
            t.starts_with("fn ")
                || t.starts_with("struct ")
                || t.starts_with("enum ")
                || t.starts_with("impl ")
                || t.starts_with("trait ")
                || t.starts_with("const ")
        })
        .unwrap_or(false);

    if looks_like_decl {
        let prog = parser::parse(tokens).map_err(|e| e.render(source, "<repl>"))?;
        interp
            .register_items(&prog, /* allow_replace */ true)
            .map_err(|e| e.render(source, "<repl>"))?;
        return Ok(());
    }

    // Statement mode: wrap the chunk in a tiny synthetic function so the
    // existing parser can handle it without inventing a "top-level stmt"
    // entry.  Then pull the statements back out and exec each one against
    // the interpreter's persistent root scope.
    let wrapped = wrap_as_fn(source);
    let toks = lexer::lex(&wrapped).map_err(|e| e.render(&wrapped, "<repl>"))?;
    let prog: Program = parser::parse(toks).map_err(|e| e.render(&wrapped, "<repl>"))?;

    // Expect exactly one synth fn item.
    let stmts: Vec<Stmt> = prog
        .items
        .iter()
        .find_map(|it| match it {
            Item::Fn(f) if f.name == "__repl_eval" => Some(f.body.stmts.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            LingoError::new(Stage::Resolve, "REPL: failed to wrap statement", Span::dummy())
                .render(&wrapped, "<repl>")
        })?;

    for s in stmts {
        interp
            .exec_top_stmt(&s)
            .map_err(|e| e.render(&wrapped, "<repl>"))?;
    }
    Ok(())
}

/// Wrap a raw chunk of statements inside `fn __repl_eval():` so the parser
/// (which only knows about top-level items) can handle it.
fn wrap_as_fn(source: &str) -> String {
    let mut out = String::from("fn __repl_eval():\n");
    for line in source.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    out
}
