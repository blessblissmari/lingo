# lingo — roadmap

> goal: a usable v1.0 self-hosted compiler.

## phase 0 — design ✅ done

- [x] vision + readme
- [x] design rationale (`docs/DESIGN.md`)
- [x] syntax reference (`docs/SYNTAX.md`)
- [x] committed decisions (`docs/DECISIONS.md`)
- [x] grammar sketch (`docs/GRAMMAR.bnf`)

## phase 1 — frontend (v0.1) — in rust  🟡 in progress

- [x] **lexer** (hand-written, INDENT/DEDENT, rejects tabs) — `compiler/src/lexer.rs`
- [x] **parser → ast** (recursive descent, source spans) — `compiler/src/parser.rs`
- [x] **tree-walking interpreter** (no codegen yet) — `compiler/src/interp.rs`
- [x] **cli** — `lingo path/to/file.lingo`, `--tokens`, `--ast`
- [x] hello world + fib + math examples passing as integration tests
- [x] **structs** with field access and `impl Type:` methods (incl. static)
- [x] **enums** with variant payloads, `Type.Variant(...)` construction
- [x] **match** with wildcard, literal, bind, and `Type.Variant(...)` patterns
- [x] **vec[T]** literal `vec[a, b, c]` + `.len/.push/.pop/.get/.set/.contains/.clear/.reverse`
- [x] **string methods** `.len/.contains/.starts_with/.ends_with/.to_lower/.to_upper/.trim/.split/.replace`
- [x] **for** over range / vec / str
- [x] **error types** `fn foo() -> T ! E:` + `raise` + `?` propagation + `match ok(v) / err(e)`
- [x] **f-strings** `f"hi {name}, sum = {a + b}"` with arbitrary expressions inside `{...}`
- [x] **map[K, V]** literal `map{key: val, key: val}` + `.len/.has/.get/.set/.remove/.keys/.values/.clear`
- [x] **utf-8 in string literals** (lexer copies full codepoints, not bytes)
- [x] **io builtins** `read_file`, `write_file`, `args()`, `int(s)`, `str(x)` — all return `T ! str` where fallible
- [x] **traits + impl-trait blocks** with required signatures, default-impl methods, and conformance checks
- [x] **C backend MVP** (`lingo build foo.lingo` → native binary via gcc). Subset: fn / if / for-range / arithmetic / print / recursion. fib(35): native 22ms vs interpreter 51s (~2300×).
- [x] **C backend structs + methods** (v0.1.8): `struct` decls lower to C structs; `impl Type:` static and instance methods lower to `Type_method`; `Pt{x: 1, y: 2}` lowers to a C99 designated initializer.
- [x] **C backend enums + match** (v0.1.9): each `enum T` becomes `T_Tag` + `struct T { tag; union { ... } as; }`; variant constructors lower to designated initializers; `match` becomes `switch (x.tag)` with payload subpatterns binding to `x.as.Variant._0/_1/...`.
- [x] **C backend f64 floats** (v0.1.10): `f64` lowers to `double`; float literals always emit a decimal point; arithmetic upcasts when either side is f64; `x ** y` on f64 lowers to `pow(x, y)` (libm linked with `-lm`); `%` on floats is a compile-time error.
- [x] **C backend debug print + keyword args** (v0.1.11): `print(value)` auto-generates a `Type{field: value, ...}` format for structs and a `Type.Variant(payload, ...)` format for enums (matches Rust's `{:?}` intent). Keyword args resolved positionally in calls (`f(name: 1, value: 2)`), with duplicate/unknown/missing-arg checks.
- [ ] auto-wrap `?` via a `From<E>` trait (needs generic trait params first)
- [ ] name resolution + scope analysis as a separate pass (today it's inline)
- [ ] type checker (hindley-milner inside fn bodies, nominal at boundaries)
- [ ] traits (`trait` + `impl Trait for Type`)
- [ ] auto-wrap `?` via `From` (needs traits)
- [ ] generics (monomorphized — interpreter currently ignores type args)
- [ ] explicit allocators + `defer`
- [ ] 500-program test suite, mostly hand-written, partly llm-generated

**exit:** the interpreter runs every `examples/*.lingo` correctly.

## phase 2 — backend (v0.2)

- [ ] mid-level IR (SSA)
- [ ] LLVM backend via `inkwell` (linux x86_64 first)
- [ ] linker integration → single-file native binary
- [ ] QBE backend for fast debug builds (optional but nice)
- [ ] basic optimisation passes (inline, dce, constant folding)
- [ ] no stdlib yet — just `print`, primitives, and structs

**exit:** fib, sieve, mandelbrot run within 2× of equivalent zig.

## phase 3 — stdlib (v0.3)

a deliberately small core:

- `io` — stdin/stdout/stderr, buffered readers/writers
- `fs` — file ops, paths
- `str` — utf-8 ops, parsing, formatting
- `vec`, `map`, `set`, `option`, `result` (built-in but documented here)
- `iter` — combinators (`map`, `filter`, `fold`, `take`, `zip`, `enumerate`)
- `time` — instants, durations
- `os` — env, args, exit
- `math` — basic numerics
- `rand` — seeded prng
- `json` — parse/stringify
- `net` — tcp, udp, ip parsing
- `sync` — mutex, atomic, channel
- `nursery` — structured concurrency runtime

**exit:** can write a non-trivial cli tool and a tiny http server in pure lingo.

## phase 4 — tooling (v0.4)

- [ ] `lingo fmt` — opinionated formatter (no options)
- [ ] `lingo doc` — html docs from `##` comments
- [ ] `lingo test` — built-in test runner; `test "name": ...` blocks
- [ ] `lingo lsp` — language server (completion, goto-def, hover, diagnostics)
- [ ] vscode + neovim plugins
- [ ] `lingo pkg` — package manager, `lingo.toml`
- [ ] **docs.lingo.dev** — package registry (only after v0.4 is real)

**exit:** an outside contributor clones a project, runs `lingo build`, it works.

## phase 5 — self-hosting (v1.0)

rewrite the compiler in lingo. once it compiles itself, freeze the v1.0
grammar.

**exit:** `lingo build` of the lingo compiler produces a binary byte-identical
to one bootstrapped from rust.

## punted past v1.0

- async/await two-color split (we have nurseries)
- gpu / cuda backends
- ios / android targets (linux/macos/windows first)
- a web playground (we'll do one anyway because it's fun)
- jit / repl
- user-facing `comptime`
