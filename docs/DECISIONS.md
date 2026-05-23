# lingo — committed decisions

> the rule: when in doubt, pick the option that makes the *reader's* life easier,
> even at the cost of the *writer's*. an llm agent is a reader 90% of the time.
>
> every entry here is **decided, not negotiable as a default**. you can still open
> an issue to change a decision, but if you write code today, write it against
> these rules.

---

## 1. memory model — **explicit allocators (zig-style)**

- there is no borrow checker. there are no lifetimes in signatures.
- any function that may allocate **must** take `alloc: &Allocator` as a
  parameter. you can see allocation from the call site.
- scope-bound resources are freed by `defer`. RAII does not exist.
- stack values, structs by value, fixed arrays — never allocate, never need an
  allocator.

**why this and not rust-style ownership:** rust's borrow checker is powerful
but its error messages and lifetime annotations are not "obvious". explicit
allocators put one rule in one place: *if you see `alloc:` in the signature,
the function may allocate*.

```lingo
fn read_file(path: str, alloc: &Allocator) -> string ! IoError:
    ...

fn main() ! IoError:
    let gpa = Gpa.new()
    defer gpa.deinit()
    let text = read_file("hello.txt", &gpa)?
    print(text)
```

## 2. errors — **one named error type per function**

- a function returns `T ! E` where `E` is exactly one type.
- if you need multiple error sources, define an `enum` that wraps them. one
  shape, always.
- `?` propagates. that is the only sugar.
- no exceptions. no panics in normal control flow.

**no union syntax `! A | B`.** that was in the draft. it's out. one error type
is more obvious than a set.

```lingo
enum AppError:
    Io(IoError)
    Parse(ParseError)
    BadConfig(str)

fn load(path: str, alloc: &Allocator) -> Config ! AppError:
    let bytes = fs.read(path, alloc)?     # auto-wraps IoError -> AppError.Io
    return Config.parse(bytes)?           # auto-wraps ParseError -> AppError.Parse
```

(the auto-wrap requires a `From` impl. one obvious mechanism, used everywhere.)

## 3. concurrency — **structured nursery only**

- every function is a normal function. there is no `async fn`. there is no
  function color split.
- to run things in parallel, open a `nursery`. tasks spawned in a nursery are
  joined or cancelled when the nursery's scope exits.
- no detached tasks. no global executor. no callback chains.

```lingo
fn fetch_all(urls: &[str], alloc: &Allocator) -> [Response] ! HttpError:
    with nursery() as n:
        let tasks = urls.map(|u| n.spawn(|| http.get(u, alloc)))
        return n.join_all(tasks)?
```

**no goroutines either:** goroutines are detached by default. that's not
obvious — it leaks resources silently. nursery makes the lifetime of every
task visible in the code.

## 4. implementation language for the bootstrap compiler — **rust**

- best parsing ecosystem (logos, chumsky, lalrpop).
- best LLVM bindings (inkwell).
- best error-message tooling (ariadne, miette).
- boring, mature, single-binary.

self-host in lingo at v1.0. nothing before that.

## 5. name — **`lingo`** (for now)

- repo is `lingo`, cli is `lingo`, extension is `.lingo`.
- not changing the name during phase 0 — it's a distraction.
- if the .com / npm namespace becomes a blocker before v1.0, we revisit *once*.

---

## the "make it obvious" rules

these come up in every file. listing them here so they're easy to grep.

### no shadowing

every `let` must introduce a name not in scope. shadowing is a compile error.

```lingo
let x = 1
let x = 2   # error: `x` already in scope
```

