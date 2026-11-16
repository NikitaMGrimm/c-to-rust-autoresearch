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
Clang's JSON -> clang -> c::Program -> unions, voids, protos, places, join, ends, found, pointers -> facts -> same -> facts -> cursors -> facts -> alias -> facts -> follow -> facts -> narrow -> facts -> lower -> rust -> text
```

| Stage | File | Does | Knows |
| --- | --- | --- | --- |
| Front end | `src/clang.rs` | Reads Clang's JSON into the C tree | Clang's JSON; nothing about Rust |
| C tree | `src/c.rs` | Types, expressions and statements with their C types, and walks over them | C |
| Analysis | `src/facts.rs` | Facts about each pointer, variable, global and function | The whole program |
| Rewrites | `src/normalise.rs` | Rewrites C to C so that one statement means one thing: `unions` makes a union that holds the bits of a number an integer, `voids` gives a `void` pointer the one type it holds, `protos` gives a pointer to a function without a prototype the parameters of the functions it holds, `places` replaces a global pointer, or a pointer field of global records, that never changes by the place it names, `join` declares a local where it gets its first value, `ends` makes the end that `strtol` stores a result of the call, `found` makes a local that only holds what a search found and is only tested an `Option<usize>`, `pointers` makes a local pointer to a pointer that only names one pointer variable that variable, `same` gives a call that hands one pointer in several arguments a copy of the function with one parameter for them (not for a `T **` parameter, which lends a place), `cursors` makes a pointer that walks an array an index, `alias` makes a local pointer that only names one place, and that Rust would reject as a borrow, that place, `follow` names the local that got an owner's memory where C reads the owner again, `narrow` names a pointer that a test found not to be NULL | The facts, for `same`, `cursors`, `alias`, `follow` and `narrow` |
| Rules | `src/lower/` | Translates each construct into a `syn` tree | The facts, one construct at a time |
| Names | `src/names.rs` | The Rust name of each C name: snake case for functions, variables and fields, camel case for records, a record without a tag after the variable or field that has it, upper case for constants and enumerators. Two C names in one Rust scope never share a name | Rust's naming and keywords |
| Output | `src/rust.rs` | Builds `syn` expressions and prints the file with `prettyplease` | Rust's syntax |

The rules are split by subject: `lower/mod.rs` (the program, records, globals,
functions, `main`), `types.rs`, `expr.rs`, `stmt.rs`, `pointer.rs`,
`function.rs` (pointers to functions), `library.rs` (the C library) and
`input.rs` (standard input).

## Where a change goes

- Clang prints a construct that the front end does not read: `clang.rs`, and
  `c.rs` if the tree has no node for it. The tree keeps what C means, not how
  Clang prints it. A documentation comment, which Clang hangs on its
  declaration, is taken out of the tree before it is read (R57). Types,
  tags and typedefs are found by name, so a program
  that gives one name two meanings in different scopes is refused. A record
  or an enumeration without a tag is found by the place of its declaration,
  which Clang prints in its type, `struct (unnamed at FILE:4:2)`.
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
| `switch` | `match`; cases that fall through share one arm, `entry @ (2 \| 1) => { if entry == 2 { … } … }` |
| `goto out;` with `out:` later in the same block or one around it | `'out: { …; break 'out; … }` |
| `goto` back to a label of a block around it, or past another label | `let mut entry = 0; 'entry: loop { if entry == 0 { … } if entry <= 1 { … } …; break; }`, a `goto` is `entry = 1; continue 'entry;`, and a `break` or `continue` of a loop around names it, `'outer` |
| `goto` into a block from outside it | The label moves to the front of the statement that holds it: `L: if (entered \|\| c) { if (!entered) { A; } entered = 0; B; }`, and the jump is `entered = 1; goto L;`, which R48 or R49 translate |
| `printf` with a literal format | `print!` or `println!`, the format converted at translation time |
| `const char *` that only holds string literals | `&str` |
| `if (p)`, `if (!p) return;` | `if let Some(p) = …`, `let Some(p) = … else { … }` |
| `malloc`, `calloc`, `realloc`, `free` | `Box::new`, `vec![zero; n]`, `resize`, `drop` |
| `scanf("%d", &x)` | `if let Some(scanned) = scan_int() { x = scanned as i32; }` |
| `fgets(line, sizeof line, stdin)` | `read_line(&mut line)` |
| `fgets(line, size, stdin)` for a `char *line` | `read_line_within(line, size)` |
| `n = fread(bytes, 1, sizeof bytes, stdin)` | `n = read_items(&mut bytes, 1)` |
| `%s` of a `char` array | `text_of(&line)` |

### Pointers

A *slot* is a place that holds a pointer: a variable, a field, a function's
result, or the place behind a pointer to a pointer (see Pointers to
pointers). The analysis gives each slot nine facts, and they decide its type:

| Fact | Set when | Effect |
| --- | --- | --- |
| `owned` | It holds memory from `malloc`, is freed, or lives in a record, a global or a result that got it from an owner | `Box` or `Vec` rather than a borrow |
| `many` | It is indexed, moved, or holds an array | `Vec` or a slice rather than one element |
| `written` | Something is stored through it, here or wherever it is handed on | `&mut` rather than `&` |
| `nullable` | NULL may reach it | Wrapped in `Option` |
| `bytes` | It points to `char` that is read or written one by one, or to another type | A `char` pointer without it is `&str` |
| `argument` | `argv[i]` may reach it as the program found it, which is NULL at `argv[argc]` | A test of it for NULL is not decided; it is refused unless the slot is `nullable` |
| `environment` | It holds what `getenv` returned, or gets that from an owner | An owned `String` for text |
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
refuses with the reason instead:

- a result that points to a local;
- two arguments of one call that point into the same memory when the
  function stores through one of them. Two elements of one array of which
  one is before the other on every run are no such case: the array is
  split between them (R54). Where the two arguments are the same pointer,
  `normalise::same` first gives the call a copy of the function in which
  their parameters are one, which they are while the function runs;
- a pointer into a global that changes, handed to a function that takes
  `&mut Globals`, or that takes `&Globals` and stores through the pointer;
- a local pointer to a variable of its function that is used after the
  variable itself was stored to since the pointer got its address, or
  read where the pointer is stored through (`Facts::in_use`). "After" is
  the order of the statements, with each branch of an `if` from the state
  before it and the body of a loop following itself; a pointer that gets
  the address from another pointer, or through a narrowing, is not
  followed. A copy of a local pointer that owns or is stored through
  borrows that pointer in the same way: `q = p; p->n += 1; p = p->next;`
  with a later read of `q` is refused. Where such a pointer gets one value
  only, in its declaration or a statement of its block before anything
  reads it, and that value is the address of a place that stays the same
  place (a variable that is no pointer, a field of one, an element at a
  constant index, or such an array as a pointer), `normalise::alias` first
  names the place at each use of the pointer and leaves the pointer out
  (R61). It leaves a pointer whose address is taken, and one used where a
  variable with the name of the place hides it, since the Rust names
  follow the C names. A local `T **pp` whose one value is `&p` for a
  pointer variable `p`, a local or a parameter, and that is used only as
  `*pp` and `pp[0]`, is `p` at every use wherever it is
  (`normalise::pointers`, R61 too), before the first analysis;
- an owner, a local or a global, that is read after its memory went to
  another owner, a field, a global, a local or a parameter, and before
  anything stored to it (`Facts::moved`), in the same order. A `free` is
  not followed, nor a read in another function.

An owner whose memory went to another is one of three things, and each
has its own translation:

- **Two names for one pointer.** `q = p;` for a local owner `q`: until `p`
  or `q` is stored to, C reads the same pointer by both names, and Rust
  has moved it to `q`. `normalise::follow` names `q` at each read of `p`
  in the statements that follow in the block, up to the first that
  stores to either; of that statement, a plain store to `p`, the value
  names `q` too, so that `gone = list; list = list->rest; free(gone);`
  is `let mut gone = list; list = gone.rest.take(); drop(gone);` (R66). A
  borrow is not what C means here: `q` is freed or handed on. The rewrite
  keeps every value, so it needs no proof about other pointers; it stops
  at a `case`, leaves a function with a label, and for a global `p` stops
  at a call of a function of the program, which may store to it.
- **A record that is linked before it is filled.** `list->rest = fresh;
  fresh->weight = 3;` and `shelf = fresh; fresh->size = 1;`: the memory
  went into a field or a global, and the local is used on. A Rust
  programmer fills the record first and links it last, so the store
  belongs after the local's last use: the order of the statements
  changes. That needs a proof that no statement between reads the field
  or the global, also through another pointer to the same record, which
  the analysis cannot give yet. Refused.
