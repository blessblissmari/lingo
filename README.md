<div align="center">

# 🦜 lingo

**a fast coding language for ai.**
fast as zig. simple as python. loved by llm agents.

</div>

> ⚠️ **status:** design phase. no compiler yet — only the spec, examples and a roadmap.
> if you have opinions about the syntax, open an issue.

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

but lingo also has to be *fast*. so the runtime model is closer to zig than to python:
no GC by default, monomorphized generics, LLVM/QBE backend, zero-cost abstractions.

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
# errors are values, never exceptions
fn parse_port(s: str) -> u16 ! ParseError:
    let n = int.parse(s)?
    if n < 0 or n > 65535:
        return err(ParseError.OutOfRange)
    return n as u16

fn main() ! Error:
    let port = parse_port(env("PORT") or "8080")?
    print("listening on", port)
```

## design principles

1. **indentation-based, python-shaped** — llms already speak this dialect fluently.
2. **one way to do it** — one loop (`for x in …`), one error shape (`! E` + `?`), one string type.
3. **types at boundaries, inferred inside** — `fn` signatures and struct fields are typed; locals use `let`.
4. **no hidden allocation** — `[]`, `{}`, string concat, etc. are either stack values or require an explicit allocator.
5. **errors as values** — `!` in the return type, `?` to propagate. no exceptions, no panics for normal flow.
6. **no implicit conversions** — `i32` and `u16` never auto-cast. you write `as`.
7. **deterministic compilation** — same code, same machine code. no hidden ordering, no nondeterminism.
8. **first-class docstrings + intent comments** — every `fn` can carry an `@intent` line the compiler preserves for tooling and llms.

read the full rationale in [`docs/DESIGN.md`](docs/DESIGN.md).
read the syntax sketch in [`docs/SYNTAX.md`](docs/SYNTAX.md).

## roadmap

see [`ROADMAP.md`](ROADMAP.md). short version:

- **v0.1** — frontend: lexer, parser, type checker. interpreter for tests.
- **v0.2** — llvm backend, single‑file binaries, no stdlib.
- **v0.3** — minimal stdlib (io, str, vec, map, fs).
- **v0.4** — package manager, language server, formatter.
- **v1.0** — self‑hosted compiler.

## examples

- [`examples/hello.lingo`](examples/hello.lingo) — hello world
- [`examples/fib.lingo`](examples/fib.lingo) — recursion + loops
- [`examples/wordcount.lingo`](examples/wordcount.lingo) — file io, hashmap, errors
- [`examples/http.lingo`](examples/http.lingo) — a tiny http server sketch

## non‑goals

- object inheritance (we have structs + traits, that's it)
- macros that rewrite syntax (we have `comptime`, like zig)
- a giant batteries‑included stdlib (small core, good package manager)
- being a php or a ruby — we don't want 14 ways to write a `for` loop

## status

🚧 nothing is real yet. this repo is the *spec*. the compiler is next.

prs and issues welcome — especially on the syntax. the worst time to change a language is *after* people write code in it.