(in the draft this was "allowed in inner scopes only". it's out completely.)

### no default arguments

every parameter is passed explicitly at the call site.

```lingo
fn greet(name: str, greeting: str) -> str:
    return f"{greeting}, {name}"

greet("artem", "hello")            # ok
greet(name: "artem", greeting: "hi")  # also ok — keyword form
```

(in the draft we had `name: str = "world"`. it's out. you write `greet("artem", "hello")`.)

### keyword args at the call site

allowed for any parameter, any time. for functions with more than two
parameters, **required**:

```lingo
fn open(path: str, mode: Mode, alloc: &Allocator) -> File ! IoError:
    ...

# wrong:
open("f.txt", .Read, &gpa)
# right:
open("f.txt", mode: .Read, alloc: &gpa)
```

reading a 3-arg call site without keywords is guesswork. that's not obvious.

### no truthiness

`if`, `while`, `?` require a `bool`. `if xs:` is a compile error. write
`if xs.len() > 0:`.

### no implicit conversions

`u8 + i32` is a compile error. write `(a as i32) + b`.

### one loop shape

```lingo
for x in iter:
    ...
```

no `while`, no `loop`, no `do…while`. infinite loops are `for _ in forever:`.

### one debug-print form (v0.1.29)

`print(value)` renders a deterministic, llm-readable debug form:

- **structs**: `Name{field: value, field: value, ...}` in *declaration order*
  (not alphabetical), so a single field move in the struct definition is
  visible at every print site.
- **enums**: `Name.Variant` for nullary, `Name.Variant(payload, ...)` for
  data-bearing variants.
- **strings inside structs / enums**: rendered **unquoted** (`name: ada`),
  same as bare `print(s)`. quote in an f-string if you need disambiguation.
- **f64**: shortest decimal that round-trips back to the same double (rust's
  `Display for f64`), with `.0` forced on whole-valued doubles
  (`5.0`, never `5`) so floats stay visually distinct from ints.
- **`vec[T]`**: `vec[a, b, c]` with each element formatted recursively.

both the interpreter and the C backend produce byte-identical output;
adding a backend without parity is a compile-error-grade regression.

extended in v0.2.0: error strings from `T ! str` builtins also obey
the parity rule. `int(s) -> int!str` renders failures as
`int: can't parse "<rust-debug-repr>"`, where the repr is exactly
`format!("{:?}", s)` — the C backend's `lingo_str_debug_escape`
runtime helper mirrors rust's `Debug for &str` byte-for-byte
(`"..."` wrapping, `\"`/`\\`/`\n`/`\t`/`\r`/NUL escapes, `\xNN` for
other ASCII control chars, non-ASCII bytes pass through).

extended in v0.2.1: `Opt[T]` renders as **the inner value's display**
when present, and **`none`** (lowercase, no quotes) when absent.  No
`Some(...)` wrapper text — same intent as `Value::display` everywhere
else.  Both backends agree byte-for-byte; the C backend uses a per-T
`lingo_opt_<T>_str` runtime helper spliced after `lingo_fmt_alloc`,
the interpreter routes through `Value::display` directly.  This is
the v0.2.1 ground truth for `match opt: some(v): / none:` and the
reason `wordcount.lingo` now matches across interp + native.

### one string interpolation form

```lingo
let s = f"hello, {name}, version {1.0}"
```

no `%`-formatting, no `.format()`, no concatenation operator on owned strings
(write `string.join([...])` or use f-strings).

### one literal form per type

- ints: `42`, `0xff`, `0b1010`, `0o777`, `1_000_000` (underscores for grouping).
- floats: `3.14`, `1e9`. no `0.` or `.5` — write `0.0` and `0.5`.
- strings: `"..."` and `"""..."""`. no `'...'` for strings (single quotes
  are `char`).
- chars: `'a'`. one char, always.

### one comment shape, one doc-comment shape

```lingo
# line comment
## doc comment (attached to the next declaration)
```

no `/* … */`. no `//`. no doctests-as-comments. doc comments are markdown.

### no user-facing comptime in v0.1

generics work (`fn first[T](xs: &[T]) -> option[T]`), but they're handled by
the compiler — you don't write `comptime T: type`. user-facing `comptime` is
deferred to v1.1, after we know what's actually needed.

### no operator overloading except via named traits

`+` is `Add.add`. `==` is `Eq.eq`. `<` is `Ord.cmp`. you implement the trait,
not the operator. no surprise behaviour for `+`.

### no inheritance

structs hold data. traits define behaviour. `impl Trait for Struct`. that's
it.

### no globals except `const`

```lingo
const MAX_USERS = 1000     # ok
let mut counter = 0        # at module level: compile error
```

mutable globals make a function's behaviour depend on something not in its
signature. that's not obvious.

### no implicit `self`

methods take `self` as a named parameter. always.

```lingo
impl Point:
    fn dist(self, other: Point) -> f64:
        ...
```

calling `p.dist(q)` is sugar for `Point.dist(p, q)`. one rule.

### no null

use `option[T]`. `?` on an `option` propagates `none`.

### everything is `snake_case` or `PascalCase`

- types, traits, enums: `PascalCase`.
- everything else (fns, vars, fields, modules): `snake_case`.
- constants: `SCREAMING_SNAKE_CASE`.

no exceptions, including stdlib. `Vec` is a type → `vec[T]` (lowercase
keyword constructor, but the type is spelled `Vec[T]` if referenced).
actually, see below.

### types: pascal case for the type, lowercase keyword for builtins

`vec`, `map`, `set`, `option`, `result`, `str`, `string`, `bool`, the
numeric types — are spelled lowercase because they're built into the
grammar. user-defined types are `PascalCase`.

this is the one place we accept *two* shapes — but the split is mechanical:
"is it a keyword? lowercase. otherwise PascalCase." an llm gets this right
the first time.