- **Memory with two owners.** A list with a pointer back (`fresh->before
  = top; top->after = fresh;`), a ring (`only->next = only;`) and a graph
  whose records point to records that a pool owns: both pointers live on,
  and neither is the one owner. The honest translation is shared
  ownership, `Rc<RefCell<T>>` with `Weak` for the pointer back, or one
  owner (a `Vec` of the records) with indices for the others. Neither is
  a default: it needs a fact that proves which slots share, and belongs
  to **Shared memory** below. Refused.

A function that returns an owner out of a record which it only borrows,
as `struct node *after(struct node *p) { return p->next; }`, is refused
too: the translation would take the owner out of the record, where C's
record still points to it. A function that nothing names is translated:
what it would do shows nowhere.

A test that the facts decide, such as `if (!p)` after a `malloc` that Rust
cannot fail, is dropped with its dead branch, and the analysis runs again
without it.

A global pointer, or a `static` one of a function, that nothing assigns and
whose address is never taken, and whose initialiser is the address of a
global, of a field of one or of an element at a constant index, or an
array of a global, is that place: `normalise::places` replaces each use by
the initialiser and drops the pointer, so that `*p` reads `x` and `p->a`
reads `s.a` (R62). The front end makes the address of a record's compound
literal that initialises such a pointer the address of a global of its
own, which has the pointer's name. Other global pointers are refused: one
that changes or whose address is taken, because a global pointer is an
owner and the address of a variable is none, and one that is NULL or has
no initialiser.

A pointer field of a record is such a place too when every record of the
type is a global or `static` with an initialiser that gives the field
such an address, and nothing stores to the field, takes its address,
stores a whole record, converts a pointer to one, or takes the record's
size: `places` replaces each read `v.p`, and `(&v)->p` that R62 left, by
`v`'s place, and takes the field out of the record, its initialisers and
the numbers of the fields after it (R64). Records of the type that are
locals, parameters, results, elements, fields or reached through a
pointer keep the field, and are refused as before.

A local pointer that is declared without a value and never assigned counts
as NULL: reading it is undefined in C, and `None` is a value that Rust can
write for it.

### Numbers in bytes

A pointer to bytes cast to a pointer to a number, `*(T *)p`, reads or
stores a number that C keeps in the bytes (R69). The bytes stay a slice of
`u8` and the number is made from them; a slice too short stops the Rust
program where C reads beyond the array. The host is little-endian, but the
translation says what C does, the bytes in the machine's order.

| Shape | Rust |
| --- | --- |
| The number: `short`, `int`, `long`, `long long`, signed or unsigned, `int16_t` to `uint64_t`, `float`, `double` | `i16` to `u64`, `f32`, `f64` |
| The number: `long double` | refused, as the type is |
| The bytes: `char *`, `unsigned char *`, `uint8_t *`, an array, a parameter, a local, a field, memory from `malloc`, `const` or not | the slice the pointer reads as elsewhere; the analysis marks it an array |
| The bytes: `signed char *`, `int8_t *` | refused: `i8` elements |
| A read `*(T *)p`, `((T *)p)[0]` | `i32::from_ne_bytes(p[..4].try_into().unwrap())` |
| A read `((T *)p)[i]` | `i32::from_ne_bytes(p[i as usize * 4..][..4].try_into().unwrap())`, `p[8..][..4]` for a constant `i` of 2 |
| `((T *)p)[i]` for a negative constant `i` | refused |
| A store `*(T *)p = v` | `p[..4].copy_from_slice(&i32::to_ne_bytes(v))` |
| `*(T *)p += v`, `++*(T *)p`, `&*(T *)p`, `&((T *)p)[i]`, a store used as a value | refused |
| `(T *)(p + k)`, `(T *)&a[k]` | the slice from `k`, `p[k..][..4]` |
| A local `T *q = (T *)p` that nothing stores to again or takes the address of, only used as `*q` and `q[i]`, whose cast names only locals and parameters that do not change in the function, their arrays, constants, and sums and elements of those | the cast at each use (`normalise::casts`), before the first analysis |
| A cast pointer that is kept: stored, returned, handed to a function, compared, advanced; such a local whose cast reads a variable that changes, a field, an element as a value or a call | refused |
| The bytes of a number, `(unsigned char *)&n` read as `b[i]` | refused |
| A cast between pointers to numbers of different sizes, `(int *)longs` | refused |
| A cast to or from a pointer to a record | refused |

### Pointers to pointers

`T **pp` is a pointer to a place that itself holds a pointer. The place is
a slot like any other: the variable, the field or the element whose address
`pp` holds, or the elements of the array that `pp` owns. So `pp` has two
levels of facts. Its own say how it holds the place (`&`, `&mut`, a slice,
a `Vec`), and those of the slot **behind** it (`Slot::Behind`) say what the
place holds (`Option<Box<T>>`, `Vec<T>`, `&str`). An array of pointers,
`T *a[N]`, has the same slot behind it for its elements. The Rust type is
the two put together, and no shape gets a type that Rust would not use for
it:

| C | Rust | Why a Rust programmer writes it so |
| --- | --- | --- |
| `void push(struct node **head, int v)`, called as `push(&list, 1)` | `fn push(head: &mut Option<Box<Node>>, v: i32)`, `push(&mut list, 1)` | the function replaces the caller's owner, so it borrows the owner itself |
| the same parameter where nothing is stored through it or behind it | `head: &Option<Box<Node>>` | a shared borrow of the owner; `Option<&Node>` would say it better and waits for the rewrite below |
| `int **rows = malloc(n * sizeof(int *)); rows[i] = malloc(m * sizeof(int));` | `Vec<Vec<i32>>` | an owned table of owned rows |
| `f(int **rows, int n)` for it | `rows: &[Vec<i32>]`, `&mut [Vec<i32>]` | a slice of the rows |
| `struct node *buckets[8];`, `struct node **b = calloc(8, sizeof *b);` | `[Option<Box<Node>>; 8]`, `Vec<Option<Box<Node>>>` | each element is a list that may be empty |
| `char *names[] = { "a", "b" };`, `f(char **names, int n)` | `[&str; 2]`, `names: &[&str]` | text that nothing stores to is `&str`, and an array of it a slice |
| `int *at[2] = { &x, &y };` | `[&i32; 2]`, `[&mut i32; 2]` | borrows in an array, as far as the borrow checker lets them live |
| `int **pp = &p;` for a local that only ever names `p`, a local or a parameter, and is only used as `*pp` and `pp[0]` | no variable: `*pp` is `p` and `**pp` is `*p` (R61) | a second name for one variable is that variable |
| `argv` handed to a function, `run(char **argv)` | `argv: &[String]` | the arguments are the program's `Vec<String>` |

