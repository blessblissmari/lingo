<div align="center">

# 🦜 lingo

**a fast coding language for ai.**
fast as zig. simple as python. loved by llm agents.

</div>

> ⚠️ **status:** v0.3.2 — bootstrap interpreter, **working C backend**, **interactive REPL**, plus everything from v0.2.x.  **Modules (v0.3.0):** `import foo` reads `foo.lingo`; `import foo.bar` reads `foo/bar.lingo`; `import foo as f` renames the alias.  The alias is the only way to reach another module's top-level names — `f.fn()`, `f.CONST`, `f.MyEnum.Variant`.  Cycle / duplicate-alias / missing-target diagnostics are mandatory.  **Cross-module types (v0.3.1):** `fn f() -> bar.Point`, `let p: bar.Point = ...`, `vec[bar.Point]`, and `bar.Point{x: 1, y: 2}` all work — the resolver rewrites every dotted reference to a flat ident before the interp / C backend runs, so both backends keep working on one flat program.  Parser limit: one module hop (`a.b.C` rejected).  Unknown alias is a clean resolver-time error.  **Structural eq (v0.3.2):** `==` and `!=` now work field-wise on structs, tag+payload-wise on enums, and element-wise on `vec[T]`, recursing through nested compound types.  `Map`/`Result`/`Opt` deliberately stay non-`==`-able — error at the call site, not at the helper.  83/83 integration tests green; **38/40 applicable examples** byte-identical interp ≡ native (two skipped are interactive `io_roundtrip` + `fib_native_bench`).  clippy 0 warnings.  No re-exports, no `import *`, no privacy.
> structs / enums / `match` / `vec[T]` / `map[str, i64]` / f-strings / utf-8 / `T ! E` error types / `?` / io builtins / traits all work in the interpreter; a growing subset compiles to native via the C backend (≈3000× faster on `fib(35)`, ≈3000× on `vec` ops, byte-identical output on `wordcount`).
> all design decisions are committed in [`docs/DECISIONS.md`](docs/DECISIONS.md).
> disagree? open an issue.

---

## try it now

```bash
git clone https://github.com/blessblissmari/lingo
cd lingo/compiler
cargo run --release -- examples/hello.lingo
# hello, lingo

cargo run --release -- examples/fib.lingo
# 0 1 1 2 3 5 8 13 21 34   (one per line)

cargo test
# 25 passed; 0 failed

# compile to a native binary via the C backend:
cargo run --release -- build examples/fib.lingo
./fib
# 0 1 1 2 3 5 8 13 21 34

# interactive REPL (NEW in v0.1.16):
cargo run --release -- repl
# >>> let x = 21
# >>> print(x + x)
# 42
# >>> fn double(n: i64) -> i64:
# ...     return n * 2
# ...
# >>> print(double(7))
# 14
```

requires rust 1.75+ (`rustup` will get you there).
no other dependencies — the bootstrap compiler is zero-dep rust.
the C backend shells out to `cc` (gcc/clang); no LLVM yet.

## why another language?

every existing language was designed for *humans*. lingo is designed for the new pair of programmers in the room: **a human and an llm agent**.

that changes the priorities:

| classical priority           | lingo priority                                   |
| ---------------------------- | ------------------------------------------------ |
| terse syntax for fast typing | **regular, unambiguous syntax** llms predict well |
| many ways to do the thing    | **one obvious way** — less variance, fewer bugs  |
| implicit magic, conventions  | **explicit > implicit** — types, errors, alloc   |
| hidden control flow          | **all control flow is visible** in the code      |
| performance via cleverness   | performance via a **predictable cost model**     |

the rule we apply when we hesitate: **pick the option that makes the *reader's*
life easier, even at the cost of the writer's.** an llm agent is a reader 90%
of the time.

lingo also has to be *fast*. so the runtime model is closer to zig than to
python: no GC, monomorphized generics, native backend, zero-cost abstractions.

## the 30‑second pitch

```lingo
# hello.lingo
fn main():
    print("hello, lingo")
```

```lingo
# fib.lingo — compiles to native, no GC, no runtime
fn fib(n: u64) -> u64:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

fn main():
    for i in 0..10:
        print(fib(i))
```

