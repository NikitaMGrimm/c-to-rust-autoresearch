# Rule catalogue

Every translation rule the research loop adds or changes has an entry here, so
the results can be traced to rules. The loop gate rejects a change to `src/`
that leaves this file unchanged.

Entry format, newest last:

```markdown
## R1: short name

- C: the construct it translates, for example `printf` with a literal format
- Rust: what it emits for it
- Motivation: the development programs that failed without it, and how
- Changes: earlier rules it replaces or extends, if any
```

## R1: integer functions with printf and scanf

- C: function definitions over the integer types (definitions with names that
  C reserves, as from system headers, are skipped); `int main(void)` falling
  off its end; local variables, uninitialised ones as 0; `=`, `+=`, `-=`,
  `*=`, `/=` and `%=` as statements; signed `+ - * / %` and unary `-`;
  integer conversions; calls of the program's own functions; `printf` with a
  literal format of `%d`/`%i` with the flags `-`, `+` and `0` and a width,
  and `%%`, whose result is discarded; `scanf` with a literal format of `%d`
  into `&variable`; either format ends at its first zero byte, as in C
- Rust: `fn` with `i8`…`u64`, `let` with `mut` only for variables that are
  assigned or whose address is taken, raw identifiers for Rust keywords;
  `self`, `Self`, `super`, `crate`, `_` and `main`, and each of them
  followed by underscores, with one more `_`, so no two C names share a
  Rust name; the last `return` as the tail expression, C's `main` as `main_`
  called by `std::process::exit`; operands in parentheses unless atomic; `print!` or
  `println!` with the format converted at translation time; `crt::scanf`, a
  runtime emitted once that reads standard input byte by byte as glibc does,
  saturating out-of-range numbers as `strtol` does, and passes each value to
  one closure that matches the argument's index and assigns its variable, so
  a variable named twice is written twice in order. Every other construct,
  including `static` and `extern` local variables, becomes
  `compile_error!("unsupported C: ...")`
- Motivation: the transpiler emitted an empty file, so every development
  program failed to compile; `g1_040_storage_class_auto` needs exactly these
- Changes: none, the first rule

## R2: control flow and goto

- C: `if`/`else if`/`else`, `while`, `do ... while` and `for` (any part
  empty); `break` and `continue`; `goto` to a label among the statements of
  the block that holds the `goto` or of a block around it; comparisons of
  integers of the same type, `&&`, `||` and `!`, as conditions and as `int`
  values; integers as conditions; `++` and `--` as statements
- Rust: Rust's `if`, `while` and `loop` with a `bool` condition (`x != 0`
  for an integer), `loop` for a nonzero constant one; a `for` becomes its
  initialiser, in a block if it declares, and a `while` ending in the
  increment; the body of a `for` or `do` is a labelled block that
  `continue` leaves, so the increment or test still runs; jumps carry a
  label only when they leave a loop other than the innermost. A block with
  labels becomes `let mut __gotoN = 0; loop { match __gotoN { ... } }` with
  one arm per label, falling through by setting the next arm; `goto` sets
  the arm and continues that loop. Its integer variables are declared before
  the loop, with their initialisers as assignments, unless that would hide
  a variable of the same name that the block uses. Comparisons as values are
  `i32::from(...)`. `++`/`--` are `+= 1`/`-= 1` on `int` and `long`, whose
  overflow C leaves undefined, and `wrapping_add`/`wrapping_sub` on unsigned
  and narrower types, which wrap. A `goto` into a block, `++`/`--` used as a
  value and `break` or `continue` outside a loop stay unsupported
- Motivation: every development program but one uses `if` or a loop, and
  `g5_021_complex_goto` jumps forwards and backwards between labels inside a
  `while` body
- Changes: extends R1, whose function bodies held only straight-line code

## R3: characters and unsigned conversions in formatted I/O