The facts of the slot behind a pointer come from flows as for any slot,
with one difference: a pointer to a place and the place are one memory, so
their facts are equal, not ordered. `pp = &p` joins the slot behind `pp`
with `p`'s slot, `pp = qq` and an argument join the two slots behind, `pp =
&s->next` joins with the field, and `pp = &a[i]`, `pp = a` join with the
elements of `a`. `*pp = e` and `pp[i] = e` are flows into the slot behind,
and `*pp`, `pp[i]` as values are flows out of it. What `malloc` and
`calloc` give `pp` is memory of the heap, so the slot behind owns what it
holds, as a field does; the zero that `calloc` leaves in an element is
NULL.

`*pp` and `pp[i]` are places that hold a pointer, as `s->next` is, and the
rules for a pointer in a field apply to them: a test for NULL, the
narrowing of R10, `(*pp)->field`, a store, a move out of the place, which
leaves `None` behind as it does in a field (`take()`), and `free(*pp)`.

| Shape | Rust, or the refusal |
| --- | --- |
| a parameter `T **out`; the argument is `&p` for a local pointer `p`, `&s.field`, `&s->field`, `&a[i]` of an array of pointers, or the caller's own `T **` | `&mut` or `&` of what the place holds, as in the table above |
| `*out = e;`, `*out` as a value, `(*out)->f`, `if (*out)`, `*out == NULL`, `free(*out)` | the rules for a pointer in a field, on `*out` |
| a function that stores through `out` on every path before it reads it, with callers that pass the address of a local | a second result would say it; built as `&mut` until every caller can be rewritten |
| the argument is the address of a global pointer, or of a place reached through one | refused: the callee would hold a borrow of `Globals` beside `globals` |
| two arguments of one call that name the same place, or a place and the pointer in it, `f(p, &p)` | refused: Rust takes `p` before it lends it |
| a local `T **pp` that only ever names one pointer variable, a local or a parameter, and is only used as `*pp` and `pp[0]` | that variable, by R61, before the analysis; `p` may change, and `*pp` reads its value of the moment as C does. `pp` stored to twice, moved, indexed at another place, compared, handed on, returned, its address taken, cast, `&s.f`, `&a[i]`, `&global`, `void **`, `T ***`, and a use where another `p` hides it stay refused |
| a local `T **link` that moves along a list, `link = &(*link)->next;`, and is stored through at the end | `let mut link = &mut head; while link.is_some() { link = &mut link.as_mut().unwrap().next; } *link = Some(node);`: a borrow that moves down the owners. Refused until the slot behind a local that is assigned twice has a rule |
| an array of pointers, a local, a global or a field: `T *a[N]`, with or without an initialiser | `[P; N]` for the type `P` of the slot behind; an element without a value is `None`, and an array of owners is built element by element since an owner is not `Copy` |
| `T **m` from `malloc(n * sizeof(T *))` or `calloc(n, sizeof(T *))` | `Vec<P>` of `n` elements, `(0..n).map(|_| zero).collect()`; after `calloc` an element is NULL, so `P` is an `Option` |
| `m[i] = malloc(…)`, `m[i][j]`, `free(m[i])`, `free(m)` | a store to the element, `m[i][j]`, `drop(take(&mut m[i]))`, `drop(m)` |
| `t = a[i]; a[i] = a[j]; a[j] = t;` for owners | refused: the move out of `a[j]` is not stored to again by the next statement; `a.swap(i, j)` is the rule to build for the exact shape |
| `t = a[i]; a[i] = e;`, `n->next = a[i]; a[i] = n;`, `a[i] = f(a[i], v);` | `a[i].take()`, which leaves `None` until the store; refused where anything may read the element between the two |
| a cursor over an array of pointers, `for (char **p = names; p < names + n; p++)` | an index, as for any cursor; `*p` is `names[p]` |
| `char *names[]` of string literals; an element stored to with another literal | `[&str; N]`; a store of a literal |
| `argv` handed on, `argv + 1`, `char **p = argv;` and `*p++` | `&args[1..]`, a cursor over `args`; `argv[i][j]` stays refused (a byte of an argument) |
| what a pointer behind a pointer points to is itself moved: `const char **s` with `*s += n`, `(*s)++` | refused: `a cursor behind a pointer`; it should be `&mut usize` beside the text |
| a result of type `T **`, a `T **` in a record or a global, or kept in a `static` | refused |
| `T ***`, a pointer to a pointer to a function, `void **`, a cast between `T **` and another pointer type | refused |
| `sizeof(T *)`, `sizeof *m` in an allocation's size | counts elements, as `sizeof(T)` does for `T *`; anywhere else it is C's 8 |
| `pp == qq`, `pp - qq` outside a cursor | refused: a comparison of two addresses |

A shape that has no rule yet is refused as `a pointer to a pointer`, or as
`a pointer in an array` for an array's elements. The analysis joins the
slot behind `pp` with the place an argument or a store names: the two
take the union of their facts each round, so that neither is ordered
after the other. A place behind that holds a borrow (`const char **out`
that gets a literal, `&mut &str`) needs two lifetimes and is refused as
`a borrow behind a pointer`; an argument that reads a variable of
another argument of the call is refused, also where both are shared
borrows. Built so far (R65): a parameter `T **` for a record, a number or
`char`, with the arguments `&p`, `&s.field`, `&s->field` and the
caller's own `T **`, and inside it the rules for a pointer in a field on
`*pp`, where a move out of `*pp` is refused unless nothing can read the
place before the next statement stores to it again; a local `T **pp`
that only names one pointer variable (R61); and the end of a number
(`char **end` of `strtol`, see Cursors). R67 builds the arrays: a local
`T **m` that owns the memory of
`malloc` or `calloc`, as `Vec<P>`; an array of pointers, a local, a
global or a field, as `[P; N]`; a parameter `T **m` that gets such an
array, as `&[P]` or `&mut [P]`, also `&[&str]` for text that nothing
stores to; and `&a[i]` for a parameter of R65. The slot behind them
holds owners or text only: an array of borrows is refused as `a borrow
in an array of pointers`. A move out of an element is built only where
it is stored to again before anything reads it: by the next statement,
whose value does not name the array and calls no function (`t = a[i];
a[i] = t->next;`, `n->next = a[i]; a[i] = n;`), or by the statement
itself, `a[i] = f(a[i], v);`, for a local array that only its elements
are taken from. So an exchange of two elements stays refused, and
`a.swap(i, j)` is not built. A cursor over an array of text works as any
cursor.
The rest of the table is refused.

### Cursors

A pointer that walks an array is not a borrow that moves. It is an index
beside the slice it walks, as a Rust programmer writes it: the slice stays
where it is, under its own name, and the pointer variable becomes a `usize`.

| C, with `p` a cursor over `a` | Rust |
| --- | --- |
| `T *p = a;`, `T *end = a + n;` | `let mut p: usize = 0;`, `let end = n as usize;` |
| `*p`, `p[i]`, `p->field` | `a[p]`, `a[p + i]`, `a[p].field` |
| `p++`, `p--`, `p += n`, `p = q + 1` | `p += 1`, `p -= 1`, `p += n`, `p = q + 1` |
| `p < end`, `p == q`, `p != a` | `p < end`, `p == q`, `p != 0` |
| `p - q` | `p as i64 - q as i64` |
| `f(p)`, `return p;`, `q = p` for a `q` that is no cursor | `f(&a[p..])`: C may hand on the end of the array, where Rust has an empty rest and no element |
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
   a type of the tree that C does not have, and every use of them is
   rewritten as the table says. A use that needs the pointer itself becomes
   `&a[p]`, which the analysis takes for a pointer into an array, and so
   `&a[p..]`.
3. A group without one base, with a base that may change, with a variable
   whose address is taken or with a value from anywhere else (NULL, `malloc`,
   a library function's result) is left as it is, and the program is refused
   with the construct that has no rule.

An index is a `usize`, and its arithmetic is Rust's: stepping before the
array is undefined in C and stops the program in Rust, and the end of the
array is a valid index for both. Compiled C runs on after such a step, so
a function is refused where its statements show one
(`normalise::steps_before`): the index is known to be 0 where it is
stepped back, followed through the statements that run one after the
other, or the cursor is tested for being before the start of its array,
as in `for (p = end - 1; p >= a; p--)`, which only such a step decides. An offset that is signed and not a constant
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

### Unions

C has no way to ask for the bits of a `float`, so programs store the number
in one member of a union and read another: `u.flt = x; n = u.num;`. Rust has
`to_bits` and `from_bits`, and safe Rust has no memory that two types share.
So a union whose members are all numbers of one size is not a type of its
own. Its value is the unsigned integer of that size, the bits, and each
member converts on its way in and out (R33):

| C | Rust |
| --- | --- |
| `union { float flt; uint32_t num; } u;` | `u: u32` |
| `u.flt = x;` as the first use of `u` | `let u = x.to_bits();` |
| `u.flt = x;`, `u.num = n;`, `u.i = k;` | `u = x.to_bits();`, `u = n;`, `u = k as u32;` |
| `u.flt`, `u.num`, `u.i` | `f32::from_bits(u)`, `u`, `u as i32` |
| `u.num ^= 1;`, `p->flt` | `u ^= 1;`, `f32::from_bits(*p)` |

`normalise::unions` does this to the tree before any other stage: the
union's type becomes the integer wherever it is written, a read of a member
becomes a conversion of the bits (`c::ExprKind::Bits` between a
floating-point number and its bits, a cast between integers) and a store
becomes a store of the converted value. So the analysis and the rules never
see the union, and `join` declares it where its first member is stored, as
it does for any local. The member that has the integer's own type is the
integer itself and may be updated in place or have its address taken. The
other members are values, not places: `u.flt += 1`, `&u.flt` and a store
whose value is used have no rule yet, nor has a union's initialiser. A
union with members of different sizes, with an array, a record or a pointer
is refused: of those, bytes beside a number would be `to_ne_bytes`, and a
union that is one of several kinds of value would be an `enum`.

### What the library reports

The C library reports through pointers and through `errno` what Rust
returns as a value: a place that was found or NULL, a variable of the
environment or NULL, the end of a number, an error beside a result, the
number of conversions or the end of the input. Each becomes a result.

**A place that a search found.** `strchr`, `strrchr`, `memchr`, `strstr` and
`strpbrk` return a pointer into their first argument, or NULL. The result is
a cursor on that argument (see Cursors) that may be absent, an
`Option<usize>` from `position`:

| C | Rust, or the refusal |
| --- | --- |
| `strchr(s, c)` of bytes | `find_byte(until_nul(&s), c as u8)`, a helper that finds the NUL at the end when `c` is 0, as C does |
| `strchr(s, 'x')` of text (`&str`, `String`) | `s.find('x')` for a constant that is ASCII and not NUL; other characters `find_byte(s.as_bytes(), c as u8)` |
| `strrchr` | `rfind_byte`, `s.rfind('x')` |
| `memchr(s, c, n)` | `s.iter().take(n as usize).position(\|&found\| found == c as u8)`: no byte beyond `n` or the array is read, and `n` of 0 finds nothing; refused for text, and for a `c` with a side effect |
| `strstr(s, t)` | `s.find(t)` for text, `find_bytes(&s, &t)` for bytes up to their NULs; an empty `t` is found at 0 |
| `strpbrk(s, set)` | `find_any(s.as_bytes(), set.as_bytes())`, a helper, for text and bytes |
| the result only tested: `if (strchr(s, c))`, `strchr(s, c) != NULL`, `== NULL`, `!strstr(s, t)`, also inside `&&`, `\|\|` and `?:` | `.is_some()`, `.is_none()`; `s.contains('x')` for text |
| kept in a local that is only tested for NULL | `let found: Option<usize> = …;` and `found.is_some()`: `normalise::found` gives the local `Type::Found` |
| kept in a local and used where the test holds: `if (p) { *p = 0; n = p - s; puts(p + 1); }`, and the `else` of `if (!p)` | `if let Some(p) = p { s[p] = 0; n = p as i64; println!("{}", text_of(&s[p + 1..])); }` (R47): an index beside `s`, which stays usable. `s` is an array, a pointer that nothing assigns again, or a local pointer that is not assigned between the search, the test that follows it at once and the uses |
| used outside such a branch, under `&&` or `\|\|`, stored to or moved there, or from searches of two arrays or of an offset | refused: `a place that a search found, used as a pointer: no rule yet` |
| the search goes on from the result in the condition of a loop: `for (s = in; (s = strchr(s, c)); s++)`, `while ((s = strstr(s + 1, t)))` | `while let Some(at) = find_byte(until_nul(&in_[s..]), c) { s += at; …; s += 1; }` (R52): `s` is an index beside `in` that starts at 0 |
| the search goes on from the result elsewhere: `p = strchr(p + 1, c)` as a statement, a pointer read after its loop | refused with the same reason. It should be the search of the rest, `find_byte(&s[p + 1..], c).map(\|at\| p + 1 + at)` |
| returned by a function of the program for its first parameter: `char *find(const char *b, …) { …; return memchr(b, c, n); }`, with the result kept in a local as above | the function returns `Option<usize>` and its call is a search like the library's (R51) |
| returned otherwise, handed on or stored in a record | refused with the same reason. It should be the rest of the array from there, `….map(\|at\| &b[at..])`, an `Option<&[u8]>` |
| the first argument is itself NULL, an argument of the program or a result of `getenv` | NULL is refused (undefined in C); the others are text |
| `c` beyond a `char` (`strchr(s, 300)`) | `c as u8`, as C converts it |
| a store through the result when the argument is `const` text | refused as any store to text |
| an argument with a side effect | evaluated once, in C's order |

**A variable of the environment.** `getenv(name)` is `std::env::var(name).ok()`,
an `Option<String>` that the program owns:

| C | Rust, or the refusal |
| --- | --- |
| `char *v = getenv("NAME");` then `if (v == NULL)`, `if (v)`, `v != NULL && …` | `let v: Option<String> = std::env::var("NAME").ok();` and the rules for a pointer that may be NULL |
| `getenv(name)` for a `name` that is text or bytes | `std::env::var(name)`, `std::env::var(text_of(&name))` |
| read as text: `atoi(v)`, `strtol(v, …)`, `printf("%s", v)`, `strlen(v)`, `strcmp(v, "x")`, `strchr(v, c)` | the rules for text, on `&v` |
| `getenv("NAME") != NULL` and `== NULL`, not kept | `std::env::var("NAME").is_ok()`, `.is_err()` |
| a byte of it: `*v`; `v[i]` | `*v` by the rule for text; `v[i]` is refused, as a byte of an argument of the program is |
| a store through it, `v[0] = 'x'`, or a move, `v++` | refused: the environment's text is not the program's to change, and a cursor over text needs its bytes |
| kept in a record or a global, or returned | refused: `kept in a record or a global: no rule yet`, and a result that borrows a local's `String` as one that points to a variable that ends with the function. It should be an owner of the `String` |
| a value that is not UTF-8, or a name with `=` or a NUL | `var` gives `None` or stops where C gives the bytes; a weak spot, listed below |
| `setenv`, `putenv`, `environ` | refused |

**`errno`.** A library function that fails stores a number in `errno` and
never clears it, so a program stores 0, calls, and compares. In Rust the
call returns the error beside its value:

| C | Rust, or the refusal |
| --- | --- |
| `errno = 0; n = strtol(text, &end, 10); if (errno == ERANGE)` | `let (n, end, range) = parse_int_checked(text);` and `if range`; `range` is whether the number was beyond its type, which is all that `strtol`, `strtoul`, `strtod` and their like report in base 10 (R37); `parse_uint_checked`, and `parse_float_checked::<f64>(text)`, for which `range` is also an inexact number below the least normal one of the type after rounding to its digits with no bound on the exponent, as glibc reports an underflow |
| `errno == ERANGE`, `errno != 0`, `errno`, `errno == 0`, `!errno` after such a call | `range`, `range`, `range`, `!range`, `!range` |
| `errno == EDOM`, `errno == EINVAL` after such a call | `false`: the call never reports it |
| `errno = 0; r = pow(x, y);` and the same tests, also for `sqrt`, `log`, `exp` and the other functions of `math.h` with a rule | `let r = x.powf(y);` and a test of the result as glibc makes it: `EDOM` where the result is NaN and no argument is, `ERANGE` where it is infinite and the arguments are finite, or zero for `pow` and `exp` where an exact result would not be (R38: `pow` and `sqrt`; `log` and `exp` have no rule yet); the result must be a `double` variable and the arguments variables or constants, which nothing stores to before the read |
| a call whose `errno` nothing reads | the call as before, without the third result |
| a read with no `errno = 0` before the call in the same block, or with another call between the store and the read that may set it (a function of the program, a library function that reports through it), or after a branch or a loop that holds such a call | refused: `a read of `errno` that is not one call's error` |
| `errno = 0` that no read follows | dropped |
| a store of another value, `errno = EINVAL`, `errno = saved` | refused |
| `errno` as a value: `int e = errno;`, `printf("%d", errno)`, `strerror(errno)`, `perror(…)`, `return errno;`, an argument | refused: the number is the platform's |
| `&errno`, and `errno` set in one function and read in another | refused |
| `errno` after `malloc`, `fopen`, `fgets`, `scanf` and other calls that report something else through it | refused |

