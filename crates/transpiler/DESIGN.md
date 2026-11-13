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
Clang's JSON -> clang -> c::Program -> join, ends -> facts -> cursors -> facts -> narrow -> facts -> lower -> rust -> text
```

| Stage | File | Does | Knows |
| --- | --- | --- | --- |
| Front end | `src/clang.rs` | Reads Clang's JSON into the C tree | Clang's JSON; nothing about Rust |
| C tree | `src/c.rs` | Types, expressions and statements with their C types, and walks over them | C |
| Analysis | `src/facts.rs` | Facts about each pointer, variable, global and function | The whole program |
| Rewrites | `src/normalise.rs` | Rewrites C to C so that one statement means one thing: `join` declares a local where it gets its first value, `ends` makes the end that `strtol` stores a result of the call, `cursors` makes a pointer that walks an array an index, `narrow` names a pointer that a test found not to be NULL | The facts, for `cursors` and `narrow` |
| Rules | `src/lower/` | Translates each construct into a `syn` tree | The facts, one construct at a time |
| Names | `src/names.rs` | The Rust name of each C name: snake case for functions, variables and fields, camel case for records, upper case for constants and enumerators. Two C names in one Rust scope never share a name | Rust's naming and keywords |
| Output | `src/rust.rs` | Builds `syn` expressions and prints the file with `prettyplease` | Rust's syntax |

The rules are split by subject: `lower/mod.rs` (the program, records, globals,
functions, `main`), `types.rs`, `expr.rs`, `stmt.rs`, `pointer.rs`,
`library.rs` (the C library) and `input.rs` (standard input).

## Where a change goes

- Clang prints a construct that the front end does not read: `clang.rs`, and
  `c.rs` if the tree has no node for it. The tree keeps what C means, not how
  Clang prints it. Types, tags and typedefs are found by name, so a program
  that gives one name two meanings in different scopes is refused.
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
| A bit-field `unsigned level : 3` | A field `level: u32`; every store keeps the low bits, `value & 7` |
| `T a[N]` | `[T; N]` |
| `T a[n]`, a local whose length is a variable | `Vec<T>`, `vec![zero; n as usize]` |
| A local without an initialiser | Declared where the block first assigns it, or by the `for` that does; otherwise it starts at zero |
| A global that never changes | `const` or `static`, in upper case |
| A global that changes | A field of `Globals`, which `main` creates and each function that uses it takes as `globals: &Globals` or `&mut Globals` |
| `main` | `fn main()`, with `-> ExitCode` unless it only returns 0; `argc` and `argv` from `std::env::args()` |
| `for (int i = a; i < b; i++)` | `for i in a..b`, when the body changes neither `i` nor the bound |
| `switch` without fall-through | `match` |
| `printf` with a literal format | `print!` or `println!`, the format converted at translation time |
| `const char *` that only holds string literals | `&str` |
| `if (p)`, `if (!p) return;` | `if let Some(p) = …`, `let Some(p) = … else { … }` |
| `malloc`, `calloc`, `free` | `Box::new`, `vec![zero; n]`, `drop` |
| `scanf("%d", &x)` | `if let Some(scanned) = scan_int() { x = scanned as i32; }` |
| `fgets(line, sizeof line, stdin)` | `read_line(&mut line)` |
| `fgets(line, size, stdin)` for a `char *line` | `read_line_within(line, size)` |
| `%s` of a `char` array | `text_of(&line)` |

### Pointers

A *slot* is a place that holds a pointer: a variable, a field or a function's
result. The analysis gives each slot eight facts, and they decide its type:

| Fact | Set when | Effect |
| --- | --- | --- |
| `owned` | It holds memory from `malloc`, is freed, or lives in a record, a global or a result that got it from an owner | `Box` or `Vec` rather than a borrow |
| `many` | It is indexed, moved, or holds an array | `Vec` or a slice rather than one element |
| `written` | Something is stored through it, here or wherever it is handed on | `&mut` rather than `&` |
| `nullable` | NULL may reach it | Wrapped in `Option` |
| `bytes` | It points to `char` that is read or written one by one, or to another type | A `char` pointer without it is `&str` |
| `argument` | `argv[i]` may reach it as the program found it, which is NULL at `argv[argc]` | A test of it for NULL is not decided; it is refused unless the slot is `nullable` |
| `local` | It may point into a local or a parameter of the function that holds it | A result that it reaches is refused: the variable ends where the function returns |
| `conflict` | No single type fits, with the reason | The program is refused |

Every assignment, argument and `return` is a flow from a source into a slot,
and the facts spread along the flows until they stop changing: `written` and
`owned` go back to where the pointer came from, `many` and `bytes` go both
ways, and `nullable` and `argument` go forward unless a test has ruled NULL out. A use of a pointer
converts between the types of two slots (`lower/pointer.rs`, `adapt`): a move
between owners, `&mut *b` or `as_deref_mut()` from an owner to a borrow, and
so on. What the borrow checker then rejects is refused by `rustc`, which is
the intended division of work: the analysis picks types, the compiler proves
them. Where a rule can see that the compiler will reject its translation, it
refuses with the reason instead: a result that points to a local, and two
arguments of one call that point into the same memory when the function
stores through one of them.

A test that the facts decide, such as `if (!p)` after a `malloc` that Rust
cannot fail, is dropped with its dead branch, and the analysis runs again
without it.

A local pointer that is declared without a value and never assigned counts
as NULL: reading it is undefined in C, and `None` is a value that Rust can
write for it.

### Cursors

A pointer that walks an array is not a borrow that moves. It is an index
beside the slice it walks, as a Rust programmer writes it: the slice stays
where it is, under its own name, and the pointer variable becomes a `usize`.

| C, with `p` a cursor over `a` | Rust |
| --- | --- |
| `T *p = a;`, `T *end = a + n;` | `let mut p: usize = 0;`, `let end: usize = n as usize;` |
| `*p`, `p[i]`, `p->field` | `a[p]`, `a[p + i]`, `a[p].field` |
| `p++`, `p--`, `p += n`, `p = q + 1` | `p += 1`, `p -= 1`, `p += n`, `p = q + 1` |
| `p < end`, `p == q`, `p != a` | `p < end`, `p == q`, `p != 0` |
| `p - q` | `p as i64 - q as i64` |
| `f(p)`, `return p;`, `q = p` for a `q` that is no cursor | `f(&a[p..])` or `f(&a[p])`, as the slot it goes to holds an array or one element |
| a parameter `char *s` that the function itself moves | `s` stays the slice, and a local `s_at` is the index |
| `if (p)`, `p == NULL` | the same test of `a`: `a + n` is NULL only where `a` is |

The **base** of a cursor is the expression for the array it walks, and it
must mean the same memory wherever the cursor is used. So it is one of: a
pointer variable that is never assigned after its declaration; an array;
a field of a record that such a variable or pointer holds; an element of
one of these at a constant index. A parameter that the function moves is its
own base.

`normalise::cursors` finds the cursors of a function and rewrites it, from C
to C, before the pointers are analysed:

1. The variables that need an index are the local pointers and parameters
   that the function moves (`++`, `--`, `+=`, `-=`, `p = p + n`) and the
   locals that it compares with another pointer or subtracts from one. Every
   value such a variable gets must be an offset from a base or from another
   such variable, which joins the two into one group.
2. A group has exactly one base. Its variables get the type `Type::Index`,
   the one type of the tree that C does not have, and every use of them is
   rewritten as the table says. A use that needs the pointer itself becomes
   `&a[p]`, which the rules for borrows translate as before.
3. A group without one base, with a base that may change, with a variable
   whose address is taken or with a value from anywhere else (NULL, `malloc`,
   a library function's result) is left as it is, and the program is refused
   with the construct that has no rule.

An index is a `usize`, and its arithmetic is Rust's: stepping before the
array is undefined in C and stops the program in Rust, and the end of the
array is a valid index for both. An offset that is signed and not a constant
is added with `wrapping_add_signed`, which gives the same index whenever
C's result is inside the array. Two pointers into different arrays have no
order in C, and their comparison stays refused.

The same holds for the end of a number that `strtol` and its like store
through their second argument. `normalise` gives each such call a variable
of its own for the end, an index into the text that was read, and the call
becomes a statement of its own:

| C | Rust |
| --- | --- |
| `char *end; long n = strtol(text, &end, 10);` | `let (n, end) = parse_int_prefix(text);` |
| `end == text`, `*end == '\0'`, `*end` | `end == 0`, `end == text.len()`, the byte of `text` at `end`, or 0 at its end |
| `puts(end)` | `println!("{}", &text[end..])` |
| `m = strtol(end, &end, 10);` | `let (m, read) = parse_int_prefix(&text[end..]);`<br>`let end = end + read;` |

Nothing is stored through `&mut end`, and no address is compared, so an
empty argument needs no memory of its own. `ends` does this before the first
analysis, and writes the call as `Stmt::Parse`; `cursors` then treats the new
variable as a cursor whose one value is the text that was read. Text has no
elements in Rust, so a base that only such ends index stays text: its
offsets stay `text + index` in the tree, which the rules for text read, and
a `&str` is cut only at the end of a number, which is never inside a
character. In an array of bytes the end is an index like any other cursor.

A read of `end` means the end of one call only where that call is the last
before it on every path: in the statements that follow the call in its
block, and in the branches of an `if` there, up to the next call. An end
pointer is refused where it is read anywhere else, where the call is inside
a larger expression than a declaration or an assignment, and where it is
assigned in any other way.

## Not yet built

The direction for each, so that the first rule for it starts in the right
place:

- **Text.** A `char *` that only holds string literals and is read as a
  whole is `&str` (R13). A `char` array stays bytes, as does every pointer
  into one; the library reads its text up to the NUL through `text_of` and
  `until_nul` (R15), and a string literal that such a pointer meets is
  `b"…\0"`. `argv[i]` is text too, `&args[i]`, and has no rule yet where
  it meets bytes. Still to build: text that a record or a global holds
  (`&'static str` when only literals reach it), `strcmp`, `strcpy` and the
  other functions of `string.h`, a width in `%s`, and above all a `String`
  for a buffer that the program only fills and reads as a whole, which
  needs a fact that says so.