- C: character literals of type `int`; `printf` conversions `%u`, `%o`,
  `%x` and `%X` with the flags and width of R1, of an `unsigned int` or an
  `int`; `scanf` conversion `%c` into a `char`; `fscanf(stdin, ...)`
- Rust: a character literal as its `int` value, with an `_i32` suffix when
  negative; `{}`, `{:o}`, `{:x}` and `{:X}`, without the `+` flag, which C
  ignores on unsigned conversions, and an `int` argument converted
  `as u32`, whose bits glibc prints; `%c` stores the next byte of input
  without skipping white space, converted `as i8`; `fscanf` on `stdin` is
  `scanf`. `%c` in `printf` stays unsupported
- Motivation: `g1_010_integer_overflow` reads a character with
  `fscanf(stdin, "%c", ...)`, starts from `' '` and prints with `%02x`
- Changes: extends R1's `printf` and `scanf`

## R4: pointers to string literals

- C: `char *` and `const char *` variables, parameters and return values
  whose values are string literals or null pointer constants such as `NULL`,
  `0` or `(char *)0`, uninitialised ones as null; `==` and `!=` against a
  null pointer constant and a pointer as a condition; `printf`'s `%s`
  without flags or a width
- Rust: `Option<&'static [u8]>`, `None` for null and otherwise the bytes of
  the literal's array through its final zero, `Some(b"...\x00".as_slice())`;
  the tests are `is_none()` and `is_some()`; `%s` is `{}` of
  `crt::c_str(...)`, a runtime part that takes the bytes before the first
  zero and panics on null, for which C has no string. The `crt` runtime now
  holds only the parts a translation calls. Literals that are not UTF-8,
  `%s` with flags or a width, which C counts in bytes and Rust in
  characters, and any other comparison of pointers stay unsupported; so does
  every write through a pointer, which leaves the literals read-only
- Motivation: `g2_011_uninit_char_ptr` and
  `g2_019_integer_overflow_char_max_multiply` print through a
  `printLine(const char *)` that tests its argument against `NULL`, and the
  first assigns a literal to an uninitialised `char *`
- Changes: extends R1's types and `printf`, and R2's conditions and the
  variables declared before a block with labels

## R5: pointers to integer variables

- C: pointers to the integer types, `const` or not, as variables and
  parameters; `&` of an integer variable, perhaps in parentheses; `*` of
  such a pointer as a value; null pointer constants, uninitialised pointers
  as null, `==` and `!=` against null and a pointer as a condition, as R4
  has them for `char *`
- Rust: `Option<&T>` for a `T *`, `Some(&x)` for `&x` and
  `*p.expect("null pointer dereference")` for `*p`, since C has no object
  at null; the tests are `is_none()` and `is_some()`. A pointer only reads:
  a write through it stays unsupported, and a write to the variable while a
  pointer borrows it fails to compile. A function returning such a pointer,
  whose borrow Rust cannot give a lifetime, `&` of anything but an integer
  variable, `&` of a `char`, whose `char *` R4 holds as a literal's bytes,
  and comparisons between two pointers stay unsupported
- Motivation: `g3_012_uninit_int_ptr` passes the address of a local `int`,
  through an `int *` variable, to a function with a `const int *` parameter
  that prints `*` of it
- Changes: extends R1's types and R4's null pointers and pointer conditions

## R6: integer division results

- C: `div`, `ldiv` and `lldiv`; variables, parameters and return values of
  type `div_t`, `ldiv_t` and `lldiv_t`, uninitialised ones as zero; reading
  their members `quot` and `rem` with `.`
- Rust: the pair `(quot, rem)`, `(i32, i32)` for `div_t` and `(i64, i64)`
  for the others, with `.0` and `.1` for the members; the calls are
  `crt::div`, a generic runtime part that returns `(numer / denom, numer %
  denom)`, which round towards zero as C does and panic for a zero divisor
  or a quotient out of range, which C leaves undefined. Writing a member,
  `->` and pointers to these types stay unsupported
