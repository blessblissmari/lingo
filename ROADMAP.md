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
- [x] **interp ≡ native parity for struct/enum debug + f64 format** (v0.1.29): the full v0.1.x consolidation pass. Audited all 28 examples across both backends; the four real divergences (`debug_print`, `floats_native`, `point`, `shapes`) all stemmed from two root causes and are now fixed. (a) **Declared field order in debug**: `Value::Struct.fields` switched from `HashMap<String, Value>` to `Vec<(String, Value)>` so the interpreter renders fields in declaration order (matching the C backend); `display()` no longer sorts. Construction still validates "every field set exactly once" via a temporary HashMap, then materializes into the Vec in the declared order. (b) **Float formatting**: the C backend now routes every f64 print site (top-level, struct/enum debug, vec[f64] elements, f-string interpolation) through a new `lingo_f64_str(double)` runtime helper — shortest decimal that round-trips back to the same double (matches Rust's `Display for f64`), with `.0` forced on whole-valued doubles (`5` → `5.0`) and `%g` as the non-finite fallback. Uses a 32-slot rotating static buffer so a single `printf` can embed many f64 values without clobbering earlier slots. (c) **String quoting in debug**: `debug_fmt_for` now emits `%s` (not `\"%s\"`) for `CType::Str` inside struct/enum payloads, matching the interpreter's `Value::display` (unquoted) — so `User{name: ada}` and `Event.Message(hi, 42)`, not `name: "ada"` / `Message("hi", ...)`. The pinned tests for `debug_print`, `point`, and `floats_native` now cross-check native output against the interpreter (`assert_eq!(stdout, interp_out)`) so any future drift fails immediately. 48/48 integration tests green; the audit script (committed at the repo root) reports 24/24 applicable examples matching byte-for-byte. **Known v0.2 gaps surfaced by the audit**: `tour.lingo` uses the `int(s) -> int!str` parsing builtin which the C backend doesn't yet route (it only treats `int(x)` as a type cast); `wordcount.lingo` uses `match` on a `map.get()` (non-enum) scrutinee, also not supported by the C backend's match lowering. Both have native-friendly companions (`wordcount_native.lingo`) and are tracked here for v0.2.
- [x] **no-shadow parity for for-loop vars and match-arm binds** (v0.1.28): closes the remaining gap called out at the end of v0.1.27. The interp and the C backend now both reject `let i = 0; for i in 0..3: ...` and `let x = 1; match v: Opt.Some(x): ...` with the same `resolve error: \`x\` already in scope (shadowing is forbidden)` diagnostic, before any C is emitted. Also covers `ok(bname)` / `err(bname)` / `err(Variant(bname))` binds against `T!E` scrutinees. `_` (the "don't bind" sigil) is still allowed everywhere. *interp*: outer-scope walk added to `Stmt::For` and `pattern_match` `Pattern::Bind`; same-pattern-duplicate check (`Pair(x, x)`) preserved with its original wording. *C backend*: `check_no_shadow` reused from v0.1.27, called from `Stmt::For` and from every `Pattern::Bind` site in `emit_match` + `emit_match_result`. 48/48 tests green (added 5: interp+native for-var, interp+native match-bind, plus a positive `_ in` smoke test).
- [x] **`let` shadowing diagnostics in the C backend** (v0.1.27): bring the C backend up to parity with the interpreter on `DECISIONS.md`'s "no shadowing" rule. Previously `let x = 1; let x = 2` in the same scope surfaced as cc's `redefinition of 'x'`, nested-block `let x` over an outer `let x` was silently accepted (legal C shadow), and a `let name = ...` inside `fn greet(name: str):` produced cc's confusing `redeclared as different kind of symbol`. All three now produce the canonical `resolve error: \`x\` already in scope (shadowing is forbidden)` with the right source span, before any C is emitted. Top-level `const X` colliding with a function-body `let X` is also caught (using the interp's `already declared at module scope` wording). Added `Codegen::check_no_shadow` and a `consts: HashSet<String>` field; called from `emit_stmt` Stmt::Let. 39/39 tests green (added 4 negative tests, one per failure mode). *Follow-up: for-loop var and match-arm-bind shadowing against outer scopes are still latent in both backends.*
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
- [x] **`int(s)` parsing builtin in the C backend** (v0.2.0): closes half of the `tour.lingo` gap. `int(s) -> int!str` now lowers to `lingo_int_parse(s)` returning the monomorphized `lingo_result_i64_str_t`. The C backend's `CType::Result` now also accepts `Str` as the error type (suffix `"str"` in `result_pairs`, field type `const char*`), and `emit_match_result` accepts ok/err binds against str-errors (variant patterns are rejected with a clear diagnostic since there are no variants on `str`). Error messages route through a new `lingo_str_debug_escape` runtime helper that mirrors Rust's `Debug for &str` byte-for-byte so `int: can't parse "..."` is byte-identical to the interpreter. The runtime block is now spliced between the typedef section and the protos section (sentinel `/* === lingo runtime helpers === */`) so helpers can reference monomorphized result typedefs. New tests: `parse_int_interp` + `c_backend_parse_int_native` (cross-checked with the interpreter via `assert_eq!(stdout, interp_out)`); audit picks up `parse_int.lingo` automatically — 50/50 integration tests green, 25/29 examples byte-identical.
- [x] **`Opt[T]` for `map.get` on both backends** (v0.2.1): `map.get(k)` now returns `Opt[V]` everywhere. The interpreter gained a first-class `Value::Opt(Option<Box<Value>>)` (separate from the existing `Value::None_` "no return value" sentinel) and `pattern_match` handles `some(x)` / `none` binds. The C backend gained `CType::Opt(Box<CType>)`, monomorphized `lingo_opt_<T>_t` typedefs (same scheme as `lingo_result_<T>_<E>_t`), a per-T `lingo_opt_<T>_str` runtime formatter spliced after `lingo_fmt_alloc` (so f-string and `print(opt)` render `none` or the inner value's display — no `Some(...)` wrapper), and a new `emit_match_opt` lowering that turns `match opt:` into a `do { if (...) { ...; break; } } while (0)` chain (same shape as `emit_match_result`, with `some(name)` binding `opt.val`).  `map.get` lowers to a GCC stmt-expr that probes `has(k)` once and returns either `{true, get(k)}` or `{false, 0}`.  `wordcount.lingo` now runs end-to-end on both backends with byte-identical output (audit shows 26/29 matching, up from 25/29).  Still deferred (v0.2.2): `?` error-type coercion (`int!str` → `int!ParseErr` etc.) for the rest of `tour.lingo` to round-trip native.
- [x] **`?` error-type coercion via `? else <expr>` (v0.2.2)**: `?` now accepts an optional `else <expr>` trailer that lifts the inner error into the caller's `raises.1` type by raising the fallback value instead of the inner err.  Parser was extended to consume `else <expr>` after the `?` token; AST `ExprKind::Try` grew an `Option<Box<Expr>>` fallback field.  Interp evaluates the fallback expression in the err path and seats it as the new `pending_raise`.  The C backend keeps the existing `inner_e == raises.1` check **unless** a fallback is provided, in which case it emits a comma-expr `((void)__tr_n.err, (<fb>))` so the inner err is still evaluated for side effects while the raised value comes from the fallback.  Closes the final `tour.lingo` gap — both interp and native now run the full tour with byte-identical output.  Audit reports 27/27 non-interactive examples matching (all examples now match, the two skipped are interactive `io_roundtrip` + `fib_native_bench`).  New tests: `try_else_coerce_interp` + `c_backend_try_else_coerce_native`; new example `examples/try_else.lingo`.  52/52 integration tests green.  Picked syntactic sugar over a real `From[E1] for E2` trait because the trait route needed instance lookup wired into the typechecker and the sugar covers 100% of the `tour.lingo` use case with a single AST field; can grow the trait later without breaking the sugar.
- [x] **Implicit `?` coercion via `impl From[E1] for E2:` (v0.2.3)**: lifts the v0.2.2 design from sugar-only to sugar + a real built-in `From` trait.  `From` is *magic* (no user-visible `trait From` decl needed) and parses as `impl From[<E1>] for <E2>:` with a generic parser extension that consumes `[IDENT (, IDENT)*]` between the trait name and `for`.  AST `ImplTraitBlock` grew a `trait_args: Vec<String>` field (regular trait impls leave it empty).  Resolution registers the single `from(e: E1) -> E2` method into a `(E1_name, E2_name) -> FnDecl` table on the interp side, and a `(E1_suffix, E2_suffix) -> mangled_fn_name` table on the C side; the C `from` body is lowered into a regular `Item::Fn` named `lingo_from_<E1>__<E2>`, so call resolution piggybacks on the existing fn machinery (no separate dispatch path).  `?` lowering now consults this table when `inner_e != caller.raises.1` and no `? else` fallback is present: if a `From` impl is registered, the err is wrapped (interp calls the `from` fn at the propagation point; native emits `<from_fn>(__tr_n.err)` in place of `__tr_n.err` as the err of the outer Result), and if no impl is found the existing diagnostic fires — now suggesting both routes (`impl From[..] for ..:` or `? else <value>`).  Interp also tracks `current_fn_raises_e: Vec<String>` pushed on entry to fns with `! E`, used to resolve the target type at `Try`-time.  New example `examples/try_from.lingo`, new tests `try_from_trait_interp` + `c_backend_try_from_trait_native`.  54/54 integration tests green; audit 29/31 examples matching (no regression — the new example added one row in both numerator and denominator).  Sugar form (`? else`) and trait form coexist: sugar wins per call site, trait covers the bulk case.
- [x] **v0.2 consolidation (v0.2.7)**: closes the 0.2.x line in the same shape as v0.1.29 closed 0.1.x.  `docs/DECISIONS.md` extended with a "v0.2 — decisions added during the 0.2.x line" section covering every shippable choice: parsing builtins as `T ! str` (v0.2.0, v0.2.4), `map.get -> Opt[T]` (v0.2.1), `? else <expr>` sugar (v0.2.2), `impl From[E1] for E2:` for auto-wrapping `?` (v0.2.3), user-defined generic traits (v0.2.5), trait method signature substitution + the "one source of truth" rule for the new `ast.rs` helpers (v0.2.6), why default-impl methods skip the sig check, why there is no overloading/SFINAE/specialization.  Also: cleaned up every clippy style warning across the compiler (the 0.2.x line accumulated ~10 of them — `map_or(false, _)` → `is_some_and`, `if x.is_none(){return None}` → `?`, `push_str("\n")` → `push('\n')`, missing `Default` impls, `.zip(args.into_iter())` → `.zip(args)`, doc list overindent).  `cargo clippy --release --all-targets` now passes with **zero warnings** across lib, lib-test, and integration tests.  No behavioural changes; 65/65 tests green, audit 32/34 examples byte-identical.
- [x] **Trait method signature substitution (v0.2.6)**: closes the v0.2.5 "lenient conformance" gap.  Each `impl Trait[A1, A2] for Target:` block now goes through a real signature-equality check between the trait method (with `type_params[i] -> trait_args[i]` and `Self -> Target` substituted into every TypeRef) and the impl method (with `Self -> Target` substituted to handle the parser-injected placeholder on `self` params).  Per-param types, return type, and `! E` raises clauses are all compared structurally — including nested type args like `vec[T]` becoming `vec[int]`.  Diagnostics are precise: `method `Encoder.encode` for `IntEnc`: parameter `v` expected `int`, got `str``.  Three failure modes covered (param-type, return-type, raises-clause mismatch); default-impl methods (taken straight from the trait body) skip the check by definition.  New shared helpers in `ast.rs`: `subst_typeref`, `typeref_eq`, `typeref_display`, `build_trait_subst`, `check_trait_method_sig` — used identically by interp and codegen.  New `examples/generic_trait_sig.lingo` (`trait Bag[T]:` with `Self` in method sigs + two impls).  65/65 integration tests green; audit 32/34 matching.
- [x] **User-defined generic traits — `trait Foo[T1, T2, ...]:` (v0.2.5)**: lifts the v0.2.3 special-cased `From[E]` machinery to a general path that every trait shares.  Parser now consumes optional `[IDENT (, IDENT)*]` brackets on `trait` declarations and stores them in a new `TraitDecl.type_params: Vec<String>` field (regular `trait Foo:` still parses with an empty vec).  Resolution (both interp and codegen) replaces the old "if trait_name == \"From\" else reject brackets" gate with a uniform arity check: every `impl Trait[A1, A2] for Target:` block looks up the trait, validates `trait_args.len() == trait_decl.type_params.len()`, and the diagnostic is shaped to the case (`trait \"Foo\" takes no type parameters, but impl provided 1 ([int])` vs `trait \"Foo\" declares 1 type parameter(s) (T); impl provided 0`).  The built-in `From[E]` is *auto-synthesized* as a real `TraitDecl { type_params: ["E"], methods: [] }` if any `impl From[..] for ..:` is seen without a user-visible declaration — so the v0.2.3 source-level shape is unchanged but the validation goes through the general gate.  `From`-specific lowering still wins (mangled standalone `lingo_from_<E1>__<E2>` fn, populating `from_impls` for `?`-coercion), but only after the general arity check passes.  New `examples/generic_trait.lingo` showing a user-defined `trait Encoder[T]:` with `impl Encoder[int] for IntEnc:` + `impl Encoder[str] for StrEnc:`, plus a `From[str] for ParseErr` impl driving `?`.  New tests `generic_trait_interp` + `c_backend_generic_trait_native` (byte-identical interp/native pin) and two negative tests `generic_trait_arity_mismatch_too_few` + `..._too_many`.  60/60 integration tests green; audit 31/33 matching (no regression — new example adds one row in both numerator and denominator).  Method-signature substitution (`T` and `Self` in the trait method's signature) is deferred — for v0.2.5 the impl must spell out concrete types in each method, same lenient stance the existing non-generic conformance check takes.
- [x] **`float(s)` parsing builtin on both backends (v0.2.4)**: closes the `int(s)`/`float(s)` symmetry gap.  Interp gains a `"float"` arm in `call_builtin_free` (identity / int→float widen / bool→float / str→`parse::<f64>`); error string is `format!("float: can't parse {:?}", s)` byte-for-byte.  C backend adds a `float` branch in `gen_call` next to `int` (str → `lingo_float_parse(s)` returning `lingo_result_f64_str_t`; int → `(double)x`; bool → `(... ? 1.0 : 0.0)`; float identity), the `("f64","str")` result-pair is always reserved (mirrors the v0.2.0 i64/str reservation) so the typedef + helper compile in every translation unit, and the runtime block grows a `lingo_float_parse` helper modelled on `lingo_int_parse` (whitespace trim → `strtod` with full-string consumption + `ERANGE` check → `lingo_str_debug_escape`-formatted err).  Slots into the v0.2.3 `From`-trait machinery for free: an `impl From[str] for FloatErr` makes plain `float(s)?` propagate into the caller's enum.  New `examples/parse_float.lingo`, new tests `parse_float_interp` + `c_backend_parse_float_native`.  56/56 integration tests green; audit 30/32 examples matching (no regression — new example adds one row in both numerator and denominator).

**exit:** fib, sieve, mandelbrot run within 2× of equivalent zig.

## phase 3 — stdlib (v0.3)

before any of this can land, we need multi-file programs.  v0.3.0 ships
**modules** so the stdlib can be one file at a time, and so user
programs aren't forced to live in one `.lingo` file.

- [x] **Multi-file modules — `import foo.bar` (v0.3.0)**: a brand-new
  `compiler/src/modules.rs` resolver runs *before* the interp / C
  backend and flattens every transitively-reachable file into one
  `Program` AST.  `import foo` reads `foo.lingo` next to the entry
  file; `import foo.bar` lowers the dots to directory separators
  (`foo/bar.lingo`); `import foo as f` lets you rename the alias.  the
  alias is the only way to reach another module's names: `f.fn()`,
  `f.CONST`, and `f.MyEnum.Variant` work, bare `fn()` only resolves
  locally.  every non-entry module's top-level names are prefixed
  `lm{i}__` deterministically so the flat program stays
  collision-free; users never see the prefix.  cycles are caught
  with a named chain (`a.lingo -> b.lingo -> a.lingo`); duplicate
  aliases inside one file and missing import targets get clear
  file-pointing diagnostics.  the C backend's `pass 3` now opens a
  module-level scope frame so top-level consts (previously
  untested in single-file v0.2.x because no example exercised them)
  are visible from any function body.  the resolver also reorders
  flattened items so types come before consts come before
  functions, matching what readers naturally write in single-file
  programs.  4 new examples (`modules_basic`, `modules_alias`,
  `modules_nested`, `modules_enum`), 7 new integration tests +
  3 negative diagnostic tests = 76/76 green; audit 36/38
  byte-identical (2 still interactive).  clippy 0 warnings.
  ~~**deferred to v0.3.x:** cross-module *type references*
  (`fn f() -> bar.Point`) and cross-module struct literals
  (`bar.Point{}`).~~ — landed in v0.3.1.

- [x] **Cross-module type refs and struct literals (v0.3.1)**:
  closes the v0.3.0 deferred items.  `fn f() -> bar.Point`,
  `let p: bar.Point = ...`, `vec[bar.Point]`, and
  `bar.Point{x: 1, y: 2}` all parse and resolve correctly.
  parser change: `type_ref()` accepts one `.IDENT` suffix after
  a leading ident (deeper paths are a parse-time error: "cross-
  module type refs are one hop only").  in expression position,
  a three-token lookahead (`Dot IDENT(upper) LBrace`) routes
  `alias.Name{...}` straight into a `StructLit` with name
  `"alias.Name"`; every other dotted form still flows through
  the existing postfix path.  resolver-side: a single change to
  `RewriteCtx::maybe_prefix_typename` splits dotted names on
  `.`, looks the alias up in `self.imports`, and resolves the
  last segment through `prefix_by_canonical`.  unknown aliases
  are recorded via a `RefCell<Vec<LingoError>>` on the ctx and
  surfaced at the end of the rewrite pass — clean failure
  instead of leaving a dotted name to confuse the backends.
  the interp and the C backend are untouched: by the time they
  run, every reference is a flat `lm{i}__Name` ident.
  new example `modules_xmod_types/` ({geom.lingo, main.lingo})
  exercising cross-module return type + struct literal + struct
  arg.  3 new tests (1 positive interp+native pin, 2 negative
  diagnostic: unknown alias + two-hop reject).  80/80 green;
  audit 37/39 byte-identical.  clippy 0 warnings.

- [x] **Structural `==` / `!=` on struct, enum, and vec (v0.3.2)**:
  before v0.3.2, `==` on user types was a type error — equality was
  defined only for `int`/`float`/`bool`/`str` and (deep inside
  `match`) for enums via `values_eq`.  v0.3.2 opens the operator
  too.  interp side: `bin_op` short-circuits `Eq`/`Ne` on any
  pair of compound values (struct/enum/vec) into a single
  `values_eq` call; `values_eq` gains `Struct` (type-name match
  + field-wise recursion) and `Vec_` (length + element-wise
  recursion) arms.  `Map_` deliberately *not* added — order-
  sensitive equality on an associative container is the wrong
  default.  C backend side: pass 1c gains a per-struct and
  per-enum forward declaration + body for
  `static bool lingo_eq_<T>(<T> a, <T> b)`.  struct body is an
  `&&`-chain over fields, enum body is a `tag` check then a
  per-variant `switch` whose arms reduce to the same `&&`-chain
  over payload slots; both recurse into nested struct/enum
  fields by calling the helper for that field's type.  if any
  field/payload is of a non-comparable type (`Map`, `Result`,
  `Opt`), the helper still gets emitted (returning `false`) so
  cross-references compile, but `==` *use* on that type is
  rejected at the call site with a localized error pointing at
  the user's `==`, not the synthesised helper.  `vec[T]` eq is
  inlined as a GCC statement expression (`({ ... })`) and
  delegates element comparison to the per-element helper.
  new example `eq_struct_enum.lingo` (nested struct + payload-
  carrying enum + vec eq).  3 new tests (positive interp pin,
  interp ≡ native pin, negative type-error pin for mixed-kind
  comparisons).  **83/83 green** (was 80/80).  audit
  **38/40** byte-identical interp ≡ native.  clippy 0 warnings.
  no AST / parser / resolver changes.

- [x] **`to_str(v) -> str` builtin (v0.3.3)**:
  closes the second half of the v0.3.2 structural-helpers gap.
  before v0.3.3, the only way to get a printable string out of
  a struct/enum/vec was to write a custom formatter or splice
  the value into an f-string (which only worked because the
  interp already had `Value::display`).  v0.3.3 surfaces that
  same display shape as a one-call builtin: `let s = to_str(p)`
  returns `"Point{x: 1, y: 2}"` for an interp value and a
  byte-identical heap string from the C backend.  intercepted
  **by name** at the call dispatch site — mirrors how `int(x)`
  and `float(x)` casts are handled — so `to_str` is not a
  keyword and `trait Show: fn show(self) -> str` style traits
  keep working.  single positional argument; multi-arg was
  considered and dropped in favour of `"label: " + to_str(p)`.
  works on int / float / bool / str / struct / enum / `vec[T]`;
  rejects `map`, `Result[T,E]`, `Opt[T]` at compile time
  (match on them first).  C backend: pass 1c gains a per-struct
  / per-enum `lingo_show_<T>` helper (alongside `lingo_eq_<T>`),
  and `gen_program` flushes a `lingo_show_vec_<T>` helper per
  distinct element type seen.  runtime helpers
  (`lingo_show_i64/u64/f64/bool/str`, `lingo_strjoin` variadic
  concat) are spliced unconditionally with `__attribute__
  ((unused))`.  new example `to_str_struct_enum.lingo`, 3 new
  tests (positive interp, interp ≡ native byte-identical,
  negative compile-error pin for `to_str(map)`).  **86/86 green**
  (was 83/83).  audit **39/41** byte-identical (the +1 is the
  new example, the failures are the same two preexisting non-
  matchers — neither involves `to_str`).  clippy 0 warnings.

- [x] **`s.replace(from, to)` in native (v0.3.4)**:
  closes the last common-string-method gap between interp and
  the C backend.  the interp has had `replace` since v0.1; the
  C backend now lowers `s.replace(from, to)` to a new
  `lingo_str_replace` runtime helper next to `lingo_str_split`.
  two-pass: first counts non-overlapping occurrences of `from`
  in `s`, then allocates exactly `s_len + count*(to_len -
  from_len) + 1` bytes and copies + substitutes in one sweep.
  matches Rust's `str::replace` byte-for-byte for non-empty
  `from` on ASCII (and on UTF-8 too, when `from` itself is
  valid UTF-8 — the substitution happens at codepoint-aligned
  positions).  empty `from` is rejected at runtime — the
  interp delegates to Rust's codepoint-aware behaviour there
  and we can't replicate it bytewise without a real UTF-8
  decoder; the diagnostic mirrors `lingo_str_split`'s
  empty-separator one (`str.replace: empty `from` not supported
  yet`).  `gen_str_method` gained the `("replace", 2)` arm and
  the empty-vec back-inference table gained the `(Str,
  "replace") -> Str` row so `let mut acc = vec[]` followed by
  `acc.push(s.replace(...))` infers the element type
  correctly.  no AST / parser / interp changes — pure C
  backend work.  new example `str_replace_native.lingo`
  (literal sub, multi-occurrence remove, no-match identity,
  space → underscore, `to_lower` chained with `replace`,
  growing replacement, shrinking replacement).  3 new tests
  (`str_replace_interp`, `c_backend_str_replace_native_matches_interp`,
  `c_backend_str_replace_empty_from_runtime_error`).  **89/89
  green** (was 86/86).  audit **40/42** byte-identical interp
  ≡ native (the +1 is the new example, the 2 skips are still
  the preexisting interactive `io_roundtrip` +
  `fib_native_bench`).  clippy 0 warnings.

then the stdlib itself, a deliberately small core:

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
