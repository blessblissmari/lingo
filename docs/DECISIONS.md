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

extended in v0.2.2: error-type coercion is **explicit**, not implicit.
`?` continues to require `inner_e == caller.raises.1` exactly — there
is no silent "lift `str` into your enum" behaviour.  Instead, callers
write `expr? else <fallback>` to opt in: the `<fallback>` value is
evaluated in the err arm and raised as the caller's error type, while
the original inner err is discarded (after evaluation, in case of
side effects).  This keeps the type system local — no global trait
lookup or `From[A] for B` instance pool — and makes the loss of inner
detail visible at the call site.  A real `from`-style trait can be
added later without breaking the sugar.

extended in v0.2.3: the `from`-style trait *was* added — `impl From[E1]
for E2:` registers a `fn from(e: E1) -> E2` that `?` consults when the
inner err's type doesn't match the caller's `raises.1`.  This makes
plain `int(s)?` work inside a fn raising `ParseErr`, no `else`
annotation per call site.  Both forms coexist by design: the v0.2.2
sugar `? else <value>` still wins per call site when it's present, so
a single odd-one-out call site can override the trait without
deleting the impl, and the type system stays *local* — no implicit
chains, no multi-step `From` searching, the lookup is a single direct
hit on the `(E1, E2)` pair.  If no `From` impl is in scope and no
`else` is supplied, the existing diagnostic fires (now suggesting
both fixes).  `From` is a built-in/magic trait (no user `trait From`
decl needed); when we add generic-trait support generally, this
machinery folds into it.

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

---

## v0.2 — decisions added during the 0.2.x line

every entry below was implemented and shipped between v0.2.0 and v0.2.7.
the form is the same as everywhere else in this file: one rule, one
mechanism, no exceptions.

### parsing builtins return a real fallible: `int(s) -> int ! str`, `float(s) -> float ! str` (v0.2.0, v0.2.4)

```lingo
let n: int = int("42")?           # ok, n = 42
let bad: int = int("oops")?       # raises str "oops"
let f: float = float("3.14")?     # ok
```

- both interp and native back ends share one shape: `T ! str` with the
  *input* as the message.  no separate `ParseError` enum, no nested
  `Result`, no two-step "is it a number first?" — one call returns
  either the value or the offending string.
- consequence: a typo in source goes through the same `?` propagation
  path as any other domain error.  no special-casing of parse failures
  in user code.

### `map.get(k) -> Opt[T]` (v0.2.1)

```lingo
let m = map[str, int]{}
match m.get("missing"):
    Some(v): print(v)
    None:    print("not there")
```

- `Opt[T]` is the language's option type.  `map.get` is the *one* place
  where it's exposed in the v0.1 stdlib (before v0.2 there was a native
  C-backend quirk: missing key returned `0`).
- decision: lift the quirk.  one shape across backends — even if the C
  backend has to thread an extra discriminant.  reader doesn't care.

### `? else <expr>` — error-type coercion at the call site (v0.2.2)

```lingo
fn parse_port(s: str) -> int ! str:
    return int(s)? else "bad port"
```

- when calling `f()? ` inside a function whose declared raise type
  doesn't match `f`'s raise type, you must say *how* to coerce.  `else`
  is the one form.
- this is sugar.  it's only there because we wanted the more general
  mechanism in v0.2.3 (`impl From[E1] for E2:`) to feel optional, not
  mandatory, for the small cases.

### `impl From[E1] for E2:` — auto-wrapping `?` (v0.2.3)

```lingo
enum AppError:
    Io(IoError)
    Parse(str)

impl From[IoError] for AppError:
    fn from(e: IoError) -> AppError:
        return AppError.Io(e)

fn load(path: str) -> Config ! AppError:
    let bytes = fs.read(path)?    # IoError auto-wraps via From impl
    return parse(bytes)?
```

- this is the "obvious mechanism" promised in §2 (one named error type
  per function).  `?` looks up `From[source_E] for target_E` and calls
  it implicitly.  no operator overloading, no implicit conversion ladder
  — exactly one trait, exactly one direction.
- `From` was originally parser-magic.  v0.2.5 made it a regular generic
  trait, declared through the same machinery as anything user code
  writes.  *one* path for built-in and user-defined generic traits.

### user-defined generic traits — `trait Foo[T1, T2, ...]:` (v0.2.5)