- Motivation: `g1_022_stdlib_div` reads two integers, calls `div` and prints
  the quotient and remainder
- Changes: extends R1's types and calls

## R7: program arguments, strtol and static local variables

- C: `int main(int argc, char **argv)`, also with `char *argv[]`; `argv[i]`
  with an integer index; `strtol` and `strtoll` with a null end pointer or
  the address of a `char *` variable; `==` and `!=` between two `char *`
  when one is an element of `argv`; `static` local variables of the integer
  types, read, assigned, with `+=`, `-=`, `*=`, `/=` and `%=`, and with
  `++` and `--` as statements
- Rust: `argv` is `&'static [Option<&'static [u8]>]`, built once by
  `crt::args`, a runtime part that leaks each argument's bytes with a final
  zero and ends the slice with `None`; Rust's `main` passes `argc` as its
  length less one. `argv[i]` indexes the slice, a negative index converted
  `as usize` so that it panics as out of bounds. `crt::strtol` skips white
  space, reads a sign, a base-0 or `0x` prefix and digits as glibc does,
  saturates out of range and stores where the number ends, or the start of
  the string without one, through `Option<&mut Option<&[u8]>>`; `errno` is
  not set. The comparison tests the slices' start addresses with
  `.map(<[u8]>::as_ptr)`: an argument is never a string literal, which C
  may or may not merge, so other pointer comparisons stay unsupported. A
  static local is a Rust static `AtomicI8`…`AtomicU64` in the block that
  declares it, named `__NAME_N`, which C reserves, initialised once, to 0
  without an initialiser; reads are `load` and writes `store`, with
  `Relaxed` order, keeping R1's overflow checks and R2's wrapping. `&` of a
  static local, writing one through `scanf`, static locals of other types,
  `main` with other parameters and other uses of `char **` stay unsupported
- Motivation: `g1_005_static_loop` parses its argument with `strtol`, tests
  `end == argv[1]` and keeps a running total in a `static int`; the static
  locals of `g1_011_static_dag` and the `argv` of most programs that read
  their arguments failed as well
- Changes: extends R1's types, `main` and local variables, and R4's pointer
  comparisons

## R8: booleans

- C: `bool` and `_Bool` variables, parameters and return values,
  uninitialised ones as false; conversions to `bool` of an integer or a
  pointer, implicit or by a cast; a `bool` promoted to an integer type; a
  `bool` as a condition
- Rust: `bool`; a conversion to `bool` is the test of a condition, so `x !=
  0`, `p.is_some()` or a comparison itself, and a constant such as
  `<stdbool.h>`'s `true` and `false` is `true` or `false`; a promotion is
  `i32::from(b)` or the target type's `from`, and a promoted `bool` that C
  tests is the `bool` itself, with `!b` for `!`. `++`, `--` and compound
  assignments on a `bool`, static `bool` locals and pointers to `bool` stay
  unsupported
- Motivation: `g1_011_static_dag` passes `true` and `false` to a function
  with a `bool` parameter that it tests with `if`
- Changes: extends R1's types and conversions and R2's conditions

## R9: unsigned and bitwise arithmetic, switch, rand, strtoul and errno

- C: `+ - * / %` on the unsigned types and unary `-` on them; `& | ^ << >>`
  and `~` on every integer type; the compound assignments of all these
  operators, on local and static local variables; `c ? a : b`; `sizeof` of
  an integer, `bool` or pointer type or expression; `switch` on an integer
  whose case labels are integer or character constants, perhaps negated or
  converted, where no case with statements falls into the next; `errno`,
  read and assigned; `strtoul` and `strtoull`; `rand` and `srand`;
  `fprintf` to `stdout` or `stderr`; `*p` of a `char *`
