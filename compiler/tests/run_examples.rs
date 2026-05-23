//! Integration tests that exercise the lexer, parser, and interpreter
//! end-to-end against the bundled `examples/` programs.
//!
//! These tests assert *behaviour*, not implementation details:
//! given an example file, the interpreter must produce a specific
//! stdout.  When v0.2 swaps the interpreter for an LLVM-backed
//! compiler, these tests stay green.

use std::process::Command;

fn cargo_bin() -> String {
    env!("CARGO_BIN_EXE_lingo").to_string()
}

/// Compile a lingo example via the C backend and run the resulting binary.
/// Returns (stdout, stderr, exit_code) of the compiled program.
/// Skipped silently when no C compiler is available.
fn run_native(file: &str) -> Option<(String, String, i32)> {
    if which_cc().is_none() {
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("lingo_native_{}", file.replace('/', "_")));
    let _ = std::fs::remove_file(&tmp);
    // build
    let build = Command::new(cargo_bin())
        .arg("build")
        .arg(format!("examples/{file}"))
        .env("LINGO_OUT", tmp.to_string_lossy().to_string())
        .output()
        .expect("failed to invoke lingo build");
    if !build.status.success() {
        panic!(
            "lingo build failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
    }
    // `lingo build` writes the binary into the cwd; find it by stem.
    let stem = std::path::Path::new(file).file_stem().unwrap().to_string_lossy().to_string();
    let bin = std::path::Path::new(&stem).to_path_buf();
    if !bin.exists() {
        return None;
    }
    let run = Command::new(format!("./{}", stem)).output().ok()?;
    Some((
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
        run.status.code().unwrap_or(-1),
    ))
}

fn which_cc() -> Option<String> {
    for cc in &["cc", "gcc", "clang"] {
        if Command::new(cc).arg("--version").output().is_ok() {
            return Some(cc.to_string());
        }
    }
    None
}

fn run(file: &str) -> (String, String, i32) {
    run_with_args(file, &[])
}

fn run_with_args(file: &str, prog_args: &[&str]) -> (String, String, i32) {
    let mut cmd = Command::new(cargo_bin());
    cmd.arg(format!("examples/{file}"));
    for a in prog_args {
        cmd.arg(a);
    }
    let out = cmd.output().expect("failed to run lingo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// v0.1.28: the no-shadowing rule now extends to **for-loop variables**
/// and **match-arm binds**, in both backends.  Pre-v0.1.28, `let i = 0`
/// followed by `for i in 0..3:` (or a match arm `Some(x)` against an
/// outer `let x`) silently shadowed the outer name for the duration of
/// the loop / arm — disagreeing with DECISIONS.md's "no shadowing" rule.
#[test]
fn interp_rejects_for_var_shadow() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let path = std::env::temp_dir().join("lingo_shadow_for_interp.lingo");
    std::fs::write(&path, "fn main():\n    let i = 0\n    for i in 0..3:\n        print(i)\n").unwrap();
    let out = std::process::Command::new(bin).arg(&path).output().expect("run lingo");
    assert!(!out.status.success(), "interp should reject for-var shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`i` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn interp_rejects_match_bind_shadow() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let path = std::env::temp_dir().join("lingo_shadow_match_interp.lingo");
    let src = "enum Opt:\n    Some(int)\n    None\n\n\
               fn main():\n    let x = 1\n    match Opt.Some(42):\n        \
               Opt.Some(x):\n            print(x)\n        Opt.None:\n            print(0)\n";
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin).arg(&path).output().expect("run lingo");
    assert!(!out.status.success(), "interp should reject match-bind shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`x` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn c_backend_rejects_for_var_shadow() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let path = std::env::temp_dir().join("lingo_shadow_for_native.lingo");
    std::fs::write(&path, "fn main():\n    let i = 0\n    for i in 0..3:\n        print(i)\n").unwrap();
    let out = std::process::Command::new(bin).arg("build").arg(&path).output().expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject for-var shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`i` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn c_backend_rejects_match_bind_shadow() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let path = std::env::temp_dir().join("lingo_shadow_match_native.lingo");
    let src = "enum Opt:\n    Some(int)\n    None\n\n\
               fn main():\n    let x = 1\n    match Opt.Some(42):\n        \
               Opt.Some(x):\n            print(x)\n        Opt.None:\n            print(0)\n";
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin).arg("build").arg(&path).output().expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject match-bind shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`x` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
}

/// `_` is the "don't bind" sigil and must still be allowed everywhere,
/// even when an outer scope has a binding called `_` (which itself is
/// only possible because `_` is treated specially in `let` too).
#[test]
fn for_var_underscore_still_allowed() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let path = std::env::temp_dir().join("lingo_shadow_for_under.lingo");
    std::fs::write(&path, "fn main():\n    let mut n = 0\n    for _ in 0..3:\n        n = n + 1\n    print(n)\n").unwrap();
    let out = std::process::Command::new(bin).arg(&path).output().expect("run lingo");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3");
}

/// v0.1.27: the C backend should reject shadowing with the same
/// `resolve error:` diagnostic the interpreter uses, instead of letting
/// `cc` complain about a redeclaration after the fact.  Three flavours
/// of shadow that previously diverged: same-scope, nested-block, and
/// param-vs-local.
#[test]
fn c_backend_rejects_let_shadow_same_scope() {
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "fn main():\n    let x = 1\n    let x = 2\n    print(x)\n";
    let path = std::env::temp_dir().join("lingo_shadow_same.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg("build")
        .arg(&path)
        .output()
        .expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject same-scope shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`x` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
    // belt-and-suspenders: must NOT be a cc-level error
    assert!(!stderr.contains("redefinition of"), "leaked cc error: {stderr}");
    assert!(!stderr.contains("cc failed"), "leaked cc failure: {stderr}");
}

#[test]
fn c_backend_rejects_let_shadow_nested_block() {
    // C nested-block shadowing is legal C — before v0.1.27, the C
    // backend silently accepted this even though the interp rejected it.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "fn main():\n    let x = 1\n    if x == 1:\n        let x = 2\n        print(x)\n";
    let path = std::env::temp_dir().join("lingo_shadow_nested.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg("build")
        .arg(&path)
        .output()
        .expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject nested-block shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`x` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn c_backend_rejects_param_let_shadow() {
    // Pre-v0.1.27 this surfaced as cc's "redeclared as different kind of
    // symbol" — useless to a lingo user.  Now caught by the resolver.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "fn greet(name: str):\n    let name = \"hi\"\n    print(name)\n\nfn main():\n    greet(\"world\")\n";
    let path = std::env::temp_dir().join("lingo_shadow_param.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg("build")
        .arg(&path)
        .output()
        .expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject param-vs-let shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`name` already in scope (shadowing is forbidden)"),
        "wrong diagnostic: {stderr}"
    );
    assert!(!stderr.contains("redeclared as different kind"), "leaked cc error: {stderr}");
}

#[test]
fn c_backend_rejects_let_shadow_against_const() {
    // Top-level consts live in the bottom scope frame; a function-body
    // `let` with the same name must also be rejected.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "const PI: int = 3\n\nfn main():\n    let PI = 4\n    print(PI)\n";
    let path = std::env::temp_dir().join("lingo_shadow_const.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg("build")
        .arg(&path)
        .output()
        .expect("run lingo build");
    assert!(!out.status.success(), "C backend should reject const-vs-let shadow");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`PI` already declared at module scope"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn hello() {
    let (stdout, stderr, code) = run("hello.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello, lingo\n");
}

#[test]
fn forever() {
    let (stdout, stderr, code) = run("forever.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "0\n121\n0\n1\n2\ndone\n";
    assert_eq!(stdout, expected);
}

#[test]
fn c_backend_forever_native_matches_interp() {
    // v0.1.26: `for _ in forever:` lowers to `while (1) { ... }` in the C
    // backend.  Cross-check against the interpreter so future codegen
    // changes can't quietly drift.
    let Some((native_out, stderr, code)) = run_native("forever.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("forever.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn forever_rejects_named_loop_var() {
    // `for x in forever:` is an error — `forever` yields no value, so the
    // loop variable must be `_`.  Both the interpreter and the C backend
    // must reject it with the same message.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "fn main():\n    for x in forever:\n        break\n";
    let path = std::env::temp_dir().join("lingo_forever_bad.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg(&path)
        .output()
        .expect("run lingo");
    assert!(!out.status.success(), "expected interp to reject named forever loop var");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`for x in forever:`"),
        "interp error missing the offending line: {stderr}"
    );
    assert!(
        stderr.contains("must be `_`"),
        "interp error doesn't explain why: {stderr}"
    );

    let out = std::process::Command::new(bin)
        .arg("build")
        .arg(&path)
        .output()
        .expect("run lingo build");
    assert!(!out.status.success(), "expected C backend to reject named forever loop var");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("must be `_`"),
        "C-backend error doesn't explain why: {stderr}"
    );
}

#[test]
fn forever_rejects_value_use() {
    // `let x = forever` must be a compile error — `forever` is not a value.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let src = "fn main():\n    let x = forever\n    print(x)\n";
    let path = std::env::temp_dir().join("lingo_forever_value.lingo");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(bin)
        .arg(&path)
        .output()
        .expect("run lingo");
    assert!(!out.status.success(), "expected interp to reject `forever` as a value");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`forever` is not a value"),
        "wrong error: {stderr}"
    );
}

#[test]
fn fib() {
    let (stdout, stderr, code) = run("fib.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "0\n1\n1\n2\n3\n5\n8\n13\n21\n34\n";
    assert_eq!(stdout, expected);
}

#[test]
fn math() {
    let (stdout, stderr, code) = run("math.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "sum of squares 1..5: 55\nneg zero pos\n1024 1 3\n";
    assert_eq!(stdout, expected);
}

#[test]
fn point() {
    let (stdout, stderr, code) = run("point.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
a: Point{x: 0.0, y: 0.0}
b: Point{x: 3.0, y: 4.0}
dist: 5.0
origin: Point{x: 0.0, y: 0.0}
";
    assert_eq!(stdout, expected);
}

#[test]
fn words() {
    let (stdout, stderr, code) = run("words.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
trimmed: the quick brown fox jumps over the lazy dog
upper: THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG
count: 9
long words: vec[quick, brown, jumps]
contains 'quick'? true
contains 'cat'? false
";
    assert_eq!(stdout, expected);
}

#[test]
fn greet() {
    let (stdout, stderr, code) = run("greet.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
hello, артём! you are 27 years old.
next year you'll be 28.
point = Point{x: 3, y: 4}, sum = 7
the answer is {42} = {42}
";
    assert_eq!(stdout, expected);
}

#[test]
fn wordcount() {
    let (stdout, stderr, code) = run("wordcount.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
unique words: 9
  the: 3
  quick: 2
  brown: 1
  fox: 2
  jumps: 1
  over: 1
  lazy: 1
  dog: 1
  is: 1
";
    assert_eq!(stdout, expected);
}

#[test]
fn traits() {
    let (stdout, stderr, code) = run("traits.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
origin=(1, 2)
ORIGIN=(1, 2)
cat says MEOW
dog says WOOF
cow says MOO
";
    assert_eq!(stdout, expected);
}

#[test]
fn tour() {
    let (stdout, stderr, code) = run("tour.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
|p|^2 = 25
total area = 41
good morning, артём — feeling dangerous?
unique words: 4
parsed: 42
";
    assert_eq!(stdout, expected);
}

#[test]
fn c_backend_hello_native() {
    let Some((stdout, stderr, code)) = run_native("hello.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello, lingo\n");
}

/// REPL end-to-end smoke test: pipe a small session and check the prompt
/// barfs the right values back.  We deliberately use a heredoc-style script
/// so the test is portable and survives parser changes.
#[test]
fn repl_basic_session() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let bin = env!("CARGO_BIN_EXE_lingo");
    let mut child = Command::new(bin)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn repl");

    let session = "\
let x = 21
print(x + x)
fn double(n: i64) -> i64:
    return n * 2

print(double(7))
:quit
";
    child.stdin.as_mut().unwrap().write_all(session.as_bytes()).unwrap();
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("42"), "REPL output missing `42`:\n{stdout}");
    assert!(stdout.contains("14"), "REPL output missing `14`:\n{stdout}");
}

#[test]
fn c_backend_words_native_matches_interp() {
    // v0.1.24 unlock: print(vec[T]) rendering + backwards inference for
    // `let mut x = vec[]` without an annotation.
    let Some((native_out, stderr, code)) = run_native("words.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("words.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_str_methods_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("str_methods_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("str_methods_native.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_greet_native_matches_interp() {
    // f-string interpolation of struct values is the v0.1.22 unlock.
    let Some((native_out, stderr, code)) = run_native("greet.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("greet.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_fstring_enum_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("fstring_enum_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("fstring_enum_native.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_parse_port_native_matches_interp() {
    // The canonical `T!E` / `?` / match-on-result example.
    let Some((native_out, stderr, code)) = run_native("parse_port.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("parse_port.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_str_chars_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("str_chars_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("str_chars_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_vec_user_types_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("vec_user_types_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("vec_user_types_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_traits_lingo_native_matches_interp() {
    // The full traits.lingo example (with `vec[Animal.Cat, ...]`) is now native too.
    let Some((native_out, stderr, code)) = run_native("traits.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("traits.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_traits_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("traits_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("traits_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_vec_push_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("vec_push_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("vec_push_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_wordcount_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("wordcount_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("wordcount_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_vec_strings_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("vec_strings_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("vec_strings_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_strings_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("strings_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("strings_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_vec_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("vec_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("vec_native.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_fib_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("fib.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("fib.lingo");
    assert_eq!(native_out, interp_out, "native and interpreter outputs diverged");
}

#[test]
fn c_backend_math_native_matches_interp() {
    let Some((native_out, stderr, code)) = run_native("math.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("math.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_struct_methods_match_interp() {
    let Some((native_out, stderr, code)) = run_native("point_int.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("point_int.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_enums_match_interp() {
    let Some((native_out, stderr, code)) = run_native("enums_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("enums_native.lingo");
    assert_eq!(native_out, interp_out);
}

#[test]
fn c_backend_debug_print_native() {
    // v0.1.29: matches the interpreter exactly — declared-order fields,
    // unquoted strings inside struct/enum debug.  Pre-v0.1.29 this pinned
    // `User{id: 1, name: "ada", active: true}` and `Event.Message("hi", 42)`
    // (quoted), which disagreed with the interp's `name: ada` / `hi`.
    let Some((stdout, stderr, code)) = run_native("debug_print.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("debug_print.lingo");
    assert_eq!(stdout, interp_out, "native debug print drifted from interp");
    assert_eq!(
        stdout,
        "User{id: 1, name: ada, active: true}\nevent: Event.Login\nevent: Event.Message(hi, 42)\n"
    );
}

#[test]
fn c_backend_point_native() {
    // v0.1.29: the C backend's f64 print now routes through
    // `lingo_f64_str` (shortest round-trip + forced `.0` on whole values),
    // so this now matches the interp exactly.  Pre-v0.1.29 native showed
    // `0` for `0.0`, breaking parity.
    let Some((stdout, stderr, code)) = run_native("point.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("point.lingo");
    assert_eq!(stdout, interp_out, "native point output drifted from interp");
    assert_eq!(
        stdout,
        "a: Point{x: 0.0, y: 0.0}\nb: Point{x: 3.0, y: 4.0}\ndist: 5.0\norigin: Point{x: 0.0, y: 0.0}\n"
    );
}

#[test]
fn c_backend_floats_native() {
    // v0.1.29: native and interp now share the float formatter
    // (`lingo_f64_str` <=> Rust's `Display`).  Pre-v0.1.29 native used
    // `%g` (6 sig figs, no trailing .0), which clipped `3.141592653589793`
    // to `3.14159`.  The interp side is the canonical format.
    let Some((stdout, stderr, code)) = run_native("floats_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("floats_native.lingo");
    assert_eq!(stdout, interp_out, "native float output drifted from interp");
    assert_eq!(
        stdout,
        "5.0\n5.0\n3.141592653589793\n19.634954084936208\n1024.0\n",
        "float format drifted"
    );
}

#[test]
fn parse_int_interp() {
    // v0.2.0: `int(s) -> int!str` builtin — interp side.  Pinned format
    // is the canonical "one debug-print form": error strings come
    // through as `int: can't parse "<rust-debug-repr>"`.
    let (stdout, stderr, code) = run("parse_int.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ok: 42\n\
         err: int: can't parse \"hello\"\n\
         ok: -7\n\
         err: int: can't parse \"\"\n\
         sum: 7\n\
         err: int: can't parse \"oops\"\n",
    );
}

#[test]
fn c_backend_parse_int_native() {
    // v0.2.0: the C backend's `int(s)` lowers to `lingo_int_parse(...)`,
    // returning the monomorphized `lingo_result_i64_str_t`.  Error
    // messages route through `lingo_str_debug_escape` so the wire
    // format matches Rust's `Debug for &str` and the interpreter
    // byte-for-byte.  Also exercises `?` on `! str` and match-binding
    // the error string into an f-string.
    let Some((stdout, stderr, code)) = run_native("parse_int.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("parse_int.lingo");
    assert_eq!(stdout, interp_out, "native parse_int drifted from interp");
    assert_eq!(
        stdout,
        "ok: 42\n\
         err: int: can't parse \"hello\"\n\
         ok: -7\n\
         err: int: can't parse \"\"\n\
         sum: 7\n\
         err: int: can't parse \"oops\"\n",
    );
}

#[test]
fn try_else_coerce_interp() {
    // v0.2.2: `? else <expr>` lifts the inner error into the caller's
    // raise type.  This is the interp-side pin.
    let (stdout, stderr, code) = run("try_else.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ok: 42\n\
         err: empty\n\
         err: nan\n\
         ok: 0\n",
    );
}

#[test]
fn c_backend_try_else_coerce_native() {
    // v0.2.2: the C backend's `?` accepts a typed mismatch when an
    // `else <expr>` fallback is provided, lowering to
    //   `if (__tr_n.is_err) return (outer){ .is_err = true, .err = <fb> }`
    // The `__tr_n.err` is referenced via a comma-expr so side effects
    // in the inner call are preserved without forcing a type match.
    let Some((stdout, stderr, code)) = run_native("try_else.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("try_else.lingo");
    assert_eq!(stdout, interp_out, "native try_else drifted from interp");
    assert_eq!(
        stdout,
        "ok: 42\n\
         err: empty\n\
         err: nan\n\
         ok: 0\n",
    );
}

#[test]
fn io_roundtrip() {
    let (stdout, stderr, code) = run_with_args("io_roundtrip.lingo", &["a", "b", "c"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
wrote and read back 36 bytes
hello from lingo, 3 arg(s) passed in
";
    assert_eq!(stdout, expected);
}

#[test]
fn parse_port() {
    let (stdout, stderr, code) = run("parse_port.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
ok: 8080 -> port
err: empty
err: not a number
err: out of range
";
    assert_eq!(stdout, expected);
}

#[test]
fn shapes() {
    let (stdout, stderr, code) = run("shapes.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
circle area: 12.56636
rect area: 12.0
triangle area: 6.0
";
    assert_eq!(stdout, expected);
}