The rewrite `normalise::errno`, at the end of `ends`, does this: the call
is the `Stmt::Parse` that `ends` makes, with a variable for the error, and
each read of `errno` that the call reaches becomes a test of that
variable. A call reaches a read where the call follows `errno = 0` and
precedes the read in the statements of one block, or in the branches of an
`if` there, and nothing between them may set `errno`. A `strtol` with a
NULL end has no `Stmt::Parse`, so its `errno` is refused.

**The end of a number**, beyond what Cursors above describes:

| C | Rust, or the refusal |
| --- | --- |
| `strtoul`, `strtoull`, `strtod`, `strtof` with `&end` | `parse_uint_prefix`, `parse_float_prefix::<f64>`, as `strtol` (R36) |
| `char *end = text;` or `end = NULL;` before the call, with nothing reading it between | the store is dropped: the call always stores the end (R36) |
| the end read before any call, or stored by anything else and then read | refused |
| a base other than 10: a constant 0, 8 or 16, or a variable | refused: no rule yet |
| an end in a field, a global or a parameter (`char **out`) | refused: a pointer to a pointer |
| a NULL text | undefined in C |

**The result of `scanf`** is the number of conversions that stored a value,
or -1 (`EOF`) where the input ended before the first of them:

| C | Rust, or the refusal |
| --- | --- |
| `scanf(…) == N`, `!= N`, `>= N`, `< N`, `> N-1`, `<= N-1` for the format's `N` conversions, so `< 1` and `> 0` with one | `if let Some(…)`, `let Some(…) = … else` (R11) |
| any other use of the value: `int n = scanf(…);`, `scanf(…) == EOF`, `!= EOF`, `> 0` with two, `== 1` with two, `== 0`, `while (scanf(…) != EOF)`, a `switch` on it, `return scanf(…);`, an argument | the count as a value (R11): `match scan_int_or_end() { Ok(scanned) => { x = scanned as i32; … 1 } Err(ScanError::End) => -1_i32, Err(ScanError::NoMatch) => 0 }` for a first number, `if let Some(scanned) = read_byte() { … } else { -1_i32 }` for a first `%c`, and `if let Some(…) = … { … } else { k }` for each later conversion, where `k` were stored before it; `scan_int_or_end` tells the end of the input after the white space from text that is no number |
| a comparison of that value with a constant that decides it for every count (`scanf("%d", &x) < 5`) | the same block; nothing is folded |
| `%c` at the end of the input, white space in the format | as R11; white space stores nothing and counts nothing |
| `%*d`, `%s`, `%[`, `%n`, `%f`, a width | refused: no rule yet |
| `sscanf`, and `fscanf` on another stream than `stdin` | refused |

### Pointers to functions

