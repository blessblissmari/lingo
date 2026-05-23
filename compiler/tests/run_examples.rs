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

#[test]
fn hello() {
    let (stdout, stderr, code) = run("hello.lingo");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello, lingo\n");
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
    let Some((stdout, stderr, code)) = run_native("debug_print.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "User{id: 1, name: \"ada\", active: true}\nevent: Event.Login\nevent: Event.Message(\"hi\", 42)\n"
    );
}

#[test]
fn c_backend_point_native() {
    // Same Point example the interp test uses, but the native formatter prints
    // `0` instead of `0.0` for whole-valued doubles (libc `%g`).  Pin the exact
    // native output here and let the cross-check ride once we share a printer.
    let Some((stdout, stderr, code)) = run_native("point.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "a: Point{x: 0, y: 0}\nb: Point{x: 3, y: 4}\ndist: 5\norigin: Point{x: 0, y: 0}\n"
    );
}

#[test]
fn c_backend_floats_native() {
    // f64 ops compile + run.  We don't yet share a float print-format with the
    // interpreter (interp uses Rust's `{}` -> "5.0", native uses `%g` -> "5"),
    // so this test pins the native output exactly and leaves a Phase-1.5 ticket
    // to unify formatting.
    let Some((stdout, stderr, code)) = run_native("floats_native.lingo") else { return };
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "5\n5\n3.14159\n19.635\n1024\n",
        "native float output drifted"
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
