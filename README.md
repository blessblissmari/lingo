<div align="center">

# 🦜 lingo

**a fast coding language for ai.**
fast as zig. simple as python. loved by llm agents.

</div>

> ⚠️ **status:** v0.1.7 — bootstrap interpreter + a *working C backend*. 15/15 integration tests green.
> structs / enums / match / vec / map / f-strings / utf-8 / `T ! E` error types / `?` / io builtins / traits all work in the interpreter; a subset compiles to native via the C backend (≈2300× faster on `fib(35)`).
> all design decisions are committed in [`docs/DECISIONS.md`](docs/DECISIONS.md).
> disagree? open an issue.

---

## try it now

```bash
git clone https://github.com/blessblissmari/lingo
cd lingo/compiler
cargo run --release -- examples/hello.lingo
# hello, lingo

cargo run --release -- ../examples/fib.lingo
# 0 1 1 2 3 5 8 13 21 34   (one per line)

cargo test
# 15 passed; 0 failed

# compile to a native binary via the C backend:
cargo run --release -- build examples/fib.lingo
./fib
# 0 1 1 2 3 5 8 13 21 34

# guided tour of every v0.1.5 feature:
cargo run --release -- examples/tour.lingo
```

requires rust 1.75+ (`rustup` will get you there).
no other dependencies — the bootstrap compiler is zero-dep rust.

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
python: no GC, monomorphized generics, LLVM backend, zero-cost abstractions.

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

## the 10 rules (full list in [`docs/DECISIONS.md`](docs/DECISIONS.md))

1. **indentation-based, python-shaped** — llms already speak it fluently.
2. **one loop, one error shape, one string interpolation, one comment shape.**
3. **types at signatures, inferred inside** — `fn` boundaries always typed.
4. **errors are values:** `T ! E` with `?` to propagate. no exceptions, ever.
5. **explicit allocators:** any fn that may allocate takes `alloc: &Allocator`.
6. **no implicit conversions, no truthiness, no shadowing, no default args.**
7. **keyword args required when a fn has >2 parameters.**
8. **structured concurrency only** — `nursery`, no `async fn`, no goroutines.
9. **traits for behaviour, structs for data.** no inheritance.
10. **LLVM backend + monomorphized generics** → target: within 10% of zig.

## what works today

the v0.1.0 compiler (in [`compiler/`](compiler/)) understands:

- `fn` declarations with typed parameters and return type
- `let` / `let mut` (shadowing is a compile error)
- `if` / `elif` / `else`
- `for i in 0..N:` (range loops)
- `return`, `break`, `continue`
- arithmetic, comparison, boolean ops, `**`, `%`
- function calls with positional and keyword arguments
  (keyword args **required** when a fn has >2 params)
- `print(...)`, ints, floats, strings, bools

what's coming in v0.1.x: structs, enums, traits, generics, error types,
`?` propagation, explicit allocators, f-strings, `match`, the stdlib.
see [`ROADMAP.md`](ROADMAP.md).

## examples

- [`examples/hello.lingo`](examples/hello.lingo) — hello world ✅ runs
- [`examples/fib.lingo`](examples/fib.lingo) — recursion + loops ✅ runs
- [`examples/wordcount.lingo`](examples/wordcount.lingo) — file io, hashmap, errors (preview — needs v0.2)
- [`examples/http.lingo`](examples/http.lingo) — tiny http server, structured concurrency (preview — needs v0.2)

## docs

- [`docs/DECISIONS.md`](docs/DECISIONS.md) — every committed rule, in one place
- [`docs/DESIGN.md`](docs/DESIGN.md) — *why* the rules look this way
- [`docs/SYNTAX.md`](docs/SYNTAX.md) — full syntax reference (v0.1)
- [`docs/GRAMMAR.bnf`](docs/GRAMMAR.bnf) — formal grammar sketch
- [`ROADMAP.md`](ROADMAP.md) — what gets built, in what order
- [`compiler/README.md`](compiler/README.md) — how the bootstrap compiler works

## roadmap (short)

- **v0.1** — frontend (lexer, parser, type checker, tree-walking interpreter), in rust. *(lexer + parser + interp live; rest in progress)*
- **v0.2** — LLVM backend, single-file native binaries.
- **v0.3** — minimal stdlib (io, fs, str, vec, map, iter, time, net, json).
- **v0.4** — `lingo fmt`, `lingo lsp`, `lingo test`, package manager.
- **v1.0** — self-hosted compiler.

## non‑goals

- object inheritance, exceptions, null, implicit conversions, function colors.
- macros that rewrite syntax (the compiler handles generics; no user `comptime` in v0.1).
- a giant batteries‑included stdlib — small core, good package manager.
- being a php or a ruby — we don't want 14 ways to write a `for` loop.

prs and issues welcome — especially on syntax. the worst time to change a
language is *after* people write code in it.