- **Input.** `scanf` and `fgets` read through helpers over
  `std::io::stdin().lock()` that take a byte at a time, so that every way
  of reading shares what is left, as in C (`lower/input.rs`, and
  `read_line` in `library.rs`). `getchar`, `fread`, `%s` and `%f` belong
  there too. The result of `scanf` has a rule only where it is compared
  with the number of conversions: it is also -1 at the end of the input.
  `fprintf(stderr, …)` is `eprint!`.
- **Pointers to pointers and output parameters.** `T **out` that is only
  stored through is `&mut Option<Box<T>>` or a second result. The end
  pointer of `strtol` is the one case built (R21), as a second result of
  the call (see Cursors above). `strtoul` and `strtod` wait for the same
  rule, as do an end in `argv[i]` for an `i` that changes, which needs the
  argument in a variable of its own, and `errno`, which should become a
  result of the helper rather than a global.
- **Cursors that may be NULL.** A cursor is an index (see Cursors above). One
  that NULL reaches, or that a library function returns (`strchr`,
  `memchr`, `strstr`), should be an `Option<usize>` from `position`; a
  cursor over a program argument needs the bytes of the argument; and a
  cursor that gets the end of a number (`p = end` in a loop over a line)
  needs the end pointer in the cursor's group.
- **Walking a list.** `while (p) { …; p = p->next; }` needs a narrowing for
  loops like the one for `if`; today it uses `is_some()` and `unwrap()`.
