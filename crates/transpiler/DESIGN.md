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
Clang's JSON -> clang -> c::Program -> unions, voids, join, ends, found -> facts -> cursors -> facts -> narrow -> facts -> lower -> rust -> text
```

| Stage | File | Does | Knows |
| --- | --- | --- | --- |
| Front end | `src/clang.rs` | Reads Clang's JSON into the C tree | Clang's JSON; nothing about Rust |
| C tree | `src/c.rs` | Types, expressions and statements with their C types, and walks over them | C |
| Analysis | `src/facts.rs` | Facts about each pointer, variable, global and function | The whole program |
| Rewrites | `src/normalise.rs` | Rewrites C to C so that one statement means one thing: `unions` makes a union that holds the bits of a number an integer, `voids` gives a `void` pointer the one type it holds, `join` declares a local where it gets its first value, `ends` makes the end that `strtol` stores a result of the call, `found` makes a local that only holds what a search found and is only tested an `Option<usize>`, `cursors` makes a pointer that walks an array an index, `narrow` names a pointer that a test found not to be NULL | The facts, for `cursors` and `narrow` |
| Rules | `src/lower/` | Translates each construct into a `syn` tree | The facts, one construct at a time |
| Names | `src/names.rs` | The Rust name of each C name: snake case for functions, variables and fields, camel case for records, upper case for constants and enumerators. Two C names in one Rust scope never share a name | Rust's naming and keywords |
| Output | `src/rust.rs` | Builds `syn` expressions and prints the file with `prettyplease` | Rust's syntax |

The rules are split by subject: `lower/mod.rs` (the program, records, globals,
functions, `main`), `types.rs`, `expr.rs`, `stmt.rs`, `pointer.rs`,
`function.rs` (pointers to functions), `library.rs` (the C library) and
`input.rs` (standard input).

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
| `switch` | `match`; cases that fall through share one arm, `entry @ (2 \| 1) => { if entry == 2 { … } … }` |
| `goto out;` with `out:` later in the same block or one around it | `'out: { …; break 'out; … }` |
| `printf` with a literal format | `print!` or `println!`, the format converted at translation time |
| `const char *` that only holds string literals | `&str` |
| `if (p)`, `if (!p) return;` | `if let Some(p) = …`, `let Some(p) = … else { … }` |
| `malloc`, `calloc`, `free` | `Box::new`, `vec![zero; n]`, `drop` |
| `scanf("%d", &x)` | `if let Some(scanned) = scan_int() { x = scanned as i32; }` |
| `fgets(line, sizeof line, stdin)` | `read_line(&mut line)` |
| `fgets(line, size, stdin)` for a `char *line` | `read_line_within(line, size)` |
| `n = fread(bytes, 1, sizeof bytes, stdin)` | `n = read_items(&mut bytes, 1)` |
| `%s` of a `char` array | `text_of(&line)` |

### Pointers

A *slot* is a place that holds a pointer: a variable, a field or a function's
result. The analysis gives each slot nine facts, and they decide its type:

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
refuses with the reason instead: a result that points to a local, and two
arguments of one call that point into the same memory when the function
stores through one of them. Two elements of one array of which one is
before the other on every run are no such case: the array is split between
them (R54).

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
| a function without a prototype, `int (*f)()`, and a variadic one | refused |
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
- **Input.** `scanf`, `fgets` and `fread` read through helpers over
  `std::io::stdin().lock()` that take a byte at a time, so that every way
  of reading shares what is left, as in C (`lower/input.rs`, and
  `read_line` and `read_items` in `library.rs`). `getchar`, `%s` and `%f` belong
  there too. The result of `scanf` is its count wherever it is a value.
- **Pointers to pointers and output parameters.** `T **out` that is only
  stored through is `&mut Option<Box<T>>` or a second result. The end
  pointer of the `strtol` family is the one case built (R21, R36), as a
  second result of the call (see Cursors above). An end in `argv[i]` for an
  `i` that changes waits for the same rule, which needs the argument in a
  variable of its own.
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
- **Others.** `goto` to a label before it or into a loop (a jump forward
  is a `break` of a labelled block, R48), a `case` that falls through with
  a variable that a later case uses, a `case` inside a loop (Duff's
  device), unions other than the bits of a number (an `enum` when a
  tag says which member is live, `to_ne_bytes` for bytes beside a number),
  `+=` and `++` on a bit-field, variadic functions, an array whose length is an expression
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
- `getenv` reads the environment with `std::env::var`, which gives no
  value for a variable that is not UTF-8, where C finds the bytes, and
  may differ from C for a name with `=` or a NUL.
- A local pointer to an array is a borrow of it, and the analysis does
  not know which array a local points into. A call that takes the pointer
  to store through and the array itself, as `char *p = s; memmove(p + 1,
  s, 3);` does, is translated and does not compile.

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
