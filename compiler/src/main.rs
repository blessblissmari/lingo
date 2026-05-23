//! CLI entry point.
//!
//! Usage:
//!     lingo path/to/file.lingo        # run the file (tree-walking interpreter)
//!     lingo build path/file.lingo     # lower to C, compile with gcc, emit native binary
//!     lingo emit-c path/file.lingo    # dump the generated C source to stdout
//!     lingo --tokens path/file.lingo  # dump the token stream
//!     lingo --ast    path/file.lingo  # dump the parsed AST
//!     lingo --version                 # print version
//!
//! Anything else (lsp, fmt, test, repl) is a v0.2+ feature.

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

    let (mode, path, prog_args) = match args[1].as_str() {
        "--tokens" => {
            if args.len() < 3 {
                eprintln!("--tokens needs a file");
                return ExitCode::from(2);
            }
            (Mode::Tokens, args[2].clone(), Vec::new())
        }
        "--ast" => {
            if args.len() < 3 {
                eprintln!("--ast needs a file");
                return ExitCode::from(2);
            }
            (Mode::Ast, args[2].clone(), Vec::new())
        }
        "build" => {
            if args.len() < 3 {
                eprintln!("build needs a file: lingo build foo.lingo");
                return ExitCode::from(2);
            }
            (Mode::Build, args[2].clone(), Vec::new())
        }
        "emit-c" => {
            if args.len() < 3 {
                eprintln!("emit-c needs a file: lingo emit-c foo.lingo");
                return ExitCode::from(2);
            }
            (Mode::EmitC, args[2].clone(), Vec::new())
        }
        // everything after the .lingo file becomes the program's `args()`.
        _ => (Mode::Run, args[1].clone(), args[2..].to_vec()),
    };

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", path, e);
            return ExitCode::from(2);
        }
    };

    match mode {
        Mode::EmitC => match lingoc::emit_c(&source, &path) {
            Ok(c) => {
                print!("{c}");
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("{msg}");
                ExitCode::FAILURE
            }
        },
        Mode::Build => {
            let c = match lingoc::emit_c(&source, &path) {
                Ok(c) => c,
                Err(msg) => {
                    eprintln!("{msg}");
                    return ExitCode::FAILURE;
                }
            };
            // derive output binary name from input file stem
            let stem = std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("a");
            let c_path = format!("{}.c", stem);
            if let Err(e) = std::fs::write(&c_path, &c) {
                eprintln!("error: cannot write {c_path}: {e}");
                return ExitCode::FAILURE;
            }
            let cc = std::env::var("LINGO_CC").unwrap_or_else(|_| "cc".to_string());
            let out_bin = stem.to_string();
            let status = std::process::Command::new(&cc)
                .arg("-O2").arg("-std=c99").arg("-Wall")
                .arg(&c_path)
                .arg("-o").arg(&out_bin)
                .arg("-lm") // libm for pow/sqrt/etc — only needed once we lower f64 ops
                .status();
            match status {
                Ok(s) if s.success() => {
                    eprintln!("compiled `{path}` -> `./{out_bin}` (via {cc})");
                    ExitCode::SUCCESS
                }
                Ok(s) => {
                    eprintln!("{cc} failed with exit code {:?}", s.code());
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("could not run {cc}: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Mode::Run => match lingoc::run_with_argv(&source, &path, prog_args) {
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
    Build,
    EmitC,
}
