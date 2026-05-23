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
- [x] **C backend vec[i64] (read-only)** (v0.1.12): `vec[1, 2, 3]` literal lowers to a C99 compound literal backing a `lingo_vec_i64_t { const int64_t* data; int64_t len; }`. Supported ops: `v.len()`, `v.get(i)`, and `for x in v` iteration. Type annotations `vec[i64]` work in fn params. Mutation (`push`/`pop`/`set`) and other element types wait until the allocator story lands. Bench: vec iteration ~3000× faster than interpreter (5ms vs 15s for 10M ops).
- [x] **`for _ in forever:` infinite loops** (v0.1.26): the canonical "loop until something inside the body says stop" shape — documented in `DECISIONS.md` and `SYNTAX.md` since phase 0, but the `Tok::Forever` keyword was unused by the parser/interp/codegen. Now lands end-to-end: parser accepts `forever` as a primary, interp runs a `loop { ... }` with full `break` / `continue` / `return` / `?`-propagation support, C backend lowers to `while (1) { ... }`. The loop variable must be `_` (forever yields no value) — both the interpreter and the C backend reject `for x in forever:` with a source-mapped error, and `forever` used as a value (`let x = forever`) is also rejected. 39/39 integration tests green (added: interp run of `forever.lingo`, native-vs-interp cross-check, two negative tests for the diagnostic paths).
- [x] **C-keyword-safe local identifiers** (v0.1.25): lingo locals/params/match-binds/for-vars that collide with a C99/C11 keyword (`long`, `int`, `register`, `static`, `enum`, `default`, …) or a runtime-symbol name (`malloc`, `printf`, `main`, …) are auto-prefixed with `l_` on emission only — the lingo source stays unchanged.  `c_local_ident()` is the single chokepoint; struct/enum/field/fn names go through their own namespaces and are unaffected.  Reverts the `words.lingo` workaround so `let mut long = vec[]` now works native.
- [x] **`print(vec[T])` rendering vec contents + backwards element-type inference for empty `vec[]`** (v0.1.24): `print` now lowers a `vec[T]` argument to a loop that emits `vec[a, b, c]` (matching the interpreter's `display`), with per-element formatting for primitives and structs.  And `let [mut] x = vec[]` without an annotation no longer defaults to `vec[i64]` — a per-function pre-pass scans for the first `x.push(e)` and back-fills the element type, tracking for-loop iter bindings so a `for w in words:` over `vec[str]` makes `w: str`.  Unlocks `words.lingo` as a native example.  *TODO:* identifier mangling so lingo locals that collide with C keywords (e.g. `long`) don't need to be renamed by the user.
- [x] **`str.trim()` + `vec[T].contains(x)` in native** (v0.1.23): `trim` strips ASCII whitespace (space/tab/newline/CR/VT/FF) into a fresh `malloc`'d copy. `vec.contains` lowers to a GCC stmt-expr scanning `.data` for `==` (primitives) or `strcmp` (strings). Step toward making `words.lingo` native; still blocked there by (a) `let mut x = vec[]` without an annotation needing backwards element-type inference from a later `.push`, and (b) `print(vec[T])` rendering vec contents instead of `<vec>`.
- [x] **f-string interpolation of struct / enum values in native** (v0.1.22): `f"point = {p}"` and `f"shape = {s}"` now render the same debug form `print` uses — `Point{x: 3, y: 4}`, `Shape.Circle(7)`. Structs lower inline via `lingo_fmt_alloc` with per-field debug specs; enums materialise a temp + switch-on-tag that assigns a freshly formatted `const char*` per variant. Unlocks `greet.lingo` as a native example.
- [x] **`T ! E` + `?` + match-on-result in native** (v0.1.21): fallible functions return a monomorphized `lingo_result_<T>_<E>_t` struct (`{ bool is_err; T ok; E err; }`), emitted once per distinct (T, E) pair after the user enum typedefs. `raise X` lowers to `return { .is_err = true, .err = X };`, normal `return v` to `{ .is_err = false, .ok = v };`. `expr?` lowers to a GCC stmt-expr that unwraps `.ok` or early-returns the propagated error. `match` on a result scrutinee supports `ok(bind|_)`, `err(EnumName.Variant(...))` with nested binds, `err(bind|_)`, and `_`. Closes `parse_port.lingo` — the canonical README example for the "errors as values" pitch — now compiling and running as a native binary, bit-for-bit matching the interpreter.
- [x] **`for ch in str:` in native** (v0.1.20): UTF-8 codepoint iteration. Each `ch` binds as `const char*` pointing into a per-iteration 5-byte buffer (max codepoint length), exactly matching the interpreter's "each char is a 1-codepoint str" semantics. ASCII paths (`parse_int`, lexers) work; cyrillic/CJK round-trip (`reverse("привет")` → `"тевирп"`) verified bit-for-bit.
- [x] **Monomorphized `vec[Struct]` / `vec[Enum]` in native** (v0.1.19): every user struct/enum gets its own `lingo_vec_<TypeName>_t` typedef + `new/push/pop/set` helpers emitted right next to the typedef. `vec[Point]`, `vec[Animal]`, etc all work end-to-end (literal, push, pop, set, for-iter). `let v: vec[Point] = vec[]` is now type-hinted so the empty literal picks the right element type. Closes the last big interp-only example: `traits.lingo` (with `vec[Animal.Cat, ...]`) now compiles native.
- [x] **Traits + enum methods in native, static dispatch** (v0.1.18): `trait T:` / `impl T for Type:` lowered to ordinary mangled C functions. `self` is now allowed to be an enum type, and method dispatch on enum receivers works. Default trait methods are baked in if the impl doesn't override. No vtables / no `&dyn T` polymorphism — that's still v0.2. New helpers: `lingo_str_to_upper` / `lingo_str_to_lower` (ASCII).
- [x] **Owning vec + `vec.push/pop/set` in native** (v0.1.17): vec changes from a read-only `const T* data; len` view to an owning `T* data; len; cap` buffer. Literals lower to a GCC stmt-expr `({ vec v = new(); push(...); push(...); v; })`. `push/set` are emitted as statements (`(void)0` rvalue), `pop` is an expression. Mutating receivers must be plain idents (same restriction as `map.set`). Buffers still leak — allocator/defer story is v0.2.
- [x] **Interactive REPL** (v0.1.16): `lingo repl`, persistent root scope + redefinable decls, multi-line input ends on blank line, `:help` / `:clear` / `:quit` meta commands. Routes top-level items (`fn`/`struct`/`enum`/`impl`/`trait`/`const`) into the interp's tables; everything else gets wrapped in a synthetic `fn __repl_eval()` and exec'd statement-by-statement against the persistent scope. `register_items(prog, allow_replace)` + `exec_top_stmt(stmt)` are the two new public Interp entry points the REPL is built on.
- [x] **C backend `map[str, i64]`** (v0.1.15): linear-scan, parallel-array, realloc-grown. Empty `map{}` literal only (non-empty needs typechecker or stmt-exprs). Methods: `.len()`, `.has(k)`, `.get(k)` (returns 0 on missing — `has`-check first to distinguish), `.set(k, v)`, `.keys() -> vec[str]`. Mutating methods require the receiver to be a plain identifier (addressable). Will swap for a real open-addressing hash table once we have a typed `hash(K)` story. Unlocks `wordcount_native.lingo`.
- [x] **C backend `vec[T]` for T ∈ {i64, f64, str} + `s.split()`** (v0.1.14): `CType::VecI64` → `CType::Vec(Box<CType>)`. Three runtime structs (`lingo_vec_{i64,f64,str}_t`), one per element type — collapses to a single template once we have monomorphization. `s.split(sep) -> vec[str]` lowered via a `lingo_str_split` runtime helper (malloc'd pieces + array). `for x in v` and `v.get(i)` produce the right element type.
- [x] **C backend str runtime + f-strings** (v0.1.13): `s1 + s2` concat (malloc'd, leaks), `s.len()` (bytes!), `s == s2` / `s != s2` (`strcmp`), `s.contains(t)`, `s.starts_with(t)`, `s.ends_with(t)`, and f-strings `f"hi {name}, you are {age}"` (two-pass `vsnprintf` into a fresh buffer). Runtime helpers (`lingo_str_concat`, `lingo_str_starts_with`, `lingo_str_ends_with`, `lingo_fmt_alloc`) are emitted in the prelude. Leak-only memory; allocator lands later. Known divergence: `len` returns bytes in native but chars in interp — fine for ASCII, pinned per test for non-ASCII.
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