```lingo
# errors are values. one error type per fn. `?` propagates.
enum ParseError:
    Empty
    OutOfRange
    BadChar(char)

fn parse_port(s: str) -> u16 ! ParseError:
    let n = int.parse(s)?
    if n < 0 or n > 65535:
        return err(ParseError.OutOfRange)
    return n as u16

fn main(args: [str]) ! ParseError:
    let port = parse_port(args.get(1).unwrap_or("8080"))?
    print("listening on", port)
```

```lingo
# wordcount_native.lingo — compiles to native via the C backend.
fn main():
    let text = "the quick brown fox jumps over the lazy dog the fox is quick"
    let mut counts: map[str, i64] = map{}
    for w in text.split(" "):
        if counts.has(w):
            counts.set(w, counts.get(w) + 1)
        else:
            counts.set(w, 1)
    print(f"unique: {counts.len()}")
    for k in counts.keys():
        print(f"{k}: {counts.get(k)}")
```

## the 10 rules (full list in [`docs/DECISIONS.md`](docs/DECISIONS.md))

1. **indentation-based, python-shaped** — llms already speak it fluently.
2. **one loop, one error shape, one string interpolation, one comment shape.**
3. **types at signatures, inferred inside** — `fn` boundaries always typed.
4. **errors are values:** `T ! E` with `?` to propagate. no exceptions, ever.
5. **explicit allocators:** any fn that may allocate takes `alloc: &Allocator`. *(allocator API lands in v0.1.x; until then the C backend leaks.)*
6. **no implicit conversions, no truthiness, no shadowing, no default args.**
7. **keyword args required when a fn has >2 parameters.**
8. **structured concurrency only** — `nursery`, no `async fn`, no goroutines.
9. **traits for behaviour, structs for data.** no inheritance.
10. **native backend + monomorphized generics** → target: within 10% of zig.

## what works today (v0.1.29)

### interpreter

- `fn` / `let` / `let mut` (no shadowing)
- `if` / `elif` / `else`, `for x in iter:` (ranges and collections), `for _ in forever:` (infinite loop), `return` / `break` / `continue`
- arithmetic, comparison, boolean ops, `**`, `%`
- structs + methods, enums + `match`, traits + `impl T for U` (incl. default methods)
- `vec[T]` literals, `map[K, V]` literals + methods (`.len`, `.has`, `.get`, `.set`, `.keys`, `.values`, `.contains`, `.remove`, `.clear`)
- string runtime: `+`, `.len`, `.contains`, `.starts_with`, `.ends_with`, `.to_upper`, `.to_lower`, `.trim`, `.split`, `.replace`
- f-strings: `f"x={x}, y={point.x + 1}"`
- error types: `T ! E`, `?` propagation, `raise`, `ok(v)` / `err(e)`
- io builtins for files/argv
- positional + keyword args (kwargs required when a fn has >2 params)

### C backend (native)

compiles a subset of the above to a single self-contained C file, then
shells out to `cc` to produce a native binary. supported today:

- ints (i64/u64), floats (f64), bools, strings (byte-counted, ASCII-safe)
- `fn`, control flow, recursion
- structs with fields + methods (static and instance), auto-debug `print(point)`
- enums with payloads + `match` + auto-debug `print(shape)`
- `vec[i64]`, `vec[f64]`, `vec[str]` literals + `.len`, `.get`, `.push`, `.pop`, `.set`, `for`-iteration (owning, heap-backed)
- `map[str, i64]` empty literals + `.len`, `.has`, `.get`, `.set`, `.keys`
- string ops: `+`, `.len`, `.contains`, `.starts_with`, `.ends_with`, `.split`
- f-strings (allocated via 2-pass `vsnprintf`)
- positional + keyword args, default-aware static dispatch

### REPL (NEW in v0.1.16)

- `lingo repl` drops you into an interactive session
- persistent root scope: `let` bindings survive across prompts
- `fn` / `struct` / `enum` / `impl` / `trait` / `const` declarations accumulate (and can be **redefined** — REPL convenience overrides the "no duplicates" rule)
- multi-line entries end on a blank line
- `:help`, `:clear`, `:quit` meta commands; Ctrl-D also quits

### examples

