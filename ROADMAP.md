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
