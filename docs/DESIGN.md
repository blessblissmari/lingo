# lingo — design rationale

this document explains *why* lingo looks the way it does.
the *what* (the rules) lives in [`DECISIONS.md`](DECISIONS.md). this doc
explains the reasoning behind them.

audience: people who already know rust, zig, go, or python and want to argue.

## the three constraints

lingo has to satisfy three constraints **simultaneously**:

1. **fast** — comparable to zig/rust. AOT, no GC, native binaries.
2. **simple** — readable by a beginner, smaller than python.
3. **llm-friendly** — an agent should write correct lingo at a higher success
   rate than it writes correct rust.

most languages pick two. lingo's bet is that you can pick all three if you
give up "expressiveness flex" — multiple ways to do the same thing — and
accept that the *writer* sometimes types more so the *reader* always has
less to infer.

## what "llm-friendly" actually means

it does **not** mean "verbose" or "english-like". llms are excellent at
python already. it means:

### 1. low syntactic ambiguity

llms predict the next token. if there are five ways to write a `for` loop,
the model spreads probability mass across all five and mixes them up. lingo
has *one* shape per concept:

- one `for`. no `while`, no `loop`.
- one error mechanism: `! E` + `?`.
- one string interpolation: `f"..."`.
- one comment shape: `#`. one doc-comment shape: `##`.
- one literal per type. one constructor pattern (`T{field: ...}` or `T.new(...)`).

(full list in `DECISIONS.md`.)

### 2. local reasoning

an llm reading a function should not have to scroll up. so:

- function signatures declare *all* parameter types and the return type.
- function signatures declare *the* error type they may return (`! E`).
- a function may not capture mutable outer state implicitly — pass it in.
- there are no globals except `const`.

### 3. no hidden behaviour

every cost is visible at the call site:

- allocation? you can see `alloc: &Allocator` in the signature.
- error? you can see `! E` and `?` at the call site.
- mutation? you can see `&mut` at the call site.
- task spawn? you can see `n.spawn(...)` inside a `nursery` block.

if you can't see it, it doesn't happen.

### 4. structural, regular stdlib naming

every method on every container has the same shape:

- constructors: `T.new`, `T.with_capacity`, `T.from_slice`
- accessors: `.len()`, `.is_empty()`, `.get(i)`, `.first()`, `.last()`
- mutators: `.push(x)`, `.pop()`, `.insert(i, x)`, `.remove(i)`
- iteration: `.iter()`, `.iter_mut()`, `.into_iter()`

no `strlen` vs `len(s)` vs `s.length` schizophrenia. the model learns the
shape once.

## what "fast" actually means

target: **within 10% of equivalent zig** for cpu-bound code.

mechanism:

- **LLVM backend** for v0.2+. QBE fallback for fast debug builds.
- **monomorphized generics**, like rust — no runtime dispatch unless you
  ask for it (`dyn Trait`).
- **no GC.** memory is scope-bound + explicit allocators.
- **value types by default.** structs live on the stack. `&` and `box[T]`
  are explicit.
- **inlining hints, simd intrinsics, link-time optimisation.**
- **no exceptions, no setjmp, no hidden unwinding.** errors are values, so
  there's nothing for the runtime to unwind.

## what "simpler than python" actually means

python is simple to *read*. lingo is simple to *learn*, because there are no
surprises:

| python wart                             | lingo answer                                |
| --------------------------------------- | ------------------------------------------- |
| `self` everywhere                       | methods take `self`, but you call `x.f()`   |
| `__dunder__` for everything             | named traits with named methods (`Add.add`) |
| `__init__` vs `__new__` vs `dataclass`  | `struct` literal `T{...}`, one way          |
| mutable default args                    | no default args                             |
| late binding of closures                | closures capture by value (`&` for ref)     |
| GIL + asyncio + threading + multiproc   | one model: structured nursery               |
| `list` vs `tuple` vs `array.array`      | `vec[T]` and `[T; N]`, that's it            |
| dynamic typing surprises                | static types everywhere                     |
| 4 ways to format a string               | one: `f"hello {name}"`                      |

## why no borrow checker

rust's borrow checker is the best static memory-safety system in production
software. it is also the #1 reason people bounce off rust. and its error
messages, while better than they were, aren't *obvious*.

lingo picks the simpler model: **explicit allocators + scope-bound resources
+ `defer`**. it's a strict subset of zig's model. you give up the static
"no use-after-free" guarantee for the rust-grade case, but you get:

- no lifetimes in signatures.
- no generic-lifetime puzzles.
- one place to read to find out "does this allocate?" — the signature.

it's a trade. we made it.

if the bootstrap compiler reveals real footguns, we add an optional borrow
check pass on top of the existing model — never inside the type system.

## why structured concurrency

three options were on the table:

- **goroutines.** detached by default. great ergonomics. bad observability.
  you lose tasks. resources leak silently. **not obvious.**
- **async/await.** function colour split. half your stdlib has to be
  duplicated. **not obvious.**
- **structured concurrency (nursery / trio-style).** every task is bound to
  a lexical scope. when the scope exits, all its tasks are joined or
  cancelled. **obvious.**

we picked #3. all functions are normal. parallelism is a block, not a
keyword on every fn.

## why one error type per function

the draft had `! A | B`. it's gone. reasoning:

- "one shape per concept" applies here too. the shape of an error in a
  signature is **one type**, full stop.
- if your fn has two error sources, you define an `enum` that wraps them.
  this is one line of code and makes the union *named*, which the llm
  (and you) can refer to in docs and tests.
- with a `From<A> for Wrapped` impl, `?` auto-wraps. ergonomics are equal
  to a union, but the union has a name.

## why no user-facing comptime in v0.1

zig's `comptime` is great. it is also conceptually heavy: types as values,
functions that run at compile time, comptime branches that change the type
of subsequent code. that's a lot of "non-obvious".

generics in v0.1 are handled by the compiler — `fn first[T](xs: &[T])` works
without exposing `comptime` to the user. when v1.0 reveals a real need for
user-level metaprogramming, we add `comptime` then, with the benefit of
hindsight.

## why "make the writer type more"

every "the writer types more" rule (keyword args, no shadowing, no defaults,
explicit allocator) costs the human a few extra characters and saves the
reader a guess. since:

- one piece of code is *written* once and *read* many times,
- the agent reads it *every time* it wants to modify it,
- bugs are a function of "what the reader misunderstood",

we trade writer effort for reader certainty. always.

## decided. not "open".

this document used to have an "open questions" section. it doesn't anymore.
every question is answered in `DECISIONS.md`. if you want to change an
answer, open an issue with a concrete proposal.