A pointer to a function is a value of Rust's type `fn(A, B) -> R`: it is
`Copy`, it points to no memory that the program owns, and none of the
pointer facts but `nullable` apply to it. The analysis does not treat it as
a pointer to memory.

| Shape | Rust, or the refusal |
| --- | --- |
| the type `R (*)(A, B)`, also through a `typedef`, and a function type `typedef R op(A, B);` used as `op *` | `fn(A, B) -> R`; `fn(A)` for `void` |
| a parameter written as a function, `void run(int f(int))` | the same: C makes it a pointer |
| where it is kept: a local, a parameter, a result, a field of a record, a global, an element of an array (`ops[i]`) | the type in that place; a record with one stays `Copy` |
| its value: a function's name, `&f`, `*f`, another such pointer, `c ? f : g`, a call's result | the function's path `f`, the value |
| NULL may reach it | `Option<fn(A) -> R>`, `None`, `Some(f)`, with the tests and the narrowing of R10: `if (p)`, `p != NULL`, `if (!p) return;` |
| a call: `p(x)`, `(*p)(x)`, `s.op(x)`, `ops[i](x)`, `select(k)(x)` | `p(x)`, `(s.op)(x)`, `ops[i as usize](x)`, `select(k)(x)` |
| a call through one that may be NULL where no test holds | `p.unwrap()(x)`, as for any pointer that may be NULL: C's behaviour is undefined there |
| the functions that reach one type use globals | every function whose address is taken and that has this C type takes the same `globals` parameter, the strongest that one of them needs (`&mut Globals` over `&Globals`), and each call through such a pointer passes it; the caller then uses the globals as they do |
| a parameter or a result of the pointed-to function is a pointer (`int (*)(const char *)`) | refused: `a pointer to a function that takes a pointer`; the functions that reach it would have to agree on one borrow |
| a parameter or result that is a record, a number, an enumeration | by value, as in any function |
| two such pointers compared, `p == f`, `p != q` | refused: `a comparison of two functions`; Rust gives two functions with one body one address, or one function two |
| a cast to another function type, to `void *` or to an integer, and `%p` | refused |
| a pointer without a prototype, `int (*f)()`, kept in any of these places | the parameters of the functions that reach it, found by `normalise::protos` before the analysis: `fn() -> i32` for functions defined with none, `fn(i32, f64) -> R` for those with parameters, `Option<fn() -> i32>` for one that only holds NULL. All such pointers with one result type are one group, which is coarser than the slots that hand them on |
| functions with different parameter lists reach one group | refused: `a pointer to a function without a prototype that holds functions of several types` |
| a call through it with another number of arguments, an argument of another type than the parameter after C's default promotions, or a parameter that the promotions change (`char`, `short`, `float`) | refused: `a call through a pointer to a function without a prototype with arguments its functions do not take`; C's behaviour is undefined there |
| a call through it that no function reaches | refused |
| a function declared without a prototype, `int zero();`, called by name | as any call by name |
| a variadic one, `int (*f)(const char *, ...)` | refused: `a pointer to a variadic function` |
| the address of a library function (`&strlen`), and one handed to the library (`qsort`, `bsearch`, `signal`, `atexit`) | refused: no rule yet |
| a pointer to a pointer to a function, and `sizeof` of one | refused |
| callee and arguments with side effects, `ops[i++](i)` | C leaves their order open; Rust evaluates the callee first |
| a function that reaches a pointer and is never called by name | still translated, and not reported as unused |
### `void` pointers

C writes `void *` where the type of the memory is not said. Rust always
says it, so the translation finds it: the **one type** that a `void`
pointer holds is the type of every pointer that is stored in it and of
every pointer it is converted to, here and wherever it is handed on. The
rewrite `normalise::voids` finds it before the analysis runs, gives the
slot that type and drops the conversions; from there the pointer is one
like any other and the pointer rules apply.

| Shape | Rust, or the refusal |
| --- | --- |
| a parameter `void *p` that the function converts, `unsigned char *d = (unsigned char *)p;`, and that every caller gives bytes | `p: &[u8]`: the byte view of bytes is the bytes |
| a `void *` local, parameter, result, field or global that always holds a `T *` and is always read as one (`void *ctx` that carries one record) | that place with the type `T *`, and its rule |
| slots that hand the pointer on to each other (an argument, an assignment, a `return`) | one group with one type: they are found together |
| a conversion with or without a cast, `int *q = p;`, `(struct s *)p`, `*(T *)p`, `((T *)p)->field` | the pointer itself |
| `char`, `signed char` and `unsigned char` | different types here: a `char` read through `unsigned char *` has another value when it is widened. Refused as two types until casts between pointer types have a rule |
| NULL in it, a test for NULL | as for any pointer (R10) |
| the result of `malloc`, `calloc` kept in a `void *` before it is converted | the type it is converted to; the allocation is of that type |
| nothing says a type: it is only handed on, compared or tested | refused: `a `void` pointer whose type nothing says` |
| two types meet: bytes of an `int` or a record (`fwrite(&x, …)`, a hash of any memory), a value of one of several types with a tag beside it | refused: `a `void` pointer that holds a T and a U`. The bytes of a number would be `to_ne_bytes`, and one of several types an `enum` |
| a function kept in it, `void *f = (void *)func;` | refused: `a function kept in a `void` pointer: no rule yet` |
| arithmetic on it (`p + 1`, a GNU extension), `sizeof(void)`, `*p` | refused |
| `void **`, and a `void *` in an array | refused: a pointer to a pointer, a pointer in an array |
| a `void *` parameter of the library (`memcpy`, `memset`, `memcmp`, `free`) | the library's rules, which take the argument's own type |
| `%p` | refused |
| a size beside it in bytes, `(void *p, size_t len)` | stays the program's number: it counts elements only where the type is a byte, and no rule changes it |
### `goto` into a block

A `goto` into a block from outside it (R68) is rewritten in
`normalise::enter` before the first analysis. The label moves to the front
of the statement that holds it, and a local `entered` of the function (with
a number where the name is taken) says that the statement is entered at the
label: the condition is not evaluated then, and what stands before the label
is skipped. Where the label is still in a block that the jump is outside of,
the statement around it moves it again, and only the innermost statement
clears the flag. A store of 0 to the flag that nothing reads afterwards is
left out, as the compiler refuses it. R48 translates a jump that now comes
before the label, R49 one that comes after it.

C enters the scope of a declaration before the label without its value.
Where the part after the label stores to the whole variable before it reads
it on every path, the declaration moves to the start of the function, as
`let mut v: i32;` without a value (`Program::unset`), and its initialiser
stays where it was, as a store that entering at the label skips, so that it
keeps its time. The check is conservative: a read in a condition, in a
call's argument, after a branch that may not store, or past a label is a
read. A declaration without an initialiser needs the same of the part
before the label after it, and of a loop's next round.

| The label stands in | The jump comes from | Rust form |
| --- | --- | --- |
| the branch of an `if (c) { A; L: B; }` | before the `if`, or after it | `L: if (entered \|\| c) { if (!entered) { A; } entered = 0; B; }` |
| the `else` of an `if (c) { T } else { A; L: B; }` | before the `if`, or after it | `L: if (!entered && c) { T } else { if (!entered) { A; } entered = 0; B; }` |
| the body of `while (c) { A; L: B; }` | before the loop, or after it | `L: while (entered \|\| c) { if (!entered) { A; } entered = 0; B; }`; `break` and `continue` in `B` stay the loop's |
| the body of `do { A; L: B; } while (c);` | before the loop, or after it | `L: do { if (!entered) { A; } entered = 0; B; } while (c);` |
| a plain block `{ A; L: B; }` | before the block, or after it | `L: { if (!entered) { A; } entered = 0; B; }` |
| any of these inside another of them | before the outer statement | both rewritten, the outer one without `entered = 0` |
| the body of a `for` | outside it | refused: `` `goto L` into a `for` loop, past its initialiser `` |
| a block after a declaration of a number in `A` that `B` stores to before it reads it | outside it | `let mut v: i32;` at the function's start, and `v = init;` in `if (!entered) { … }` |
| a block after a declaration in `A` that `B` may read first, of an array, a record or a pointer, of a variable whose address is taken or whose name another variable of the function has | outside it | refused: `` `goto L` into a block past a declaration that the part after the label reads `` |
| a block after a variable-length array | outside it | C rejects it |
| a statement that another moved label enters too: two labels of one block, or one in the branch and one in the `else` | outside it | refused: `` `goto L` into a block that another label of the same statement enters `` |
| the other branch of the same `if`, or a block that the jump's own statement holds | the statement itself | refused as before: `` `goto L` into a block from outside it `` |
| a `case` of a `switch` | outside it | refused as before: `` `goto L` to a label under a `case` `` |
| a statement expression | outside it | C rejects it |

### Statement expressions

