# lingo — design rationale

this document explains *why* lingo looks the way it does. the target audience is
people who already know rust, zig, go, or python and want to argue.

## the three constraints

lingo has to satisfy three constraints **simultaneously**:

1. **fast** — comparable to zig/rust. AOT, no GC by default, native binaries.
2. **simple** — readable by a beginner, smaller than python.
3. **llm-friendly** — an agent should be able to write correct lingo at a
   higher success rate than it writes correct rust.

most languages pick two. lingo's bet is that you can pick all three if you give
up "expressiveness flex" — multiple ways to do the same thing.

## what "llm-friendly" actually means

it does **not** mean "verbose" or "english-like". llms are excellent at python
already, which is neither verbose nor english. it means:

### 1. low syntactic ambiguity

llms predict the next token. if there are five ways to write a `for` loop, the
model has to spread probability mass across them, and is more likely to mix
them up. lingo has *one* loop shape:

```lingo
for x in iter:
    ...
```

ranges (`0..n`), `enumerate`, `zip`, etc. are library functions that return
iterators. no `while`, no `loop`, no `do…while`. infinite loops are
`for _ in forever:`.

### 2. local reasoning

an llm that reads a function should not need to scroll. lingo enforces this by:

- function signatures must declare *all* parameter types and the return type.
- function signatures must declare *all* errors they can return (`! E`).
- a function may not capture *mutable* outer state implicitly — you pass it in.
- there are no globals except `const`.

### 3. one shape for errors

exceptions are invisible control flow. they're famously hard for both humans
and models. lingo has **one** error mechanism, copied from rust/zig:

```lingo
fn read_config(path: str) -> Config ! IoError | ParseError:
    let bytes = fs.read(path)?
    return Config.parse(bytes)?
```

`!` lists the error types. `?` propagates. there is no try/catch — you `match`
on the result if you want to handle it. no exceptions, no panics for normal
flow. `panic` exists but is only for unrecoverable bugs (asserts).

### 4. one shape for ownership

rust's borrow checker is powerful but its error messages are a meme. zig's
manual memory is faster to learn but easier to get wrong. lingo picks a middle
path:

- **values are owned by their lexical scope.** when the scope ends, the
  destructor runs.
- **references (`&T`, `&mut T`) borrow** with a borrow checker, but the rules
  are simpler than rust: no lifetimes in signatures, only structural borrow
  checking. a reference may not outlive its scope. period.
- **explicit allocators.** anything that allocates takes an `Allocator` —
  heap, arena, page, gpa. no hidden allocations anywhere.

```lingo
fn join(parts: &[str], alloc: &Allocator) -> str:
    ...
```

this is more verbose than python but it's *predictable*, which is what an llm
needs.

### 5. no implicit conversions

`u8 + i32` is a compile error. you write `(a as i32) + b`. boolean context
requires an actual `bool` — no truthiness on ints, strings, or pointers. `if
xs:` is a compile error; you write `if xs.len() > 0:`.

### 6. structural, regular stdlib naming

every method on every container has the same name shape:

- constructors: `T.new`, `T.with_capacity`, `T.from_slice`
- accessors: `.len()`, `.is_empty()`, `.get(i)`, `.first()`, `.last()`
- mutators: `.push(x)`, `.pop()`, `.insert(i, x)`, `.remove(i)`
- iteration: `.iter()`, `.iter_mut()`, `.into_iter()`

no `strlen` vs `len(s)` vs `s.length` schizophrenia. the model only has to
learn the shape once.

## what "fast" actually means

lingo's target is **within 10% of equivalent zig** for cpu-bound code. it gets
there with:

- **LLVM backend** for v0.2+, with a QBE fallback for fast debug builds.
- **monomorphized generics** like rust — no runtime dispatch unless you ask
  for it with `dyn Trait`.
- **no GC.** memory is scope-bound (RAII) plus explicit allocators.
- **value types by default.** structs are stack-allocated. `&` and `box[T]`
  are explicit.
- **inlining hints, comptime evaluation, simd intrinsics** as zig has.
- **no exceptions, no setjmp, no hidden stack unwinding.**

## what "simpler than python" actually means

python is already very simple to *read*. lingo aims to be simpler to *learn*
by removing surprises:

| python wart                             | lingo answer                                |
| --------------------------------------- | ------------------------------------------- |
| `self` everywhere                       | methods take `self`, but you call `x.f()`   |
| `__dunder__` for everything             | traits with named methods (`Add.add`)       |
| `__init__` vs `__new__` vs `dataclass`  | `struct` literal, one way                   |
| mutable default args                    | not allowed                                 |
| late binding of closures                | closures capture by value (`&` for ref)     |
| GIL + asyncio + threading + multiproc   | one concurrency model (structured tasks)    |
| `list` vs `tuple` vs `array.array`      | `vec[T]` and `[T; N]`, that's it            |
| dynamic typing surprises                | static types everywhere                     |
| 4 ways to format a string               | one: `f"hello {name}"`                      |

## comptime, not macros

like zig, lingo has `comptime`: arbitrary code that runs at compile time.
unlike rust macros and c preprocessor, comptime code is just lingo code — same
syntax, same semantics. that means:

- generics are functions that take `comptime T: type`.
- conditional compilation is `if comptime target.os == .linux:`.
- no separate macro language to learn (and no separate language for llms to
  fail at).

## traits, not classes

no inheritance. structs hold data. traits define behavior. `impl Trait for
Struct:` adds methods. that's the whole oop story.

```lingo
trait Greet:
    fn hello(self) -> str

struct Cat:
    name: str

impl Greet for Cat:
    fn hello(self) -> str:
        return f"meow, i am {self.name}"
```

## concurrency: structured tasks

one model. inspired by trio / kotlin's structured concurrency.

```lingo
fn fetch_all(urls: &[str]) -> [Response] ! HttpError:
    with nursery() as n:
        let results = urls.map(|u| n.spawn(|| http.get(u)))
        return n.join_all(results)?
```

- tasks are bound to a `nursery`. when the nursery exits, all tasks are joined
  or cancelled.
- no detached tasks. no callback hell. no two-color functions (`async fn`
  doesn't exist — all functions are normal, the runtime is cooperative).

## what's deliberately missing

- **no inheritance** — composition + traits.
- **no exceptions** — errors are values.
- **no null** — `option[T]` is explicit, `?` works on options too.
- **no implicit `this`** — `self` is a named parameter.
- **no operator overloading** — except via traits (`Add`, `Mul`, ...).
- **no macros that rewrite syntax** — comptime is enough.
- **no significant overloading by arity** — `fn foo` and `fn foo2` if you need
  two; or use default arguments.

## open questions

these are not decided yet:

- **memory:** rust-style ownership vs. zig-style explicit allocators vs. a
  hybrid. current bet: hybrid.
- **strings:** utf-8 always, but bytes vs grapheme api split?
- **modules:** file = module, directory = package. but how does naming work
  for re-exports?
- **ffi:** must be ergonomic enough to wrap c libraries in 5 lines. design
  tbd.
- **the name.** "lingo" is fine, but it's also a domain-squatter magnet.

prs to this document are encouraged — the goal is to have something
opinionated enough to *implement* by end of phase 0.
