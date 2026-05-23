<div align="center">

# 🦜 lingo

**a fast coding language for ai.**
fast as zig. simple as python. loved by llm agents.

</div>

> ⚠️ **status:** design phase. no compiler yet — only the spec, examples and a roadmap.
> all design decisions are committed in [`docs/DECISIONS.md`](docs/DECISIONS.md).
> disagree? open an issue.

---

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

## examples

- [`examples/hello.lingo`](examples/hello.lingo) — hello world
- [`examples/fib.lingo`](examples/fib.lingo) — recursion + loops
- [`examples/wordcount.lingo`](examples/wordcount.lingo) — file io, hashmap, errors
- [`examples/http.lingo`](examples/http.lingo) — a tiny http server, structured concurrency

## docs

- [`docs/DECISIONS.md`](docs/DECISIONS.md) — every committed rule, in one place
- [`docs/DESIGN.md`](docs/DESIGN.md) — *why* the rules look this way
- [`docs/SYNTAX.md`](docs/SYNTAX.md) — full syntax reference (v0.1)
- [`docs/GRAMMAR.bnf`](docs/GRAMMAR.bnf) — formal grammar sketch
- [`ROADMAP.md`](ROADMAP.md) — what gets built, in what order

## roadmap (short)

- **v0.1** — frontend (lexer, parser, type checker, tree-walking interpreter), in rust.
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
