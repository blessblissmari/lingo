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

fn run(file: &str) -> (String, String, i32) {
    let out = Command::new(cargo_bin())
        .arg(format!("examples/{file}"))
        .output()
        .expect("failed to run lingo");
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
