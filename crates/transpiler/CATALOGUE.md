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