**multi-file (v0.3.0+)** — every entry file is `main.lingo` inside its
own directory; the resolver follows `import` lines to sibling
`.lingo` files.  v0.3.1 adds cross-module types
(`fn f() -> bar.Point`, `bar.Point{x: 1}`):
[`modules_basic`](compiler/examples/modules_basic/main.lingo) ·
[`modules_alias`](compiler/examples/modules_alias/main.lingo) ·
[`modules_nested`](compiler/examples/modules_nested/main.lingo) ·
[`modules_enum`](compiler/examples/modules_enum/main.lingo) ·
[`modules_xmod_types`](compiler/examples/modules_xmod_types/main.lingo)

**structural equality (v0.3.2)** — `==` / `!=` work on user types:
field-wise on structs, tag+payload-wise on enums, element-wise on
`vec[T]`.  [`eq_struct_enum`](compiler/examples/eq_struct_enum.lingo)

native-capable:
[`hello`](compiler/examples/hello.lingo) ·
[`forever`](compiler/examples/forever.lingo) ·
[`fib`](compiler/examples/fib.lingo) ·
[`math`](compiler/examples/math.lingo) ·
[`point`](compiler/examples/point.lingo) ·
[`point_int`](compiler/examples/point_int.lingo) ·
[`enums_native`](compiler/examples/enums_native.lingo) ·
[`floats_native`](compiler/examples/floats_native.lingo) ·
[`debug_print`](compiler/examples/debug_print.lingo) ·
[`vec_native`](compiler/examples/vec_native.lingo) ·
[`strings_native`](compiler/examples/strings_native.lingo) ·
[`vec_strings_native`](compiler/examples/vec_strings_native.lingo) ·
[`vec_push_native`](compiler/examples/vec_push_native.lingo) ·
[`wordcount_native`](compiler/examples/wordcount_native.lingo) ·
[`shapes`](compiler/examples/shapes.lingo) ·
[`traits_native`](compiler/examples/traits_native.lingo) ·
[`traits`](compiler/examples/traits.lingo) ·
[`vec_user_types_native`](compiler/examples/vec_user_types_native.lingo) ·
[`str_chars_native`](compiler/examples/str_chars_native.lingo) ·
[`parse_port`](compiler/examples/parse_port.lingo)

interp-only (waiting on `?`/`!E` lowering, trait vtables, or `T!E` lowering):
[`words`](compiler/examples/words.lingo) ·
[`tour`](compiler/examples/tour.lingo) ·
[`parse_port`](compiler/examples/parse_port.lingo) ·
[`io_roundtrip`](compiler/examples/io_roundtrip.lingo) ·
[`traits`](compiler/examples/traits.lingo) ·
[`greet`](compiler/examples/greet.lingo)

## docs

- [`docs/DECISIONS.md`](docs/DECISIONS.md) — every committed rule, in one place
- [`docs/DESIGN.md`](docs/DESIGN.md) — *why* the rules look this way
- [`docs/SYNTAX.md`](docs/SYNTAX.md) — full syntax reference (v0.1)
- [`docs/GRAMMAR.bnf`](docs/GRAMMAR.bnf) — formal grammar sketch
- [`ROADMAP.md`](ROADMAP.md) — what gets built, in what order
- [`compiler/README.md`](compiler/README.md) — how the bootstrap compiler works

## roadmap (short)

- **v0.1.x** — bootstrap frontend (lexer, parser, tree-walking interp) + C backend MVP + REPL. *(we are here)*
- **v0.2** — LLVM backend, allocators + `defer`, generics via monomorphization, single-file native binaries.
- **v0.3** — minimal stdlib (io, fs, str, vec, map, iter, time, net, json) — written in lingo.
- **v0.4** — `lingo fmt`, `lingo lsp`, `lingo test`, package manager.
- **v1.0** — self-hosted compiler.

## non‑goals

- object inheritance, exceptions, null, implicit conversions, function colors.
- macros that rewrite syntax (the compiler handles generics; no user `comptime` in v0.1).
- a giant batteries‑included stdlib — small core, good package manager.
- being a php or a ruby — we don't want 14 ways to write a `for` loop.

prs and issues welcome — especially on syntax. the worst time to change a
language is *after* people write code in it.
