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
    which_cc()?;
    // `lingo build` writes its output binary into the *current
    // directory* under the entry file's stem.  Multiple tests
    // building `main.lingo` in parallel (the v0.3.0 module examples
    // all use `examples/{name}/main.lingo`) would otherwise race on a
    // shared `./main` file.  Give each test its own scratch cwd so
    // builds and runs are isolated.
    let project_root = std::env::current_dir().expect("cwd");
    let entry_abs = project_root.join("examples").join(file);
    let stem = std::path::Path::new(file).file_stem().unwrap().to_string_lossy().to_string();
    let work_dir = std::env::temp_dir().join(format!(
        "lingo_native_{}",
        file.replace(['/', '.'], "_")
    ));
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir).expect("scratch dir");
    let build = Command::new(cargo_bin())
        .current_dir(&work_dir)
        .arg("build")
        .arg(&entry_abs)
        .output()
        .expect("failed to invoke lingo build");
    if !build.status.success() {
        panic!(
            "lingo build failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
    }
    let bin = work_dir.join(&stem);
    if !bin.exists() {
        return None;
    }
    let run = Command::new(&bin).current_dir(&work_dir).output().ok()?;
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
fn generic_trait_interp() {
    // v0.2.5: user-defined generic trait + the built-in `From` going
    // through the same general impl-resolution gate.  Two `Encoder[T]`
    // impls (T = int and T = str), each with a distinct receiver
    // struct, plus a `From[str] for ParseErr` impl that drives `?`.
    let (stdout, stderr, code) = run("generic_trait.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
int[4](42)\n\
name=ada\n\
ok: 17\n\
err: empty\n";
    assert_eq!(stdout, expected);
}

#[test]
fn c_backend_generic_trait_native() {
    // v0.2.5: native build of the same generic-trait example.  Static
    // dispatch keeps the two `Encoder` impls as plain mangled C fns
    // (`IntEnc_encode` / `StrEnc_encode`), and the `From[str] for
    // ParseErr` impl emits as the existing `lingo_from_str__ParseErr`
    // mangled helper the `?` operator already knows how to call.
    let Some((stdout, stderr, code)) = run_native("generic_trait.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("generic_trait.lingo");
    assert_eq!(stdout, interp_out, "native generic_trait drifted from interp");
}

#[test]
fn generic_trait_sig_interp() {
    // v0.2.6: trait method sig uses `T` + `Self`, impl spells out
    // the concrete types — substitution makes it line up.
    let (stdout, stderr, code) = run("generic_trait_sig.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "int bag first = 7\nstr bag first = ada\n";
    assert_eq!(stdout, expected);
}

#[test]
fn c_backend_generic_trait_sig_native() {
    // v0.2.6: same example through the C backend — still byte-
    // identical to interp.
    let Some((stdout, stderr, code)) = run_native("generic_trait_sig.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("generic_trait_sig.lingo");
    assert_eq!(stdout, interp_out, "native generic_trait_sig drifted from interp");
}

fn run_source(src: &str, filename: &str) -> (String, String, i32) {
    use std::process::Command;
    let p = std::env::temp_dir().join(filename);
    std::fs::write(&p, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lingo")).arg(&p).output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn generic_trait_sig_param_type_mismatch() {
    // v0.2.6: typo in impl method's parameter type is caught at
    // resolve time with a precise expected/got diagnostic.
    let src = "trait Encoder[T]:\n    fn encode(self, v: T) -> str\n\nstruct IntEnc:\n    pad: int\n\nimpl Encoder[int] for IntEnc:\n    fn encode(self, v: str) -> str:\n        return \"oops\"\n\nfn main():\n    print(\"hi\")\n";
    let (_, stderr, code) = run_source(src, "lingo_v026_sig_param.lingo");
    assert_ne!(code, 0);
    assert!(stderr.contains("parameter `v` expected `int`, got `str`"),
            "expected param-type diagnostic, got: {stderr}");
}

#[test]
fn generic_trait_sig_return_type_mismatch() {
    let src = "trait Encoder[T]:\n    fn encode(self, v: T) -> str\n\nstruct IntEnc:\n    pad: int\n\nimpl Encoder[int] for IntEnc:\n    fn encode(self, v: int) -> int:\n        return 0\n\nfn main():\n    print(\"hi\")\n";
    let (_, stderr, code) = run_source(src, "lingo_v026_sig_ret.lingo");
    assert_ne!(code, 0);
    assert!(stderr.contains("return type expected `str`, got `int`"),
            "expected return-type diagnostic, got: {stderr}");
}

#[test]
fn generic_trait_sig_raises_mismatch() {
    let src = "trait Parse[E]:\n    fn parse(self, s: str) -> int ! E\n\nstruct P:\n    n: int\n\nimpl Parse[str] for P:\n    fn parse(self, s: str) -> int ! int:\n        return 0\n\nfn main():\n    print(\"hi\")\n";
    let (_, stderr, code) = run_source(src, "lingo_v026_sig_raises.lingo");
    assert_ne!(code, 0);
    assert!(stderr.contains("raises clause expected `str`, got `int`"),
            "expected raises-clause diagnostic, got: {stderr}");
}

#[test]
fn generic_trait_arity_mismatch_too_few() {
    // v0.2.5: clear diagnostic when a generic trait's bracket arity
    // doesn't match the impl.  Smoke-test only — running the source
    // is expected to fail at resolve time on both backends.
    use std::process::Command;
    let dir = std::env::temp_dir().join("lingo_v025_arity_too_few.lingo");
    std::fs::write(&dir, "trait Foo[T]:\n    fn show(self) -> str\n\nstruct S:\n    n: int\n\nimpl Foo for S:\n    fn show(self) -> str:\n        return \"x\"\n\nfn main():\n    print(\"hi\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lingo")).arg(&dir).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("trait `Foo` declares 1 type parameter"),
            "expected arity diagnostic, got: {stderr}");
}

#[test]
fn generic_trait_arity_mismatch_too_many() {
    use std::process::Command;
    let dir = std::env::temp_dir().join("lingo_v025_arity_too_many.lingo");
    std::fs::write(&dir, "trait Foo:\n    fn show(self) -> str\n\nstruct S:\n    n: int\n\nimpl Foo[int] for S:\n    fn show(self) -> str:\n        return \"x\"\n\nfn main():\n    print(\"hi\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lingo")).arg(&dir).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("trait `Foo` takes no type parameters"),
            "expected arity diagnostic, got: {stderr}");
}

#[test]
fn parse_float_interp() {
    // v0.2.4: `float(s) -> float ! str` on the interp side.  Identical
    // shape to v0.2.0's `parse_int_interp`: parsing + From-trait coercion
    // into a caller's enum without an `else` annotation.
    let (stdout, stderr, code) = run("parse_float.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "\
ok: 3.14\n\
err: empty\n\
err: not a number\n\
ok: 1000000000.0\n";
    assert_eq!(stdout, expected);
}

#[test]
fn c_backend_parse_float_native() {
    // v0.2.4: native build of the same example.  Failure messages route
    // through the same `lingo_str_debug_escape` helper that the int
    // parser uses, so `float: can't parse "..."` is byte-identical to
    // the interpreter for ASCII inputs.
    let Some((stdout, stderr, code)) = run_native("parse_float.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("parse_float.lingo");
    assert_eq!(stdout, interp_out, "native parse_float drifted from interp");
}

#[test]
fn try_from_trait_interp() {
    // v0.2.3: `impl From[E1] for E2:` makes `int(s)?` auto-coerce its
    // `str` err into the caller's `ParseErr` without an `else` fallback.
    let (stdout, stderr, code) = run("try_from.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ok(42)\n\
         Empty\n\
         NotANumber\n",
    );
}

#[test]
fn c_backend_try_from_trait_native() {
    // v0.2.3: native build for the same example — the codegen looks up
    // `from_impls[(inner_e_suffix, raises_e_suffix)]` and emits a call to
    // the mangled `lingo_from_<E1>__<E2>` fn in place of `__tr_n.err`,
    // so the err arm of the outer Result is constructed from the wrapped
    // value, not the raw inner err.
    let Some((stdout, stderr, code)) = run_native("try_from.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("try_from.lingo");
    assert_eq!(stdout, interp_out, "native try_from drifted from interp");
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

// =====================================================================
// v0.3.0 — multi-file modules.
//
// Each module example lives in its own subdirectory under `examples/`
// (so the resolver has a concrete entry-file directory to walk
// `import foo.bar` from).  Interp + native are checked together to
// catch any drift between the two backends.
// =====================================================================

#[test]
fn modules_basic() {
    let (stdout, stderr, code) = run("modules_basic/main.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "2 + 3 = 5\nsquare(7) = 49\nPI ~ 3\n";
    assert_eq!(stdout, expected);
}

#[test]
fn modules_basic_native() {
    let Some((stdout, stderr, code)) = run_native("modules_basic/main.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("modules_basic/main.lingo");
    assert_eq!(stdout, interp_out, "native diverged from interp");
}

#[test]
fn modules_alias() {
    let (stdout, stderr, code) = run("modules_alias/main.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "m.add(10, 20) = 30\nm.PI = 3\n";
    assert_eq!(stdout, expected);
}

#[test]
fn modules_alias_native() {
    let Some((stdout, stderr, code)) = run_native("modules_alias/main.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("modules_alias/main.lingo");
    assert_eq!(stdout, interp_out);
}

#[test]
fn modules_nested() {
    // `import foo.bar` resolves to `foo/bar.lingo` relative to the
    // entry file's directory.  Verifies the dotted-path lowering in
    // the resolver.
    let (stdout, stderr, code) = run("modules_nested/main.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "bar.greet() = hello from foo.bar\nbar.SHOUT = HI\n";
    assert_eq!(stdout, expected);
}

#[test]
fn modules_nested_native() {
    let Some((stdout, stderr, code)) = run_native("modules_nested/main.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("modules_nested/main.lingo");
    assert_eq!(stdout, interp_out);
}

#[test]
fn modules_enum() {
    // Each module is free to declare its own structs/enums and use
    // them in its own functions.  Cross-module *type references*
    // (`fn f() -> bar.Point`) are deferred to v0.3.x — this example
    // only exercises within-module uses.
    let (stdout, stderr, code) = run("modules_enum/main.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    let expected = "area_sq(4) = 16\narea_rect(3, 5) = 15\n";
    assert_eq!(stdout, expected);
}

#[test]
fn modules_enum_native() {
    let Some((stdout, stderr, code)) = run_native("modules_enum/main.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    let (interp_out, _, _) = run("modules_enum/main.lingo");
    assert_eq!(stdout, interp_out);
}

// ---- diagnostic-shape negative tests -------------------------------

#[test]
fn modules_reject_missing_file() {
    // `import does_not_exist` with no matching .lingo on disk should
    // produce a clear, file-pointing diagnostic — not a generic IO
    // error from the OS layer.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let dir = std::env::temp_dir().join("lingo_modules_missing");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.lingo");
    std::fs::write(&entry, "import does_not_exist\nfn main():\n    print(\"hi\")\n").unwrap();
    let out = Command::new(bin).arg(&entry).output().expect("run lingo");
    assert!(!out.status.success(), "should reject missing import target");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot resolve `import does_not_exist`"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn modules_reject_duplicate_alias() {
    // Two `import` statements in the same file that produce the same
    // alias (either by name collision or via `as`) must be rejected.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let dir = std::env::temp_dir().join("lingo_modules_dup_alias");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.lingo");
    let a = dir.join("a.lingo");
    let b = dir.join("b.lingo");
    std::fs::write(&entry, "import a\nimport b as a\nfn main():\n    print(\"hi\")\n").unwrap();
    std::fs::write(&a, "fn x() -> int:\n    return 1\n").unwrap();
    std::fs::write(&b, "fn y() -> int:\n    return 2\n").unwrap();
    let out = Command::new(bin).arg(&entry).output().expect("run lingo");
    assert!(!out.status.success(), "should reject duplicate alias");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("duplicate import alias `a`"),
        "wrong diagnostic: {stderr}"
    );
}

#[test]
fn modules_reject_import_cycle() {
    // Two modules importing each other should report a cycle, not
    // recurse forever.
    let bin = env!("CARGO_BIN_EXE_lingo");
    let dir = std::env::temp_dir().join("lingo_modules_cycle");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.lingo");
    let a = dir.join("a.lingo");
    let b = dir.join("b.lingo");
    std::fs::write(&entry, "import a\nfn main():\n    print(a.hello())\n").unwrap();
    std::fs::write(&a, "import b\nfn hello() -> str:\n    return b.tag()\n").unwrap();
    std::fs::write(&b, "import a\nfn tag() -> str:\n    return \"b\"\n").unwrap();
    let out = Command::new(bin).arg(&entry).output().expect("run lingo");
    assert!(!out.status.success(), "should reject import cycle");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cyclic import"), "wrong diagnostic: {stderr}");
}
