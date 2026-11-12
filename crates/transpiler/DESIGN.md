# Transpiler design

What the transpiler should produce, how its code is arranged, and where a
change belongs. The [catalogue](CATALOGUE.md) lists the rules one by one; this
file says what they share.

## The goal

The translation is Rust that a Rust programmer would write for the same
program: values that Rust owns and frees, borrows where the program allows
them, `Option` for a pointer that may be NULL, and the standard library in
place of C's. It is not C kept alive in safe Rust. There is no emulated
memory, no table of objects with indices standing in for addresses, and no
runtime that copies the C library.

Two consequences shape everything else:

- **A rule or a refusal.** A program is translated in full by rules, or it is
  refused with the construct that has no rule and its line:
  `unsupported: a pointer to a pointer, at line 12`. No rule falls back to a
  translation that always works. The refusals of the development programs are
  the list of what to build next.
- **Types come from the whole program.** Whether `int *p` is `&i32`,
  `&mut [i32]`, `Box<i32>` or `Option<Vec<i32>>` depends on everything the
  program does with `p`. The analysis answers that once, before any rule
  runs.

## Stages

```text
Clang's JSON -> clang -> c::Program -> facts -> normalise -> facts -> lower -> rust -> text
```

| Stage | File | Does | Knows |
| --- | --- | --- | --- |
| Front end | `src/clang.rs` | Reads Clang's JSON into the C tree | Clang's JSON; nothing about Rust |
| C tree | `src/c.rs` | Types, expressions and statements with their C types, and walks over them | C |
| Analysis | `src/facts.rs` | Facts about each pointer, variable, global and function | The whole program |
| Rewrites | `src/normalise.rs` | Rewrites C to C so that one statement means one thing | The facts |
| Rules | `src/lower/` | Translates each construct into a `syn` tree | The facts, one construct at a time |
| Names | `src/names.rs` | The Rust name of each C name | Rust's naming and keywords |
| Output | `src/rust.rs` | Builds `syn` expressions and prints the file with `prettyplease` | Rust's syntax |

The rules are split by subject: `lower/mod.rs` (the program, records, globals,
functions, `main`), `types.rs`, `expr.rs`, `stmt.rs`, `pointer.rs` and
`library.rs` (the C library).

## Where a change goes

- Clang prints a construct that the front end does not read: `clang.rs`, and
  `c.rs` if the tree has no node for it. The tree keeps what C means, not how
  Clang prints it.
- A rule needs to know something about the rest of the program (is this
  pointer ever NULL, does this function change a global, is this array
  resized): a fact in `facts.rs`. A rule never searches the program itself.
- A C idiom spans several statements and Rust says it in one (test for NULL
  and then use, a cursor that walks an array, an output parameter that is
  really a result): a rewrite in `normalise.rs`, from C to C with a new node
  in `c.rs` when C has no way to say it.
- One construct has no translation: a rule in `lower/`, with a catalogue
  entry and a test.

Keep to these:

- Build Rust as a tree. Expressions are made with the builders in `rust.rs`
  (`binary`, `method`, `cast`, `index`, …), so the printer adds the
  parentheses that precedence needs. `parse_quote!` is for statements, items
  and macros, and takes an expression only between delimiters.
- Never write Rust as text, and never patch what an earlier rule printed.
- A rule translates a C construct, not a program: no program names, outputs
  or constants from a benchmark.
- A helper function is emitted once, only in programs that use it, and only
  where the standard library has no function that behaves as C's does
  (`parse_int` for `strtol`). It is ordinary Rust, not a C library.
- A rule that would be wrong for some programs asks the analysis and refuses
  those. A wrong translation that compiles is the worst outcome; a refusal is
  the best after a pass.

## Target

