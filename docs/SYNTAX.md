# lingo — syntax sketch

this is the *target* syntax for v0.1. nothing here is final.

## comments

```lingo
# line comment
## doc comment (attached to the next declaration)
```

## values and bindings

```lingo
let x = 42            # immutable, type inferred (i32 here)
let mut y = 0         # mutable
let z: i64 = 100      # explicit type
const PI = 3.14159    # compile-time constant, must be a literal expression
```

no `var`, no `:=`, no `const` shadowing rules to learn. `let` vs `let mut` is
the whole story. shadowing is allowed in inner scopes only.

## primitive types

```
bool                    true | false
i8 i16 i32 i64 isize    signed ints
u8 u16 u32 u64 usize    unsigned ints
f32 f64                 floats
str                     immutable utf-8 string (slice)
char                    a unicode codepoint (u32)
```

no implicit conversions between them. write `as`:

```lingo
let n: i64 = (x as i64) + 1
```

## collections

```lingo
let xs: [i32; 4] = [1, 2, 3, 4]   # fixed array (stack)
let v: vec[i32] = vec[1, 2, 3]    # growable, heap-allocated
let m: map[str, i32] = map{"a": 1, "b": 2}
let s: set[i32] = set{1, 2, 3}
let t: (i32, str) = (1, "one")    # tuple
```

## functions

```lingo
fn add(a: i32, b: i32) -> i32:
    return a + b

fn greet(name: str = "world") -> str:
    return f"hello, {name}"
```

- all params typed.
- return type required (use `-> none` for "no return value").
- default args allowed; keyword args at call site allowed.
- no overloading, no variadics except `...args: [T]`.

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

no `while`, no `loop`, no `do…while`. infinite loops are `for _ in forever:`.
`break` and `continue` exist. labeled break is `break :outer`.

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

fn run() ! ParseError | IoError:
    let p = parse_u16(read_line()?)?     # ? propagates either error
    print(p)

# explicit handling
match parse_u16(input):
    ok(n):              print("got", n)
    err(ParseError.OutOfRange): print("too big")
    err(e):             print("other:", e)
```

## options

```lingo
let maybe: option[i32] = some(5)
let none_val: option[i32] = none

match maybe:
    some(x): print(x)
    none:    print("nothing")

let v = maybe.unwrap_or(0)
let v = maybe?      # propagates none as an early return (in option-returning fns)
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

constructors are just `T{field: value, ...}`. no `__init__`.

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

generics are monomorphized. type parameters in `[]` to avoid `<>` ambiguity.

## comptime

```lingo
fn sized_buf[comptime N: usize]() -> [u8; N]:
    return [0; N]

# compile-time branch
if comptime target.os == .linux:
    use linux_specific
```

## memory and allocators

```lingo
fn main():
    let arena = Arena.new()
    defer arena.deinit()

    let buf = arena.alloc[u8](1024)
    # buf is freed when arena.deinit() runs at scope end
```

stack values, struct literals, fixed arrays — no allocator needed.
heap collections (`vec`, `map`, owned strings) take an allocator explicitly,
or use the thread's default allocator if none is given.

## modules

```lingo
# in file `math.lingo`
fn square(x: f64) -> f64:
    return x * x

# in another file
use math
let s = math.square(2.0)

# selective import
use math.{square, cube}
let s = square(2.0)
```

one file = one module. directory with `mod.lingo` = package.

## strings

`str` is a utf-8 slice. `string` is an owned, growable utf-8 buffer. one
literal form, one interpolation:

```lingo
let name = "lingo"
let s = f"hello, {name}, version {1.0}"
let multi = """
multiline
string
"""
```

## what's not in v0.1

- pattern guards in match
- nested patterns
- async/await syntax (we have nurseries + spawn)
- macros (we have comptime)
- operator overloading (we have traits)
- variadic generics
- HKTs (and probably never)

## minimal reference grammar

a sketch of the lexer/parser grammar lives in `compiler/grammar.bnf` once the
compiler exists. for now, "indent-significant, like python; expressions are
c-like; declarations start with a keyword (`fn`, `let`, `struct`, `trait`,
`impl`, `enum`, `use`, `const`)".