`({ statements; value; })` is a block that is an expression, as every block
of Rust is. The front end reads Clang's `StmtExpr` as `c::ExprKind::Block`,
the statements and the value that the last of them gives, if it is an
expression; the rule prints `{ statements; value }`. The header `assert.h`
writes its macro with one, and that expansion is a construct of its own,
`c::ExprKind::Assert`, since Rust has a word for it.

| Shape | Rust, or the refusal |
| --- | --- |
| `assert(e);` as glibc expands it, `((void) sizeof ((e) ? 1 : 0), __extension__ ({ if (e) ; else __assert_fail(…); }))` | `assert_holds(e, "FILE:LINE: FUNCTION: Assertion `E' failed.");`, with the texts and the line that the expansion holds, in one literal. The helper `assert_holds` prints the message to standard error and calls `std::process::abort()` where `e` is false: `assert!` would end the program with another exit code than C's `abort` |
| `assert(e)` with `NDEBUG`, `((void) (0))` | nothing: a statement without an effect |
| the condition of an `assert`: a number, a comparison, a pointer, `&&` and `||`, a call, one with a side effect | the first argument, by the rule for conditions; C evaluates it once, since the operand of `sizeof` is not evaluated, and so does the translation |
| an `assert` as the left side of a comma, `(assert(n > 0), n)` | the statement before the value, as for any comma |
| an `assert` where a value is needed | refused: it has none |
| a value: `int x = ({ int y = f(); y + 1; });` | `let x = { let y = f(); y + 1 };` |
| the positions of a value: an initialiser, the right side of an assignment, an argument, an operand, a returned value, the condition of an `if` or a loop, the header of a `for`, inside another statement expression | the same block in that position; a condition is evaluated as often as C evaluates it |
| a statement of its own, `({ a; b; });`, also with a value that nothing reads | a block statement `{ a; b; }`; a last expression without an effect is dropped as in any expression statement |
| no value: the last statement is no expression, or the block is empty, `({ })` | a block statement where it is a statement; refused where a value is needed |
| the value's type: an integer, a floating-point number, `_Bool`, an enumeration; with C's promotions of the last expression | a block of that type |
| a value that is a pointer, a record, an array or a function | refused: `a statement expression that gives a pointer`, …; the analysis has no flow out of a block yet |
| statements inside: declarations with and without a first value, expression statements, `if`, loops and `switch` that stay inside | the statements of a block. A `break` or `continue` inside belongs to a loop or `switch` inside |
| `return`, `goto`, a label, and a `break` or `continue` that leaves the statement expression | refused: `a jump out of a statement expression` |
| a store inside to a variable outside, `x + ({ x = 2; x; })` | as for any assignment inside an expression: the operands run from left to right |
| a local inside with the name of one outside, `int x = 1; int y = ({ int x = 5; x + 1; });` | the block is a scope: the inner name shadows, and `names` gives each its own name only where one Rust scope would hold both |
| a pointer inside to a variable outside, `({ int *p = &x; *p += 1; x; })` | the rules for a local pointer; the analysis reads the statements inside like any others of the function |
| the address of a local of the block as its value | refused with the pointer values above |
| a macro's `__typeof__(a) _a = (a);` | the type that Clang resolved it to; refused where Clang names none |
| a statement expression outside a function | not C |

Every stage that walks the statements of a function walks those inside an
expression too: the analysis, `join`, `ends`, `cursors` and `narrow` find
them through one walk in `c.rs`. A rewrite that cannot be right for a
statement inside an expression (an end pointer whose call is there, a
cursor that is stepped there) leaves it and the program is refused.

## Not yet built

The direction for each, so that the first rule for it starts in the right
place:

- **Text.** A `char *` that only holds string literals and is read as a
  whole is `&str` (R13). A `char` array stays bytes, as does every pointer
  into one; the library reads its text up to the NUL through `text_of` and
  `until_nul` (R15), and a string literal that such a pointer meets is
  `b"…\0"`. `argv[i]` is text too, `&args[i]`, and has no rule yet where
  it meets bytes. Still to build: text that a record or a global holds
  (`&'static str` when only literals reach it), the functions of `string.h`
  that return a pointer (`strchr`, `strstr`, `strtok`), a width in `%s`, and above all a `String`
  for a buffer that the program only fills and reads as a whole, which
  needs a fact that says so.
- **Numbers in bytes.** A number read or stored at once through a cast
  of a pointer to bytes is made from the bytes (R69). Still to build: the
  bytes of a number, `(unsigned char *)&n`, which is `n.to_ne_bytes()[i]`
  where the bytes are only read; a cast pointer that is kept, which needs
  a slot that holds a slice of bytes and the number's size; `signed char`
  bytes; and an update in place, `*(T *)p += v`.
- **Global pointers.** A global pointer that is NULL and never changes
  waits for a rule that reads each use as NULL; one that changes needs a
  slot for a borrow of a global. A pointer field that never changes is
  its place only where every record of the type is a global (R64); one
  reached through a pointer, or of a record copied or sized, waits for
  the same slot. The address of a compound literal other
  than as a global pointer's initialiser, and of an array's, as a local's
  initialiser or an argument, is refused: such a literal needs a variable
  of its own where it is written.
- **Input.** `scanf`, `fgets` and `fread` read through helpers over
  `std::io::stdin().lock()` that take a byte at a time, so that every way
  of reading shares what is left, as in C (`lower/input.rs`, and
  `read_line` and `read_items` in `library.rs`). `getchar`, `%s` and `%f` belong
  there too. The result of `scanf` is its count wherever it is a value.
- **Pointers to pointers and output parameters.** See Pointers to
  pointers above for the shapes and which of them have a rule. Beside
  them: an output parameter that is really a result (`int parse(const
  char *s, int *out)` is `-> Option<i32>`, `T **out` that is stored
  before it is read a second result), a `T **` that is only read through
  as the `Option<&T>` it points to, a place behind a pointer that holds a
  borrow (`&mut &str`, which needs a lifetime for each level), and an end
  of a number in `argv[i]` for an `i` that changes, which needs the
  argument in a variable of its own.
- **A pointer beside the variable it names.** A local pointer that only
  names one fixed place is that place (R61). One that names a place through
  another pointer, `&q->f`, or at an index that changes, `&a[i]`, or that
  holds two places in turn, is still refused where the variable is used
  while the pointer is: it needs the place split from the borrow, or the
  pointer as an index beside the array as for a cursor. A pointer that
  gets its address from another pointer is not followed by
  `Facts::in_use`, so Rust's borrow checker rejects what it misses.
- **Cursors that may be NULL.** A cursor is an index (see Cursors above). One
  that NULL reaches, or that a library function returns (`strchr`,
  `memchr`, `strstr`), should be an `Option<usize>` from `position`; a
  cursor over a program argument needs the bytes of the argument; and a
  cursor that gets the end of a number (`p = end` in a loop over a line)
  needs the end pointer in the cursor's group.
- **Walking a list.** `while (p) { …; p = p->next; }` needs a narrowing for
  loops like the one for `if`; today it uses `is_some()` and `unwrap()`,
  and `p.take().unwrap().next.as_deref_mut()` for the step of a borrow
  that is stored through.
- **Shared memory.** Two slots that own the same memory do not compile: the
  second use is a use after a move, which is refused where the first slot
  is a local. `Rc` belongs only on a slot that a fact
  proves shared, and `RefCell` only where it is also written; neither is a
  default.
- **Others.** `goto` into a `for` loop or into a block from a statement
  that holds it, past a declaration that the part after the label may read
  before it stores to it, or of an array, a record or a pointer, into a
  statement that another label enters too (R68), to a label under
  a `case`, from a `switch` into a block entered at a label, and a
  local of one part of such a block that a later part uses (a jump forward
  is a `break` of a labelled block, R48, and one back a loop entered at a
  part, R49), a computed `goto`, a `case` that falls through with
  a variable that a later case uses, a `case` inside a loop (Duff's
  device), unions other than the bits of a number (an `enum` when a
  tag says which member is live, `to_ne_bytes` for bytes beside a number),
  `+=` and `++` on a bit-field, variadic functions, two pointers to functions without a prototype with one result type that hold functions of different parameters (a group for each set of slots that hand them on, as `voids` finds), an array whose length is an expression
  (Clang prints it only as text), a local that gets its first value in a branch or
  in a loop (`let x;`).

What two reviews of the passing translations found that no rule writes as
Rust would, most frequent first. Each needs the proof named with it, and
keeps C's promotions, checked arithmetic and the input it consumes:

- **Types that inference gives.** `let n: i64 = parse_int(…)`: a call of
  a helper.
- **A value from an expression instead of a variable set later.** `let mut
  x = 0;` and then a store in a branch or after a read, where `let x =
  if …` or `scan_int().map_or(0, …)` says it. Needs every store and read
  of the variable, also through pointers.