- **Shared memory.** Two slots that own the same memory do not compile: the
  second use is a use after a move. `Rc` belongs only on a slot that a fact
  proves shared, and `RefCell` only where it is also written; neither is a
  default.
- **Others.** `goto`, a `case` that falls through, pointers to functions
  (`fn` types), unions (an `enum` when a tag says which member is live),
  `+=` and `++` on a bit-field, variadic functions, an array whose length is an expression
  (Clang prints it only as text), a local that gets its first value in a branch or
  in a loop (`let x;`).

What a review of the passing translations found that no rule writes as Rust
would, most frequent first. Each needs the proof named with it, and keeps
C's promotions, checked arithmetic and the input it consumes:

- **Types that inference gives.** `let foo: FooT = FooT { … }` in every
  program. An annotation goes where the value alone fixes the type: a
  record, a call, a cast, a literal with a suffix.
- **A value from an expression instead of a variable set later.** `let mut
  x = 0;` and then a store in a branch or after a read, where `let x =
  if …` or `scan_int().map_or(0, …)` says it. Needs every store and read
  of the variable, also through pointers.
- **Stores that nothing reads.** A first value that the next statement
  overwrites, and a result that is never read. Needs liveness; the
  expression stays where it can stop the program or read input.
- **Promotions with no effect.** `args.len() as i32 != 2`, `foo.x as i32 as
  u32` for a `u32`, `!!(b != 0)`. Only where the widths make the casts an
  identity: a `char`'s sign extension and a narrowing stay.