- Rust: unsigned `+ - *` wrap with `wrapping_add`, `wrapping_sub` and
  `wrapping_mul`, as C defines them, and unary `-` with `wrapping_neg`; the
  other operators are Rust's, so overflow checks still catch what C leaves
  undefined: signed overflow, a zero divisor and a shift count outside the
  type; `~` is `!`. A compound assignment that wraps becomes `x =
  x.wrapping_add(v)`, the rest `x op= v`. `?:` is `if c { a } else { b }`.
  `sizeof` is a literal such as `4_u64` with x86-64 Linux's sizes. A
  `switch` is a `match` with an arm for each run of labels and `_` last,
  or `_ => {}` without `default`; a `break` that ends an arm is dropped, any
  other leaves a labelled block around the `match`, and `continue` still
  continues the loop around it. `errno` is `crt::ERRNO`, a static
  `AtomicI32`. `crt::strtol` and `crt::strtoul` share one parser; out of
  range they saturate and set `errno` to `ERANGE`, and `strtoul` negates a
  value after `-` modulo 2^64, as glibc does. `crt::rand` and `crt::srand`
  are glibc's `random`: 31 words seeded by its linear congruential
  generator, 310 results discarded and each result a sum of two words
  shifted right by one, seeded with 1 until `srand`. `fprintf` is
  `print!` or `eprint!` as R1's `printf`. `*p` is the first byte of the
  slice as `i8`, and panics past the end of the array. Fallthrough between
  cases, labels inside other statements, case ranges, case values that are
  not literal constants, a variable declared in one case and used in
  another and other streams stay unsupported
- Motivation: `g1_016_switch_arith` parses a seed with `strtoul` and
  `errno`, draws two numbers with `srand` and `rand`, combines them with
  every unsigned and bitwise operator and a `?:`, and prints one of ten
  messages from a `switch`; it prints its usage with `fprintf(stderr, ...)`
- Changes: extends R1's operators, compound assignments and `printf`, R2's
  control flow, R4's `char *` and R7's `strtol`, which now sets `errno`

## R10: integer arrays and pointers into them

- C: local arrays of the integer types other than `char`, of a fixed length,
  perhaps with an initialiser list, designators included, or of a variable
  length that is a variable's name; `a[i]` and `p[i]` with an integer index,
  read, assigned, with the compound assignments that R9 has for variables
  and with `++` and `--` as statements; an array as a pointer, `&a[i]` and `*p` as a value and as a
  target; `&` of an integer variable or parameter; `scanf` into `&a[i]`;
  `sizeof` of a fixed-length array; `restrict`, `const` and `volatile` on a
  pointer itself, as in `int *restrict`
- Rust: every integer the program points to is a `std::cell::Cell`, so
  pointers to the same object, as when one array is passed as the output and
  the inputs of one function, all read it with `get` and write it with `set`.
  A `T *` is `Option<&[Cell<T>]>`, the cells from the one it points to
  through the end of its array; `&x` is `Some(std::slice::from_ref(&x))`,
  `&a[i]` is `Some(&a[i..])`, and an array decays to `Some(&a[..])`. A
  fixed-length array is `&[Cell<T>]` borrowing `[const { Cell::new(0) };
  N]`, so a constant index out of bounds panics when it runs rather than
  failing to compile where it never runs; its initialiser sets the listed
  elements, the rest staying 0. A variable-length one is `vec![Cell::new(0);
  n as usize]`. `p[i]` indexes the slice of
  `p.expect("null pointer dereference")`, with a negative index converted
  `as usize` so it panics as out of bounds, and `*p` is element 0. A compound
  assignment or `++` on a cell finds the cell once, `{ let __cell = &...;
  __cell.set(...) }`, keeping R1's overflow checks and R9's wrapping. A
  variable whose address is taken is a cell from its declaration, a
  parameter from the start of its function; one whose address goes only to
  `scanf` stays a plain variable. Pointer arithmetic and comparisons between
  pointers, arrays of `char`, of arrays and of other types, arrays in a block
  with labels or static, a variable length given by an expression and
  returning a pointer stay unsupported