- **Stores that nothing reads.** A first value that the next statement
  overwrites, and a result that is never read. Needs liveness; the
  expression stays where it can stop the program or read input.
- **Promotions with no effect.** `args.len() as i32 != 2`, where `argc`
  could be compared as the `usize` it is. Needs the range of the other
  side; a `char`'s sign extension and a narrowing stay.
- **A step inside an index.** `*dst++ = *src++` is `dst[{ let before =
  dst_at; dst_at += 1; before }] = …`; the steps belong after the
  statement, `dst[dst_at] = src[src_at]; dst_at += 1; src_at += 1;`, where
  nothing else in the statement reads the cursors.
- **Statements of C in conditions.** `if !({ x -= e; x } > 0)` for a
  store in a condition.
- **The spelling of numbers.** `0xAAAA` printed as `43690`. Needs the
  literal's text from Clang.
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
- **A test for NULL and then `unwrap`.** `if found.is_none() || found
  .unwrap().id == 0` and `p = find(…); if p.is_none() { break; }` with
  `p.unwrap()` after it, where `narrow` (R10) only knows the plain `if`.
  `let Some(p) = find(…) else { break; }` and `match found { Some(found)
  if found.id != 0 => …, _ => … }` need the narrowing for a condition with
  `||` or `&&`, and for a pointer that a loop carries.
- **A buffer that only holds text.** `store_text(&format!(…), &mut
  buffer)` and `copy_string` between `[u8; N]` arrays where the program
  fills the buffer as a whole and reads it as a whole: a `String`, as the
  **Text** entry above says, with the fact that no byte of it is read or
  stored by itself.
- **A search that returns an element.** A cursor that a function returns
  or hands on is the rest of the array from there, `&a[p..]`, and the
  caller reads `found[0].value`, since the cursor may be at the end of the
  array. Where the return is under the loop's test `p < end`, the cursor
  is an element, `&a[p]`, and the caller reads `found.value`. Needs the
  range that the test proves, up to the return.

What a third review found beside these, most frequent first:

- **A function's value as `return` in both branches of its last `if`**, or
  as a local that the last statement only returns: the `if` and the
  expression are the value.
- **The checked parsers scan a number three times**, for its end, its
  value and its range. One scan can keep all three, also whether a
  significand had a digit that is not zero.
- **A result and a flag.** `fn parse_val(text: &[u8], val: &mut i32) ->
  bool` that stores once where it returns true is `-> Option<i32>`. Needs
  every caller: the output is not read where the call fails.
- **A reborrow that nothing needs.** `as_deref_mut()` on an `Option` that
  nothing reads again could take it.
- **A vector of zeros that a loop fills.** `vec![0; n]` and then `ones[i] =
  1` for every `i` is `vec![1; n]`, when the loop stores each element once
  and nothing reads the vector before.
- **A number that is only a truth.** A field that only ever gets 0 or 1 and
  is only tested is a `bool`. Needs every store and read of it.
- **A test before a loop with the same test.** `if s[at] != 0 { while s[at]
  != 0 { … } }` where nothing is between them and the test has no effect.
- **Tests that are one condition.** `if entry == 0 { if x == 1 { … } }`
  with nothing else in the outer branch is `if entry == 0 && x == 1`; the
  float helpers have the same shape. Needs no analysis: the order and the
  short circuit stay.
- **Functions that nothing reaches.** A copy with one parameter for two
  (R53) leaves the function it was made from, and C programs carry helpers
  that nothing calls. Needs what `main` reaches, through calls and through
  the functions whose address is taken.