- **A step inside an index.** `*dst++ = *src++` is `dst[{ let before =
  dst_at; dst_at += 1; before }] = …`; the steps belong after the
  statement, `dst[dst_at] = src[src_at]; dst_at += 1; src_at += 1;`, where
  nothing else in the statement reads the cursors.
- **Statements of C in conditions and at the end.** `if !({ x -= e; x } >
  0)` for a store in a condition, `return;` as a function's last
  statement, `x = x * y` for `x *= y`. The compound form needs a place
  without effects.
- **The spelling of numbers.** `0xAAAA` printed as `43690`, `INT_MIN` as
  `-2147483647 - 1` (`i32::MIN`), an `f32` constant with all the digits of
  its `double`. Needs the literal's text from Clang.
- **A counter where the elements would do.** `while i < n { …a[i as
  usize]…; i += 1; }` as `for value in &a`, when the counter is dead after
  the loop and only indexes. Two slices walked together need lengths that
  are proved equal.
- **A length beside its slice.** `(a: &[i32], len: i32)` where every
  caller could pass `&a[..len]`. Needs all callers, and a `len` with no
  other use.
- **A container for one global.** `Globals { sum }` for a program with one
  piece of state, which could be a local of `main` passed as `&mut i32`.
- **Numbers in a `char`.** A `char` that only holds numbers is `u8` with
  `as i8 as i32` at each use; `i8` would say it. Needs the `bytes` fact's
  counterpart for values, since buffers stay `u8`.
- **Bytes read as numbers through text.** `parse_int(&text_of(&buffer))`
  makes a `String` of bytes only to read digits; the parse helpers should
  take `until_nul(&buffer)`.
- **The end of a number that is only tested.** Where `end` is only
  compared with the text (R21), `let Some(n) = … else { … }` on a helper
  that returns nothing when it read no digit, instead of an index that is
  compared with 0.
- **Operations the standard library has.** `x.min(120)` for a conditional,
  `reverse_bits`, `div_euclid`. Only by rules for exact shapes: a
  near-miss such as `d ^ (d >> 31)` is not `abs`.
- **The helpers.** One `StdinLock` held by `main` and passed on, rather
  than a lock for every byte; `parse_int_prefix` scans its text twice.

## Known weak spots

Places where a rule assumes more than the analysis proves. Each should
become a fact and a refusal:

- An owner moved out of a field or a global leaves `None` behind
  (`take()`); a C program that reads the field again before storing to it
  would see NULL in Rust.
- A pointer into the middle of an array that is no cursor is the rest of
  the array from there (`&a[1..]`), so an index or a step back before it
  (`p[-1]`) stops the program where C reads the element.
- A cursor that steps one before its array, as the last `p--` of
  `for (p = end - 1; p >= a; p--)` does, stops the program there. C leaves
  that undefined, the sanitizers do not report it, and compiled C runs on.
- `%c`, `%s` of a program argument and `argv` assume ASCII; C passes any
  bytes through. `text_of` prints bytes that are not UTF-8 as U+FFFD.
- A conversion from a floating-point number to an integer saturates in Rust
  where C's behaviour is undefined, and a NaN prints differently.
- `parse_float` reads decimal numbers only: `strtod` also reads
  hexadecimal ones (`0x1p3`), of which this reads the `0`.
- `read_line` takes an error of the read for the end of the input: after
  some bytes it returns them, where `fgets` returns NULL.
- `&args[i]` stops the program where `i` is `argc`, and C reads NULL
  there: a program that only stores `argv[argc]` runs on in C.
- `%08.2f` of an infinity is padded with zeros in Rust and with spaces in
  C.
- A pointer that a function returns into what a parameter may store
  through keeps the argument borrowed for as long as the result is used.
  A C program that reads the argument meanwhile, as
  `e = find(entries, …); printf("%d %d", entries[0].value, e->value);`
  does, is translated and does not compile.

## Tests

`tests/translate/` holds one test per subject. `check` compiles the C program
with Clang and the translation with `rustc` (`-F unsafe_code -D warnings`,
overflow checks on), runs both on each input and compares output and exit
code; the test then asserts the lines that show the rule's shape, so a
translation that passes but reads worse fails the test. `tests/dev.rs` checks
that every development program is translated or refused, never a panic.
