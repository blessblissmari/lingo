# lingo — syntax reference (v0.1)

this is the target syntax for v0.1. every rule here is decided
(`DECISIONS.md`).

## file shape

- one `.lingo` file = one module.
- a directory containing `mod.lingo` = one package.
- top-level may contain only: `use`, `const`, `fn`, `struct`, `enum`, `trait`,
  `impl`, `type`.
- there are no top-level statements. there is no "script mode".

## comments

```lingo
# line comment
## doc comment (markdown, attached to the next declaration)
```

no block comments. no `//`. no `/* */`.

## bindings

```lingo
let x = 42            # immutable, type inferred
let mut y = 0         # mutable
let z: i64 = 100      # explicit type
const PI = 3.14159    # compile-time constant (literal expression only)
```

- no `var`, no `:=`.
- shadowing is a **compile error**. every `let` must introduce a fresh name.
- `const` is allowed only at module scope.

## primitives

```
bool                    true | false
i8 i16 i32 i64 isize    signed ints
u8 u16 u32 u64 usize    unsigned ints
f32 f64                 floats
str                     immutable utf-8 string slice
string                  owned, growable utf-8 buffer (allocator-backed)
char                    a unicode codepoint (u32)
```

no implicit conversions. always `as`:

```lingo
let n: i64 = (x as i64) + 1
```

no truthiness. `if x:` requires `x: bool`.

## literals

- ints: `42`, `0xff`, `0b1010`, `0o777`, `1_000_000`
- floats: `3.14`, `1e9`, `0.5` (never `.5`)
- bool: `true`, `false`
- char: `'a'`, `'\n'`, `'\u{1F44B}'`
- str: `"hello"`, `"""multi-line"""`
- f-string: `f"hello, {name}"`
- arrays: `[1, 2, 3]` (fixed) and `vec[1, 2, 3]` (growable)
- maps: `map{"a": 1, "b": 2}`
- sets: `set{1, 2, 3}`
- tuples: `(1, "one", 3.0)`

## collections

```lingo
let xs: [i32; 4] = [1, 2, 3, 4]   # fixed array, on stack
let v: vec[i32] = vec[1, 2, 3]    # growable, allocator-backed
let m: map[str, i32] = map{"a": 1, "b": 2}
let s: set[i32] = set{1, 2, 3}
let t: (i32, str) = (1, "one")    # tuple
```

`vec`, `map`, `set`, owned `string` — all take an allocator. if you write the
literal at function scope, the default allocator (the enclosing function's
`alloc:` param) is used. if there's no allocator in scope, it's a compile error.

## functions

```lingo
fn add(a: i32, b: i32) -> i32:
    return a + b

fn greet(name: str, greeting: str) -> string ! AllocError:
    return string.concat([greeting, ", ", name])

fn read_file(path: str, alloc: &Allocator) -> string ! IoError:
    ...
```

- all parameters typed. return type required (use `-> none` for "no value").
- **no default arguments.** every parameter is passed at the call site.
- **keyword args required when a fn has more than 2 parameters.**
- no overloading. no variadics (use `...args: [T]` for variadic-like).
- closures: `|x| expr` or `|x, y| { stmts; expr }`. they capture by value;
  use `&` to capture by reference.

## calling

```lingo
add(1, 2)
greet("artem", "hi")
greet(name: "artem", greeting: "hi")            # keyword form
read_file("a.txt", alloc: &gpa)                 # keyword required (3 args)
```

## control flow

```lingo
if x > 0:
    print("pos")
elif x < 0:
    print("neg")
else:
    print("zero")

for n in 0..10:
    print(n)

for (i, x) in xs.iter().enumerate():
    print(i, x)

match shape:
    Circle(r):    print("circle", r)
    Rect(w, h):   print("rect", w, h)
    _:            print("other")
```

- one loop form. no `while`, no `loop`. infinite loops: `for _ in forever:`.
- `break`, `continue` ok. labelled break: `break :outer`.
- `if` is a statement; for ternary use `if-expression`:
  `let s = if x > 0 then "pos" else "neg"`.

## errors