- **The steps of C's cleanup.** `if let Some(mut a) = Some(a) {
  drop(take(&mut a.items)); drop(a); }` and `p = None` after a `free`,
  where the owner's end does it all. The wrapped test needs no analysis;
  dropping a `free` needs the last use and that the order of releases
  shows nowhere.
- **The bytes of a text found again for each search.**
  `find_byte(until_nul(&in_[s..]), c)` in each round of R52's loop, and
  `s.iter().take(until_nul(s).len())` for `memchr(s, c, strlen(s))`,
  where one `let text = until_nul(in_);` serves. The second needs no
  analysis; the first needs that nothing stores to the array meanwhile.
- **The type of a callback written out.** `Option<fn(&mut Globals, i32) ->
  i32>` at each use of a `typedef`: a `type` with the C name. Needs the
  name kept through the front end.
- **Labels of a `case` that are a range.** `entry @ (7 | 6 | 5 | 4 | 3 | 2
  | 1 | 0)` and `matches!(entry, 7 | 6 | 5)` are `0..=7` and `5..=7`.
  Needs no analysis.
- **`do { … } while (0)`.** `loop { …; if !false { break; } }` is a block
  where no `break` or `continue` is in it. Needs no analysis.
- **A place that is only tested, in a variable.** `let mut bad = s.find(',');
  if bad.is_some() { … } bad = s.find(';');` is `if s.contains(',')` twice.
  `normalise::found` knows the local is only tested.
- **A record for two numbers.** `DivT { quot: x / y, rem: x % y }` for a
  local whose two members are read is `let (quot, rem) = (x / y, x % y);`.

What a fifth review found beside these, in programs outside the
development set too, most frequent first:

- **Names that give way to names of another kind.** A local renamed for a
  field of `Globals` or for a function that its scope never calls, and a
  constant that takes its record's name with an underscore, in lower
  case: `static r_: R`. Needs the scopes kept apart when a name is
  chosen, and a fallback in capitals.
- **A rotation as two shifts.** `(x >> 9) | (x << 23)` for a `u32` is
  `x.rotate_right(9)`, also with `^` between the halves. Only for the
  exact shape: one unsigned operand without an effect, two constant
  counts that add up to its width.
- **An exchange.** Three `^=` between two locals are `std::mem::swap`, and
  a call of a function that only exchanges two elements of one array,
  for which R54 splits the array, is `a.swap(i, j)`. The second needs
  what the callee does.
- **A place or -1.** A search that returns an index or `-1` is `->
  Option<usize>`. Needs which returns say that nothing was found, that
  the others are no negative numbers, and every caller.
- **A `do` loop whose test is a constant.** R58's block and its label for
  a `continue`, and `if !true { break; }`, where the test is never false:
  a plain `loop`. Needs no analysis. A `continue` that ends the body
  needs no statement either.
- **The labels of a `switch` as numbers.** `match action { 11 => …, 12 =>
  … }` where C names enumerators: the constants' names. Needs the
  enumerator kept beside the value of a `case`.
- **Statements after a jump forward.** A block that a `goto` leaves at
  once keeps what C wrote after the `goto`, and a `return` follows a
  `return`. What follows a statement that always leaves can go, where no
  label stands in it.
- **An array that is replaced by a longer copy.** `let mut longer = vec![0;
  n]; for i in 0..used { longer[i] = items[i]; } drop(take(&mut items));
  items = longer;` is `items.resize(n, 0)`. Needs that the copy is of the
  whole used part and that nothing else points to the old array.

What a sixth review found beside these, most frequent first:

- **A record filled field by field.** `let mut node = Box::new(Node::
  default()); node.data = data; node.next = None;` is `Box::new(Node {
  data, next: None })`. Needs that nothing reads the record or hands it
  on before its last field is stored, and the order of the values kept.
- **A test negated around its comparison.** `if !(left != 0)` is `if
  left == 0`, and `i32::from(!(n != 0))` is `i32::from(n == 0)`. Needs no analysis; a comparison of floating-point numbers by order
  stays, since a NaN answers both ways.
- **A length beside the vector that a record owns.** `list.items[list.used
  as usize] = x; list.used += 1;` after R59's block is `list.items.
  push(x)` where the vector holds exactly the elements in use. Needs
  that the length and the storage always agree, and that no spare
  element is read; the `Option` and the block for a size of 0 stay until
  then.
- **A pointer that only names one place and causes no conflict.** `let p:
  &i32 = &x; if *p != 2` is `if x != 2`. R61 with its own proofs, for a
  pointer that the borrow checker would accept as well.
- **Truths combined as numbers.** `i32::from(a < n) & i32::from(b < n) !=
  0` is `(a < n) & (b < n)`, which evaluates both sides as C does. Needs
  no analysis where both operands are comparisons.
- **A missing value handed on by hand.** `let Some(buffer) = buffer else {
  return None; };` is `let buffer = buffer?;` where the function returns
  an `Option` and the branch does nothing else. Needs no analysis.

What a seventh review found beside these, most frequent first:

- **A loop that counts down or by more than one.** `let mut i = 5; while
  i > 0 { … i -= 1; }` is `for i in (1..=5).rev()`, and a step of 2 is
  `(0..=n - 2).step_by(2)`. R3's test of a counted loop, for a step
  other than one up; a step that could pass the type's limit stays a
  `while`.
- **A function that only answers yes or no.** `fn is_empty(…) -> i32`
  whose every `return` is 0 or 1, compared with 0 by its callers, is `->
  bool`. Needs every return and every caller, also through a pointer to
  the function.
- **A table built empty and then filled.** `(0..n).map(|_| Vec::new()).
  collect()` followed by a loop that stores `vec![0; m]` in every row is
  `vec![vec![0; m]; n]`, and `[0; 2]` with a store to each element is
  the array of the values. Needs that every element is stored once
  before anything reads it.
- **A record on the heap that nothing links.** A `Box<Stack>` whose
  record holds a `Vec` and is in no list is `Stack`. Needs that no
  pointer to it is kept in another record; a node of a list stays a
  `Box`.
- **A length that the type of a parameter gave.** `fn mix(state: &mut
  [u64], block: &[u64])` for C's `uint64_t state[8]` is `&mut [u64; 8]`
  where every caller hands an array of that length.
- **A borrow handed back to the caller that lent it.** `p = grow(p, n);`
  for a `grow` that returns its own parameter is `grow(p, n);` with no
  result. Needs that every `return` names the parameter and every caller
  stores the result where the argument came from.
- **A star before a method.** `(*head).take()` for a `head: &mut
  Option<…>` is `head.take()`. Needs no analysis.
- **Limits and a new line spelled as numbers.** `2147483647` is
  `i32::MAX`, and `print!("{}", 10 as u8 as char)` is `println!()`: with
  the spelling of numbers above.

## Known weak spots

Places where a rule assumes more than the analysis proves. Each should
become a fact and a refusal:

- An owner moved out of a field or a global leaves `None` behind
  (`take()`); a C program that reads the field again before storing to it
  would see NULL in Rust. A global that its own function reads again is
  refused, and so is a field that its own statement, the `return` that
  ends the function or a later statement of the block reads again before
  a store. Not refused, and translated wrongly: a field that is read
  after the block of the move, in the next round of a loop around it or
  after a `goto`; one that the caller reads after the function moved it
  out; one that is read under another name of its record (`s = &x;` and
  then `x.p`, or the name that a second test `if (s)` gives the pointer);
  one whose index variable changed before the store that seems to
  restore it (`q = a[i].p; i++; a[i].p = NULL;`); and a global that
  another function reads. The refusal is also wider than it must be: any
  call of a function of the program between the move and the store
  counts as a read.
- A pointer into the middle of an array that is no cursor is the rest of
  the array from there (`&a[1..]`), so an index or a step back before it
  (`p[-1]`) stops the program where C reads the element. A global pointer
  or field that never changes (R62, R64) and a local that R61 replaces
  are no such pointers: `normalise::offsets` reads `p[n]`, `*(p + n)`,
  `*(p - n)` and `(p + n)->f` as an element of the array itself. A
  parameter, and a local that stays a borrow, are.
- A cursor that steps one before its array stops the program there. C
  leaves that undefined, the sanitizers do not report it, and compiled C
  runs on. It is refused where the statements show it (see Cursors), and
  not where the index is 0 only for some inputs, or after a loop.
- `%c`, `%s` of a program argument and `argv` assume ASCII; C passes any
  bytes through. `text_of` prints bytes that are not UTF-8 as U+FFFD.
- A conversion from a floating-point number to an integer saturates in Rust
  where C's behaviour is undefined, and a NaN prints differently.
- `read_line` takes an error of the read for the end of the input: after
  some bytes it returns them, where `fgets` returns NULL. `read_items` does
  too, as `fread` does, but leaves no error for `ferror`.
- `&args[i]` stops the program where `i` is `argc`, and C reads NULL
  there: a program that only stores `argv[argc]` runs on in C.
- `%08.2f` of an infinity is padded with zeros in Rust and with spaces in
  C.
- A pointer that a function returns into what a parameter may store
  through keeps the argument borrowed for as long as the result is used.
  A C program that reads the argument meanwhile, as
  `e = find(entries, …); printf("%d %d", entries[0].value, e->value);`
  does, is translated and does not compile.
- A pointer that a function returns from its parameter is not known to
  point to the caller's local: `int *p = identity(&x); return p;` is
  translated and does not compile, where the result should be refused.
- A copy of a pointer is followed through the statements in their order
  (`Facts::in_use`). Two reads in one statement, as `printf("%d %d", *q,
  *p)` for a `q` that is stored through, and a copy that is read after a
  `goto` and its label, are translated and do not compile.
- A function that takes a record out of a list through a local, as `n =
  p->next; p->next = n->next; return n;`, is translated and does not
  compile: the local borrows the field that is then stored to.
- A local pointer to a global, or to a `static` local, is a borrow of it
  that the analysis does not follow: `int *p = &x; x = 2;` with a later
  read of `*p`, for a global `x`, is translated and does not compile.
- A record that owns memory and is passed by value is moved into the
  call: `n = get(s);` with a later read of `s.next` is translated and does
  not compile, where C copies the record.
- A function that nothing names and that returns an owner out of a record
  it borrows takes the owner out (`take()`); it is refused only once
  something names it.
- `getenv` reads the environment with `std::env::var`, which gives no
  value for a variable that is not UTF-8, where C finds the bytes, and
  may differ from C for a name with `=` or a NUL.
- A local pointer to an array is a borrow of it. A use of the pointer
  after a store to the array, or after a read of it where the pointer is
  stored through, is refused, but one call that takes the pointer to
  store through and the array itself, as `char *p = s; memmove(p + 1, s,
  3);` does, is translated and does not compile, as is the same with a
  parameter `s`, whose memory the analysis does not follow.
- The compiler follows constants through locals and refuses arithmetic
  that it proves to overflow, also in a statement that never runs: `char
  data = CHAR_MAX; char twice = data * INT_MAX;` in a function that the
  program does not call for its inputs is translated and does not
  compile. An operation that the lints refuse as written (`x - x`, `x &
  0`, a constant index beyond its array) is refused.
- Other code that the compiler or its lints refuse as written is still
  translated and does not compile: two constants (`0 * 3`, `7 & 0`), `x %
  1`, a shift by the width of its type or a division by a literal 0 in a
  statement that never runs, and an index that is a `const` local beyond
  its array. A constant index beyond its array is refused also in a
  branch that a constant test drops (`if (0)`), where the translation
  would compile.
- The address of the end of an array, `&a[N]`, is an index only for a
  cursor or the end of a loop; as an argument for a borrow of one
  element it is translated and does not compile.
- A borrow of an element of an array of pointers that is held across a
  store to another element, `t = a[i]; a[j] = t->next;` with a later read
  of `t`, is translated and does not compile; so is a record that is
  read through a second name (`r = s;`) while a field of it is moved out,
  and a local that borrows a part of a record, `t = a->next;`, held
  while a statement reads `a->next` by its own path.
- A conditional that moves a local owner on one side, `take(k ? s->p :
  other)`, followed by `if (k) free(other);`, is translated and does not
  compile: the compiler does not know that the two tests agree.
- A copy of a function with one parameter for two (R53) still declares
  the locals that were pointers of the two: `int *p = a, *q = b;` in
  `f(void *a, void *b)` called as `f(x, x)` is one `&mut` and one `&` of
  the same memory, and does not compile.
- A failing `assert` aborts, and C loses what it printed to a `stdout`
  that is not a terminal and that it has not flushed. Rust's `stdout`
  writes each line as it ends, so the translation prints those lines.

## The listing

`translate` stops at a program's first refusal. `blockers`, the second entry
point (`transpiler --list FILE.c`), runs the same stages and goes on after
each refusal, to say what else the program needs. It prints a line for each
refusal: the stage (`read` for the front end, `rewrite`, `rules`), the
function, the refusal as `translate` words it, and how often the function
meets it. Its first line is the refusal of `translate`, and it is empty for
a program that is translated.

A stage goes on by leaving out what it refused: the front end a top-level
declaration or a statement, the rules a record, the globals, a function's
signature or a statement. `translate` runs the same code with no list to
add to, where every refusal ends the translation as before.

The list is a lower bound. It hides:

- what the rules would refuse in a statement that the front end refused,
  and all but the first refusal of any statement;
- the body of a function whose parameters or result the front end refused,
  and everything after the point where a stage panics on a program with
  parts left out, which ends the list with a line that says so;
- what depends on a part left out: the facts are those of the program
  without it, so a pointer may get another type, and a conflict may
  appear or go. A refusal that names a declaration left out earlier is
  marked `follows`, since it may only follow from that one;
- everything after the translation: what `rustc` rejects, and output that
  differs from the C program's. An empty list says that the program is
  translated, not that it passes.

## Tests

`tests/translate/` holds one test per subject. `check` compiles the C program
with Clang and the translation with `rustc` (`-F unsafe_code -D warnings`,
overflow checks on), runs both on each input and compares output and exit
code; the test then asserts the lines that show the rule's shape, so a
translation that passes but reads worse fails the test. `tests/dev.rs` checks
that every development program is translated or refused, never a panic, and
that its listing starts with that refusal.
