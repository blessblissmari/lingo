# playground — audit-driven sample projects

These four programs were written end-to-end on top of `v0.3.8` to see
how the language *feels* in real use.  The audit they produced drove
the v0.3.9 ergonomics fixes (ternary `if-then-else`, compound-assigns,
tail-position auto-`?`).

| file | what it exercises | run |
| --- | --- | --- |
| `01_fizzbuzz.lingo` | range loops, `%`, simple branches, `to_str(n)` | `lingo 01_fizzbuzz.lingo` |
| `02_calc.lingo` | CLI args, `T ! E`, `?`, match-driven dispatch | `lingo 02_calc.lingo 12 + 30` |
| `03_todo.lingo` | persistent state via `read_file`/`write_file`, `vec[Struct]` | `lingo 03_todo.lingo add "buy milk"` |
| `04_adventure.lingo` | finite-state machine on `enum Room`, inventory `vec[Item]`, persisted world | `lingo 04_adventure.lingo look` |

`02`/`03`/`04` use `args()` and file IO, both of which are interp-only
today; the C backend will reject `lingo build` until those builtins
land natively.  `01_fizzbuzz` builds and runs as a native binary
(`lingo build 01_fizzbuzz.lingo && ./01_fizzbuzz`).