```lingo
enum ParseError:
    Empty
    OutOfRange
    BadChar(char)

fn parse_u16(s: str) -> u16 ! ParseError:
    if s.is_empty():
        return err(ParseError.Empty)
    ...

fn run(args: [str]) ! ParseError:
    let p = parse_u16(args[1])?
    print(p)

match parse_u16(input):
    ok(n):                      print("got", n)
    err(ParseError.OutOfRange): print("too big")
    err(e):                     print("other:", e)
```

- **one error type per function.** if you need to combine sources, make an
  `enum` and impl `From` for each underlying error.
- `?` propagates. `?` requires the enclosing fn to declare a compatible `! E`.
- no exceptions, no panics for normal flow.

## options

```lingo
let maybe: option[i32] = some(5)
let nothing: option[i32] = none

match maybe:
    some(x): print(x)
    none:    print("nothing")

let v = maybe.unwrap_or(0)
let v = maybe?                  # propagates `none` (in option-returning fns)
```

## structs and traits

```lingo
struct Point:
    x: f64
    y: f64

impl Point:
    fn new(x: f64, y: f64) -> Point:
        return Point{x: x, y: y}

    fn dist(self, other: Point) -> f64:
        return ((self.x - other.x)**2 + (self.y - other.y)**2).sqrt()

trait Shape:
    fn area(self) -> f64

struct Circle:
    r: f64

impl Shape for Circle:
    fn area(self) -> f64:
        return 3.14159 * self.r * self.r
```

- no inheritance. `impl Trait for Struct` adds methods.
- constructors are just `T{field: value, ...}`. no `__init__`.
- `self` is a named parameter. `x.f(y)` is sugar for `T.f(x, y)`.

## generics

```lingo
fn first[T](xs: &[T]) -> option[T]:
    if xs.len() == 0:
        return none
    return some(xs[0])

struct Stack[T]:
    items: vec[T]

impl[T] Stack[T]:
    fn push(self, x: T):
        self.items.push(x)
```

- monomorphized. type parameters in `[]` (avoids `<>` ambiguity with `<`).
- no user-facing `comptime` in v0.1.

## memory and allocators

```lingo
fn main():
    let gpa = Gpa.new()
    defer gpa.deinit()

    let buf = gpa.alloc[u8](1024)
    defer gpa.free(buf)

    let text = read_file("hello.txt", &gpa)?
    print(text)
```

- stack values, struct literals, fixed arrays: no allocator needed.
- anything growable (`vec`, `map`, `string`, ...) requires an allocator.
- `defer expr` runs `expr` at the end of the enclosing scope.

## concurrency

```lingo
fn fetch_all(urls: &[str], alloc: &Allocator) -> [Response] ! HttpError:
    with nursery() as n:
        let tasks = urls.map(|u| n.spawn(|| http.get(u, alloc)))
        return n.join_all(tasks)?
```

- structured. tasks are bound to a `nursery`. nursery exit = join-or-cancel.
- no `async fn`. no future. no goroutine.
- the runtime is cooperative; long blocking calls happen on a worker pool.

## modules

```lingo
# in file `math.lingo`
fn square(x: f64) -> f64:
    return x * x

# in another file:
use math
let s = math.square(2.0)

use math.{square, cube}     # selective import
let s = square(2.0)
```

one file = one module. one directory with `mod.lingo` = one package.

## strings

```lingo
let name = "lingo"
let greeting = f"hello, {name}, version {1.0:.2}"
let multi = """
multiline
string
"""
```

- `str` is a utf-8 slice (immutable, borrowed).
- `string` is utf-8 owned + growable.
- one format-spec syntax inside `{...}` (subset of python's, documented in
  the stdlib reference).

## naming conventions

- types, traits, enums: `PascalCase` (`User`, `IoError`, `Shape`).
- builtin types are keywords and lowercase: `vec`, `map`, `set`, `option`,
  `result`, `str`, `string`, `bool`, `i32`, etc.
- functions, variables, fields, modules: `snake_case` (`read_file`, `user_id`).
- constants: `SCREAMING_SNAKE_CASE` (`MAX_USERS`).
- enum variants: `PascalCase` (`Empty`, `OutOfRange`, `BadChar`).

## what's not in v0.1

- pattern guards in match
- nested patterns in match
- user-facing comptime
- macros
- operator overloading (it's only via traits)
- variadic generics
- HKTs

if you need one of these now, file an issue with the use case.