```lingo
trait Bag[T]:
    fn put(self, v: T) -> Self
    fn first(self) -> T

impl Bag[int] for IntBag:
    fn put(self, v: int) -> IntBag: ...
    fn first(self) -> int: ...
```

- bracket type-params declared once on the trait, supplied per-impl as
  `impl Foo[A1, A2] for Receiver:`.  arity is checked uniformly.
- before v0.2.5 the only generic trait was the parser-magic `From[E]`.
  after v0.2.5 user code uses the same mechanism, and `From` is just
  one declaration in the prelude.

### trait method signature substitution (v0.2.6)

- when an `impl Trait[A1, A2] for Target:` block is resolved, each
  trait method's declared signature is substituted (`type_params[i] ->
  trait_args[i]` and `Self -> Target`) and matched structurally against
  the impl method's signature.  per-param types, return type, and
  `! E` raises clause all compared — including nested args like
  `vec[T]` becoming `vec[int]`.
- diagnostics are shaped per case:
  - ``method `Encoder.encode` for `IntEnc`: parameter `v` expected `int`, got `str` ``
  - ``method `Encoder.encode` for `IntEnc`: return type expected `str`, got `int` ``
  - ``method `Parse.parse` for `P`: raises clause expected `str`, got `int` ``
- the alternative ("lenient conformance — only check method names") was
  what v0.2.5 actually shipped.  it let typos in impl method
  signatures silently miscompile.  this is the rule: at resolve time,
  the impl block must structurally match the trait, or the resolver
  refuses to lower it.

### one source of truth for type-equality + substitution (v0.2.6 plumbing)

- `subst_typeref`, `typeref_eq`, `typeref_display`, `build_trait_subst`,
  `check_trait_method_sig` all live in `ast.rs` and are used identically
  by `interp.rs` and `codegen_c.rs`.  there is no second implementation
  of "are these two types the same?" — adding one would be a layering
  violation.  if a future check needs to compare types, it calls
  `typeref_eq` (or extends it, in one place).

### default-impl methods skip signature checks (v0.2.6 corollary)

- if an impl block omits a method that the trait provides as a default
  body, the resolver uses the trait method directly.  there is nothing
  to compare — the body and signature are both the trait's.  this is
  *not* a hole in the signature check; it's the absence of an impl
  method to check against.

### no overloading, no SFINAE, no specialization

- a generic trait is parameterised by its type args.  two impls can
  exist for the same trait only on different `(trait_args, target)`
  tuples.  there is no "more specific wins".  there is no
  `impl<T> Foo[T] for Bar where ...:`.  the resolver looks up by
  structural equality of `(trait_name, trait_args, target)` and finds
  exactly zero or one match.
- this rules out a class of bugs (which impl did i actually get?) at
  the cost of expressivity we don't need yet.


## v0.3 — decisions added during the 0.3.x line

### modules: one file = one module, dotted paths, alias optional

- `import foo` reads `foo.lingo` next to the entry file.
- `import foo.bar` reads `foo/bar.lingo` next to the entry file — the
  dots map directly to directory separators.  there is no module
  search path, no `LINGO_PATH`, no current-dir-vs-file-dir
  confusion: everything is relative to the file that holds the
  `import`.  the same dotted path means the same file no matter
  which module reads it.
- `import foo.bar as b` introduces the alias `b`; without `as` the
  alias is the last dotted segment (`bar`).
- the alias is **the only way** to reach another module's names.
  `b.fn()`, `b.CONST`, and `b.MyEnum.Variant` work; bare `fn()` only
  resolves locally.  reading code, you can see at a glance which
  module a name came from — that's worth more than the keystrokes.
- imports are **resolved before any other pass runs**.  the resolver
  flattens every transitively-reachable file into one `Program` by
  prefixing every non-entry module's top-level names with `lm{i}__`
  and rewriting every `alias.name` access in any module to the
  matching prefixed name.  the interpreter and the C backend never
  see an `Item::Import` — they keep working on one flat program,
  exactly as in v0.2.x.
- **deferred to v0.3.x:** cross-module *type references*
  (`fn f() -> bar.Point`) and cross-module struct literals
  (`bar.Point{ x: 1 }`).  workaround today: write a constructor
  function in `bar` and call it from the entry module.  this is
  enough for real programs to live in more than one file, and we
  can add the parser surface for `bar.Point` later without
  changing today's surface area.
- **no re-exports, no `import *`, no privacy modifiers**.  one
  module exports every top-level name it declares; if you wanted
  fewer names visible, you'd be writing a smaller module.
- **cycle detection is mandatory** — `a.lingo` imports `b.lingo`
  and `b.lingo` imports `a.lingo` is a hard error, not a runtime
  surprise.  the diagnostic names the cycle chain (`a.lingo ->
  b.lingo -> a.lingo`) so the reader knows exactly which file to
  edit.
- **duplicate aliases inside one file** (`import foo` then
  `import bar as foo`) is a hard error.  silently letting one
  shadow the other would defeat the "you can see which module
  this name came from" rule.

### name mangling is a backend detail, not a surface feature

- top-level names in non-entry modules become `lm0__foo`, `lm1__foo`,
  …  the prefix is deterministic (modules are sorted by their
  assigned prefix when flattened), so re-running the compiler on
  the same inputs produces byte-identical C.
- users never see, write, or import a mangled name.  the prefix
  exists only to keep the flat program collision-free; if you want
  to call `math.add` you write `math.add`.

### cross-module type refs and struct literals are dotted, one hop only (v0.3.1)

- `fn f() -> bar.Point`, `let p: bar.Point = ...`, `vec[bar.Point]`
  and `bar.Point{x: 1}` all work in v0.3.1.  this finishes the
  v0.3.0 modules surface — code in one file can now name and
  construct types from another file without an intermediate
  constructor helper.
- the parser accepts exactly **one** `.IDENT` suffix after a
  type-position identifier.  deeper paths like `a.b.Point` are a
  hard parse error (`cross-module type refs are one hop only`).
  reason: lingo modules don't nest.  `import a.b` already means
  "the file `a/b.lingo` reached through one alias", not "module
  `b` inside module `a`".  letting the type surface suggest
  otherwise would confuse the reader.
- in expression position, `alias.Name{...}` is consumed as a
  struct literal exactly when the three-token lookahead is
  `Dot IDENT(uppercase) LBrace`.  every other `alias.thing` form
  (field access, function call, enum-variant access) still goes
  through the regular postfix path — the new branch only fires
  when there is unambiguously a struct literal to build.
- the resolver rewrites every `alias.Name` reference to the flat
  `lm{i}__Name` form before the interp / C backend runs, so neither
  backend ever sees a dotted name.  this is the same approach
  v0.3.0 took for cross-module function and constant references —
  modules stay a *front-end* concern.
- an unknown alias (`other.Point` when `other` was never
  imported) is rejected at resolver time with a precise
  diagnostic (``cannot resolve `other.Point`: `other` is not an
  import in this module``).  no silent passthrough.

### `==` is structural for user types, but only when the user types are made of structural things (v0.3.2)

- `Point{x: 1} == Point{x: 1}` is now `true`.  before v0.3.2 it
  was a type error — the operator only handled `int`/`bool`/
  `str`/`float`.  match-on-enum already used structural eq
  internally (via `values_eq`), so this just lifts that same
  rule to the operator surface.
- struct eq: same nominal type, then field-wise `==` (recursing
  through struct/enum/vec fields).  enum eq: same `tag` first,
  then payload-wise on the matched variant.  `vec[T]` eq: same
  `len`, then element-wise.  field name order is the struct's
  *declared* order; users can rely on that for byte-identical
  C output (already pinned by the audit).
- `Map`, `Result`, `Opt` deliberately do **not** get structural
  eq.  ordering on a hash-table is the wrong default (you'd
  need to canonicalise, which is footgunny), and `Result`/`Opt`
  comparisons almost always want a `match` to discriminate
  variant first — adding `==` would invite "is this Ok?" code
  written as `r == Ok(...)` instead of pattern-matching, which
  reads worse and skips the value bind.
- when a struct or enum holds a non-eq-able field/payload
  (today: only the deliberately-excluded `Map` / `Result` /
  `Opt`), the C backend still emits the `lingo_eq_<T>` helper
  (so unrelated code that mentions `T` keeps compiling), but
  the body is a permanent `return false`.  any actual `==` on
  such a value is rejected at the **call site** with a
  localized diagnostic that names the offending field — the
  error points at the user's `==`, not at the synthesised
  helper.
- mixed-kind comparisons (`p == 1` where `p: Point`) stay
  errors, because the existing primitive arms in `bin_op`
  catch the type mismatch before the structural path runs.
  this is why the interp short-circuit only fires when at
  least one side is a compound *and* both sides are the same
  compound kind.