| C | Rust |
| --- | --- |
| `char` | `u8`; widening goes through `i8`, since C's `char` is signed here |
| `short`, `int`, `long`, `unsigned`, … | `i16`, `i32`, `i64`, `u32`, …, as on x86-64 |
| `_Bool`, `float`, `double` | `bool`, `f32`, `f64` |
| Signed arithmetic | Rust's operators: an overflow, undefined in C, stops the program |
| Unsigned arithmetic | `wrapping_add`, `wrapping_sub`, `wrapping_mul` |
| `struct s` | `struct S` with `Clone`, and `Copy` and `Default` where its fields allow |
| `T a[N]` | `[T; N]` |
| A local without an initialiser | Starts at zero |
| A global that never changes | `const` or `static`, in upper case |
| A global that changes | A field of `Globals`, which `main` creates and each function that uses it takes as `globals: &Globals` or `&mut Globals` |
| `main` | `fn main()`, with `-> ExitCode` unless it only returns 0; `argc` and `argv` from `std::env::args()` |
| `for (int i = a; i < b; i++)` | `for i in a..b`, when the body changes neither `i` nor the bound |
| `switch` without fall-through | `match` |
| `printf` with a literal format | `print!` or `println!`, the format converted at translation time |
| `if (p)`, `if (!p) return;` | `if let Some(p) = …`, `let Some(p) = … else { … }` |
| `malloc`, `calloc`, `free` | `Box::new`, `vec![zero; n]`, `drop` |

### Pointers

A *slot* is a place that holds a pointer: a variable, a field or a function's
result. The analysis gives each slot five facts, and they decide its type:

| Fact | Set when | Effect |
| --- | --- | --- |
| `owned` | It holds memory from `malloc`, is freed, or lives in a record, a global or a result that got it from an owner | `Box` or `Vec` rather than a borrow |
| `many` | It is indexed, moved, or holds an array | `Vec` or a slice rather than one element |
| `written` | Something is stored through it, here or wherever it is handed on | `&mut` rather than `&` |
| `nullable` | NULL may reach it | Wrapped in `Option` |
| `conflict` | No single type fits, with the reason | The program is refused |

Every assignment, argument and `return` is a flow from a source into a slot,
and the facts spread along the flows until they stop changing: `written` and
`owned` go back to where the pointer came from, `many` goes both ways, and
`nullable` goes forward unless a test has ruled NULL out. A use of a pointer
converts between the types of two slots (`lower/pointer.rs`, `adapt`): a move
between owners, `&mut *b` or `as_deref_mut()` from an owner to a borrow, and
so on. What the borrow checker then rejects is refused by `rustc`, which is
the intended division of work: the analysis picks types, the compiler proves
them.

A test that the facts decide, such as `if (!p)` after a `malloc` that Rust
cannot fail, is dropped with its dead branch, and the analysis runs again
without it.

## Not yet built

The direction for each, so that the first rule for it starts in the right
place:

- **Text.** `char *` that only ever holds text is `&str` or `String`, which
  needs a fact that says so; other `char` arrays stay bytes. String literals,
  `%s` of a variable, `strlen`, `strcmp`, `strcpy` and the rest wait for it.
- **Input.** `scanf`, `fgets` and `getchar` read from `std::io::stdin().lock()`.
  `fprintf(stderr, …)` is `eprint!`.
- **Pointers to pointers and output parameters.** `T **out` that is only
  stored through is `&mut Option<Box<T>>` or a second result.
- **Cursors.** A pointer that walks an array (`p++`, `*p`) is an index beside
  the slice, by a rewrite.
- **Walking a list.** `while (p) { …; p = p->next; }` needs a narrowing for
  loops like the one for `if`; today it uses `is_some()` and `unwrap()`.
- **Shared memory.** Two slots that own the same memory do not compile: the
  second use is a use after a move. `Rc` belongs only on a slot that a fact
  proves shared, and `RefCell` only where it is also written; neither is a
  default.
- **Others.** `goto`, a `case` that falls through, pointers to functions
  (`fn` types), unions (an `enum` when a tag says which member is live),
  bit-fields, variadic functions, arrays whose length is not a constant
  (`Vec`), `%e` and `%g`, a local that is assigned before every read
  (`let x;`).

## Known weak spots

Places where a rule assumes more than the analysis proves. Each should
become a fact and a refusal:

- An owner moved out of a field or a global leaves `None` behind
  (`take()`); a C program that reads the field again before storing to it
  would see NULL in Rust.
- `%c`, `%s` of a program argument and `argv` assume ASCII; C passes any
  bytes through.
- A conversion from a floating-point number to an integer saturates in Rust
  where C's behaviour is undefined, and a NaN prints differently.

## Tests

`tests/translate/` holds one test per subject. `check` compiles the C program
with Clang and the translation with `rustc` (`-F unsafe_code -D warnings`,
overflow checks on), runs both on each input and compares output and exit
code; the test then asserts the lines that show the rule's shape, so a
translation that passes but reads worse fails the test. `tests/dev.rs` checks
that every development program is translated or refused, never a panic.
