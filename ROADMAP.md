# lingo — roadmap

> goal: a usable v1.0 self-hosted compiler. realistic time horizon: 12-18 months
> of focused work, or ~36 if it's a nights-and-weekends thing. either is fine.

## phase 0 — design (we are here)

- [x] vision + readme
- [x] design rationale (`docs/DESIGN.md`)
- [x] syntax sketch (`docs/SYNTAX.md`)
- [ ] decide: rust-style borrow checker vs. zig-style allocators vs. hybrid
- [ ] decide: single-error-type vs. union-of-errors per fn (`! E` vs `! E | F`)
- [ ] write the bnf grammar
- [ ] hand-write 20+ programs in the proposed syntax and grade their readability
- [ ] get 3 outside reviewers to roast the design

**exit criteria:** the grammar is stable enough that two people writing
example programs independently produce code that looks the same.

## phase 1 — frontend (v0.1)

implementation language: **rust** (boring, fast, great parsing libs).

- [ ] lexer (hand-written, with significant indentation)
- [ ] parser → ast
- [ ] desugaring pass (e.g. `?` → match)
- [ ] type checker (hindley-milner style for locals, nominal for boundaries)
- [ ] borrow checker (the simple version, no lifetimes)
- [ ] tree-walking interpreter (for tests and quick experiments)
- [ ] a 500-program test suite, mostly hand-written

**exit criteria:** the interpreter runs `examples/*.lingo` correctly.

## phase 2 — backend (v0.2)

- [ ] mid-level IR (SSA, similar to MIR in spirit)
- [ ] LLVM backend (codegen + linker integration)
- [ ] QBE backend for fast debug builds (optional but nice)
- [ ] single-file native binaries on linux + macos + windows
- [ ] basic optimization passes (inline, dce, constant folding) on the mid-IR
- [ ] no stdlib yet — just `print`, primitives, and structs

**exit criteria:** fib, sieve, mandelbrot, and a couple of micro-benchmarks
run within 2x of equivalent zig.

## phase 3 — stdlib (v0.3)

a deliberately small core:

- `io` — stdin/stdout/stderr, buffered readers/writers
- `fs` — file ops, paths
- `str` — utf-8 string ops, parsing, formatting
- `vec`, `map`, `set`, `option`, `result` (built-in but documented here)
- `iter` — combinators (`map`, `filter`, `fold`, `take`, `zip`, `enumerate`)
- `time` — instants, durations
- `os` — env, args, exit
- `math` — basic numerics
- `rand` — seeded prng
- `json` — parse/stringify
- `net` — tcp, udp, ip parsing
- `sync` — mutex, atomic, channel (for the nursery runtime)

**exit criteria:** can write a non-trivial cli tool and a tiny http server
in pure lingo.

## phase 4 — tooling (v0.4)

- [ ] `lingo fmt` — opinionated formatter (no options)
- [ ] `lingo doc` — generates html docs from `##` doc comments
- [ ] `lingo test` — built-in test runner; `test "name": ...` blocks
- [ ] `lingo lsp` — language server (completion, goto-def, hover, diagnostics)
- [ ] vscode + neovim plugins
- [ ] **pkg** — package manager, single-source-of-truth `lingo.toml`
- [ ] **docs.lingo.dev** — package registry

**exit criteria:** an outside contributor can clone a project, run
`lingo build`, and have it work.

## phase 5 — self-hosting (v1.0)

rewrite the compiler in lingo. once it can compile itself, freeze the v1.0
grammar.

**exit criteria:** `lingo build` of the lingo compiler produces a binary
byte-identical to one bootstrapped from rust.

## things explicitly punted past v1.0

- async/await two-color split (we're starting with nurseries)
- gpu / cuda backends
- ios / android targets (linux/macos/windows first)
- a web playground (we'll do one anyway because it's fun)
- jit / repl

## what to do *first* if you have one weekend

1. lock the grammar (bnf in `compiler/grammar.bnf`).
2. write the lexer + a parser that can parse `examples/hello.lingo`.
3. write a tree-walker that can execute it.
4. celebrate. now do `fib.lingo`.

incremental wins beat a one-year stealth compiler every time.
