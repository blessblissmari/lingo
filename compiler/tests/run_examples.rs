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