- Motivation: `g3_030_mutable_buffer_overlap_extrahard`,
  `g3_048_mutable_buffer_overlap2` and `g3_031_disjoint_arrays` read numbers
  into an `int[100]` with `scanf("%d", &data[i])` and pass it to a function
  that writes `out[i] = mul1[i] * mul2[i] + add[i]` through `int *`
  parameters, which the first points all at the same array, the second at a
  variable-length array and the third, as `int *restrict`, at three of them
- Changes: replaces R5's `Option<&T>`, a read-only borrow, with the slice of
  cells, so a program can now write through a pointer and to the variable it
  points to; extends R1's local variables, assignments and `scanf`, R2's
  variables declared before a block with labels and R9's `sizeof`

## R11: char arrays and the string functions that write them

- C: local arrays of `char` of a fixed length, perhaps initialised with a
  string literal or a list, or of a variable length that is a variable's
  name; `a[i]` of such an array with an integer index, read and assigned,
  and `p[i]` of a `char *`, read; such an array as a `char *` or `const char
  *`, passed to the program's own functions, `printf`'s `%s`, `atoi`,
  `strlen` or as a source; `fgets(a, n, stdin)`, `memset(a, c, n)`,
  `strcpy(a, src)` and `strncpy(a, src, n)` into such an array, their
  results discarded or, for `fgets`, compared with null; `atoi`, `atol`,
  `atoll` and `strlen` of any `char *`
- Rust: the array is `&mut [u8]` borrowing `[0; N]`, so a constant index out
  of bounds panics when it runs; a literal copies its bytes in, without the
  final zero when the array has no room for it, and a list sets the listed
  elements; a variable length is `&mut vec![0; n as usize]`. An element reads
  `a[i] as i8` and is assigned `a[i] = (v) as u8`, a `char *` element reads
  `p.expect("null pointer dereference")[i] as i8`, and the array as a
  pointer is `Some(&a[..])`. A `char *` variable is now `Option<&[u8]>`, so it
  can borrow a local array, and the `char *` parameters and return value of a
  function all borrow for one lifetime, `fn f<'a>(s: Option<&'a [u8]>) ->
  Option<&'a [u8]>`, so a function may return a parameter or a literal, but
  not its own array. The writing functions are `crt::fgets`, `crt::memset`,
  `crt::strcpy` and `crt::strncpy` on `&mut a[..]`: `fgets` reads standard
  input with `scanf`'s buffer, up to and including a newline and at most `n
  - 1` bytes, stores a zero after them, and returns `None`, leaving the array
  as it was, for `n` below 1 or input that ends before a byte, and otherwise
  `Some(())`; `n` of 1 stores only the zero, as glibc does. `crt::atoi` is
  `crt::strtol` in base 10 converted `as i32`, as glibc defines it, and
  `atol` and `atoll` are `crt::strtol`; R7's runtime now takes a string of any
  lifetime. Reading or writing past an array panics. Writing to the array
  while a `char *` borrows it fails to compile; so does using the result of
  `fgets` as a pointer. Writing through a `char *`, compound assignments and
  `++`/`--` on a `char` element, `char` arrays that are static, global or in
  a block with labels, other streams, and `memset` or `strcpy` into arrays of
  other types stay unsupported
- Motivation: `g2_009_stack_buffer_overflow` and
  `g2_017_signed_length_confusion` read a number with `fgets` into a
  `char[14]` initialised with `""` and `atoi`; the second also fills a
  `char[100]` with `memset`, terminates it by assigning `'\0'` to an
  element, copies it with `strncpy` and passes the array to a `const char *`
  parameter
- Changes: extends R4's `char *`, which held only literals and arguments with
  the `'static` lifetime, R7's `strtol` runtime and R10's local arrays, whose
  variable-length form is now borrowed as a slice too
