# lingoc — the bootstrap compiler

This crate is the rust-hosted bootstrap compiler for the
[lingo](../README.md) language. It currently ships:

- a hand-written **lexer** with python-style INDENT/DEDENT
- a recursive-descent **parser** that produces an AST
- a tree-walking **interpreter**
- a **CLI** that runs `.lingo` files

It does **not** ship a code generator yet — that comes in v0.2 (LLVM backend
via `inkwell`).

## Quickstart

```bash
cd compiler
cargo run --release -- ../examples/hello.lingo
# hello, lingo

cargo run --release -- ../examples/fib.lingo
# 0 1 1 2 3 5 8 13 21 34   (one per line)
```

## CLI

```text
lingo <file.lingo>           # run the file
lingo --tokens <file>        # dump the token stream
lingo --ast    <file>        # dump the parsed AST
lingo --version
```

## Language subset (today)

Implemented:

- `fn` declarations with typed parameters and return type
- `const` declarations (literal expressions only)
- `let` / `let mut` (no shadowing — compile error if you try)
- `if` / `elif` / `else`
- `for i in 0..N:` (range loops)
- `return`, `break`, `continue`
- arithmetic (`+ - * / % **`), comparison (`< <= > >= == !=`), boolean (`and or not`)
- function calls with positional **and** keyword arguments
  (keyword args **required** when a fn has >2 params)
- `print(...)` builtin
- literals: int, float, str (with `\n \t \r \\ \" \0`), bool, none

Not yet implemented (coming):

- structs, enums, traits, generics
- error type `! E`, `?` propagation
- explicit allocators, `defer`
- f-strings, collections (`vec`, `map`, `set`, `string`)
- structured concurrency (`nursery`)
- pattern `match`
- file/network I/O
- LLVM backend (v0.2)

## Layout

```text
compiler/
├── Cargo.toml
├── src/
│   ├── main.rs      # CLI
│   ├── lib.rs       # re-exports
│   ├── lexer.rs
│   ├── ast.rs
│   ├── parser.rs
│   ├── interp.rs
│   └── error.rs
├── examples/
│   ├── hello.lingo
│   ├── fib.lingo
│   └── math.lingo
└── tests/
    └── run_examples.rs
```

## Running the test suite

```bash
cargo test
```

All three example programs are pinned to expected stdout in
`tests/run_examples.rs`. CI green = the language behaviour is unchanged.
