//! CLI entry point.
//!
//! Usage:
//!     lingo path/to/file.lingo        # run the file (tree-walking interpreter)
//!     lingo --tokens path/file.lingo  # dump the token stream
//!     lingo --ast    path/file.lingo  # dump the parsed AST
//!     lingo --version                 # print version
//!
//! Anything else (build, lsp, fmt, test, repl) is a v0.2+ feature.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: lingo [--tokens|--ast] <file.lingo>");
        return ExitCode::from(2);
    }
    if args[1] == "--version" {
        println!("lingo {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let (mode, path) = match args[1].as_str() {
        "--tokens" => {
            if args.len() < 3 {
                eprintln!("--tokens needs a file");
                return ExitCode::from(2);
            }
            (Mode::Tokens, args[2].clone())
        }
        "--ast" => {
            if args.len() < 3 {
                eprintln!("--ast needs a file");
                return ExitCode::from(2);
            }
            (Mode::Ast, args[2].clone())
        }
        _ => (Mode::Run, args[1].clone()),
    };

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", path, e);
            return ExitCode::from(2);
        }
    };

    match mode {
        Mode::Run => match lingoc::run(&source, &path) {
            Ok(_) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        Mode::Tokens => match lingoc::lexer::lex(&source) {
            Ok(toks) => {
                for t in toks {
                    println!("{:>6}..{:<6}  {:?}", t.span.start, t.span.end, t.tok);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e.render(&source, &path));
                ExitCode::FAILURE
            }
        },
        Mode::Ast => {
            let tokens = match lingoc::lexer::lex(&source) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{}", e.render(&source, &path));
                    return ExitCode::FAILURE;
                }
            };
            match lingoc::parser::parse(tokens) {
                Ok(p) => {
                    println!("{:#?}", p);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.render(&source, &path));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

enum Mode {
    Run,
    Tokens,
    Ast,
}
