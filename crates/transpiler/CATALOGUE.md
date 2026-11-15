# Rule catalogue

Every translation rule has an entry here, so the results can be traced to
rules. The loop gate rejects a change to `src/` that leaves this file
unchanged. [DESIGN.md](DESIGN.md) says what the rules share and where a new
one belongs.

R1 to R10 are the skeleton, written before the loop's first iteration. They
cover little on purpose: each construct without a rule is refused with its
name and line.

Entry format, newest last:

```markdown
## R11: short name

- C: the construct it translates, for example `printf` with a literal format
- Rust: what it emits for it
- Needs: the facts or rewrites it relies on, if any
- Motivation: the development programs that failed without it, and how
- Changes: earlier rules it replaces or extends, if any
```

## R1: functions, `main` and calls

- C: function definitions that are not variadic; calls of the program's own
  functions by name; `main` with no parameters or with `argc` and `argv`;
  `return`
- Rust: `fn` with the last `return` as the tail expression; `fn main()`, or
  `fn main() -> ExitCode` when some `return` that can run gives another value
  than the constant 0, with the low byte as the code; `argc` as
  `args.len() as i32` from `let args: Vec<String> = std::env::args().collect();`;
  `exit` as `std::process::exit`. Names that are Rust keywords or that the
  translation uses itself get a `_`
- Needs: `Facts::truth`, to leave out returns in branches that cannot run
- Motivation: skeleton
- Changes: none

## R2: numbers

- C: `char`, the integer types, `_Bool`, `float`, `double`, enumerators;
  literals and `sizeof`; `+ - * / % << >> & | ^ ~ !`, comparisons, `&&`,
  `||`, `?:`, the comma operator; conversions between numbers; `=`, the
  compound assignments, `++` and `--`; locals with and without an initialiser
- Rust: `u8` for `char`, `i8` to `u64`, `bool`, `f32`, `f64`, a `const` in
  upper case for each enumerator used, with a `_` where two enumerators of
  different functions share a name; Rust's operators for signed arithmetic, so that an
  overflow stops the program, and `wrapping_add`, `wrapping_sub` and
  `wrapping_mul` for unsigned; a floating-point literal as the shortest
  text that reads back as the same number, and as a quotient that is the
  same number, `314.0 / 1e2`, where the lints would take it for a constant
  such as `PI` written out;
  `as` for conversions, through `i8` when a
  `char` widens; a comparison as a value is `i32::from(…)`, a number as a
  test is `!= 0`; a compound assignment on a type narrower than `int` is
  computed as C does, in the wider type; an assignment, `++` or `--` used
  for its value is a block that ends in the value, and is refused when its
  target does more than name a place, since Rust would do that twice; a
  literal takes its suffix only where Rust cannot tell its type; `let`, with `mut` only when the variable
  changes, and zero for a missing initialiser
- Needs: `Facts::mutated`
- Motivation: skeleton
- Changes: none

## R3: control flow

- C: `if`/`else`, `while`, `do … while`, `for`, `break`, `continue`, and
  `switch` whose cases each end in `break`, `return` or `continue`
- Rust: `if`/`else if`/`else`; `while`, or `loop` for a test that is always
  true; `loop` ending in `if !test { break; }` for `do`; `for i in a..b`, or
  `a..=b` for a signed counter, for a `for` that counts a local up by one to
  a bound that its body cannot change, `_` for a counter the body does not read, and otherwise the
  initialiser and a `while` ending in the step; `match` with `|` for cases
  that share statements. A test that the facts decide is left out with the
  branch that cannot run
- Needs: `Facts::truth`
- Motivation: skeleton
- Changes: none

## R4: `printf` and numbers from the arguments

- C: `printf` with a literal format of text, `%%`, `%d %i %u %x %X %o` with
  `hh h l ll z`, `%f` with a precision (not `%F`, which prints an infinity
  in capitals), `%c`, and `%s` of a literal or of
  `argv[i]`, with the flags `-`, `0`, `+` and a width; `puts` of a literal;
  `putchar`; `atoi`, `atol`, `atoll`, and `strtol` or `strtoll` in base 10
  with a NULL end pointer, of `argv[i]`; `abs`, `sqrt`, `fabs`, `floor`,
  `ceil`, `pow`
- Rust: `print!`, or `println!` when the format ends in a newline, each value
  converted to the type its conversion reads; `parse_int(&args[i])`, a
  helper emitted once that reads a number as `strtoll` does (leading white
  space, a sign, digits, 0 without digits, the nearest limit out of range);
  the methods of `i32` and `f64` for the mathematics. The value that
  `printf`, `puts` and `putchar` return is refused
- Needs: nothing
- Motivation: skeleton; `g1_div_euclid` and `g1_encode_quant` pass with R1 to
  R4
- Changes: none

## R5: records

- C: `struct` definitions, values of them, member access with `.` and `->`,
  initialiser lists, assignment of a whole record
- Rust: `struct` in CamelCase with `#[derive(Clone)]`, its fields in snake
  case, with a `_` where two fields would share a name, plus `Copy` when no
  field owns memory and `Default` when the standard library derives it, else
  a written `impl Default`; `Name { field: value, … }` with zeros for members
  the list leaves out; `Name::default()` for a record without an initialiser
- Needs: the facts of each field that is a pointer (R7, R8)
- Motivation: skeleton
- Changes: none

## R6: arrays

- C: arrays of a constant length, indexing, initialiser lists, and `char`
  arrays initialised from a string literal
- Rust: `[T; N]`, `[zero; N]` or `std::array::from_fn` for one without an
  initialiser, `a[i as usize]`, `*b"…"` for text. An index that C widened to
  64 bits is converted to `usize` directly
- Needs: nothing
- Motivation: skeleton
- Changes: none

## R7: borrows

- C: a pointer that neither holds memory from `malloc` nor is freed: a
  parameter, a local or a result; `&x`, `&a[i]`, an array used as a pointer,
  `p + n`; `*p`, `p->field`, `p[i]`
- Rust: `&T` or `&[T]`, with `mut` when something is stored through the
  pointer or through any slot it is handed to; `&x`, `&mut a[i..]`,
  `std::slice::from_ref`; a `&mut` variable is reborrowed where it is handed
  on. A function that returns a borrow and takes several gives all of them
  the lifetime `'a`. Refused: a result that may point to a local or a
  parameter of its function, which ends where the function returns; and a
  call with two arguments that point into the same memory, such as
  `f(a, a + 1)`, when the function stores through one of them, since Rust
  lends memory once to whoever stores there
- Needs: `Facts::pointer` for each slot, with `local` for a pointer into a
  variable of its function; `Facts::stored_through` for `mut` on the
  binding of an owner or an `Option`
- Motivation: skeleton
- Changes: none

## R8: heap

- C: `malloc(sizeof(T))`, `malloc(n * sizeof(T))`, `calloc(n, sizeof(T))`,
  `free`; pointers that hold such memory in locals, parameters, results,
  fields and globals
- Rust: `Box::new(zero)` and `vec![zero; n]`, where the pointers in
  `calloc`'s memory count as NULL; `Box<T>` or `Vec<T>` for every
  slot that owns; a move between owners, `&mut *b` or `&mut v` from an owner
  to a borrow; `drop(p)` for `free(p)`. A field or a global gives up what it
  owns with `take()` or `std::mem::take`, since its holder stays in use. The
  NULL test after an allocation is left out, since allocation in Rust does
  not return NULL
- Needs: `Facts::pointer` (`owned`, `many`); `Facts::truth`
- Motivation: skeleton
- Changes: none

## R9: globals

- C: global variables and `static` locals
- Rust: `const`, or `static` for an array or a record, in upper case when the
  program never changes the variable; otherwise a field of `struct Globals`,
  which `main` creates and every function that uses a global, itself or
  through a call, takes as its first parameter: `globals: &Globals`, or
  `&mut Globals` when it changes one
- Needs: `Facts::mutated`, `Facts::access`
- Motivation: skeleton
- Changes: none

## R10: pointers that may be NULL

- C: a pointer that NULL may reach; `if (p)`, `if (p != NULL)`, and the guard
  `if (!p) return …;` (or `break`, `continue`, `exit`) for a local or a
  parameter whose address is not taken and that is not assigned where the
  test holds
- Rust: `Option<…>` around the slot's type, `None` for NULL, `Some(…)` for a
  value; `if let Some(p) = …` and `let Some(p) = … else { … };`, with `p`
  inside as the type without `Option`; `is_some()` and `is_none()` for a
  test with no such form; `as_deref()`, `as_deref_mut()` and `unwrap()` for
  other uses, and `.map(|found| &*found)` for a call's result that may be
  stored through and is kept to read, since `as_deref()` would borrow from
  a value that ends with its statement
- Needs: `Facts::pointer` (`nullable`); the rewrite `normalise::narrow`,
  which gives the tested pointer a variable of its own
- Motivation: skeleton
- Changes: none

## R11: `scanf` of numbers and characters

- C: `scanf`, and `fscanf` on `stdin`, with a literal format of white space
  and `%d`, `%u` with `hh` or `h`, `%ld`, `%lld` and `%c`, each into a
  variable of the conversion's size; the call as a statement, or its result
  compared with its number of conversions. A pointer whose expression
  assigns or calls is refused, and so is a pointer after the first with an
  index that is computed: C finds every pointer before it reads, and the
  translation each one when it stores
- Rust: `if let Some(scanned) = scan_int() { x = scanned as i32; }`, one
  inside the other for several conversions, since `scanf` stops at the first
  that finds nothing; `let Some(scanned) = scan_int() else { … };` for
  `if (scanf(…) != 1)` that leaves the block; `read_byte()` for `%c` and
  `skip_space()` for white space in the format that no number follows. The
  helpers read `std::io::stdin().lock()` a byte at a time and leave what a
  conversion does not take, as C does
- Needs: nothing
- Motivation: `g1_040_storage_class_auto`, `g1_010_integer_overflow` and
  `g3_024_struct_and_static` were refused for `scanf` or `fscanf`; seven more
  development programs read their input this way
- Changes: none

## R12: a pointer that is never assigned

- C: a local pointer declared without a value that no statement assigns
- Rust: `None`, as for a pointer that holds NULL: reading it is undefined
  in C, so any value is right, and this is the one Rust can write. What the
  pointer is handed to is an `Option` too, and stops the program where C
  would read through it
- Needs: `Facts::mutated`; the analysis gives such a variable a flow from
  NULL
- Motivation: `g3_012_uninit_int_ptr` did not compile: `let data: &i32;`
  was read without a value. `g2_011_uninit_char_ptr` has the same shape
- Changes: extends R10

## R13: text

- C: a pointer to `char` that only ever holds string literals, in a local,
  a parameter or a result, and is only handed on, tested against NULL or
  read by `printf`'s `%s`, `puts`, `strlen`, `atoi`, `atol` or `atoll`
- Rust: `&str`, or `Option<&str>` where NULL may reach it, and `&'static
  str` for a result; the literal as written up to its first NUL, where its
  readers stop; `println!("{}", text)` for `puts` and `%s`,
  `text.len() as u64` for `strlen`, `parse_int(text)` for `atoi`
- Needs: `Facts::pointer` (`bytes`): a pointer that is indexed, moved,
  dereferenced, stored through, handed to another library function or that
  meets one such pointer or the address of a variable holds bytes, and has
  no rule for a literal yet
- Motivation: `g2_011_uninit_char_ptr` and
  `g2_019_integer_overflow_char_max_multiply` were refused for text other
  than a program argument
- Changes: extends R4, whose `%s` took a literal or `argv[i]`, and R7

## R14: an initialiser of zeros

- C: `{0}`, `{}` or any list of zeros for an array, of numbers or of records
- Rust: the array of zeros, `[0; 40]`, whatever its length
- Needs: nothing
- Motivation: `g2_009_stack_buffer_overflow` was refused for
  `int buffer[10] = { 0 }`, an array whose initialiser leaves much of it out
- Changes: extends R6, which wrote out up to eight missing elements

## R15: text in bytes

- C: text in a `char` array or behind a pointer to one, read by `printf`'s
  `%s`, `puts`, `strlen`, `atoi`, `atol` or `atoll`; a string literal for a
  pointer that the program also reads byte by byte or that meets an array
- Rust: the array stays `[u8; N]` and the pointer `&[u8]`; `text_of(&word)`
  to print or parse it and `until_nul(&word).len()` for its length, two
  helpers that read up to the first NUL and stop the program where there is
  none, since C would read on beyond the array; `b"literal\0"` for the
  literal
- Needs: `Facts::pointer` (`bytes`), which tells this text from R13's
- Motivation: `g2_009_stack_buffer_overflow`, `g2_016_divide_by_zero_float`
  and `g2_017_signed_length_confusion` parse a line that they read into a
  `char` array
- Changes: extends R4 and R13; a literal for a pointer to bytes was refused

## R16: `fgets`

- C: `fgets(buffer, size, stdin)` as a statement or tested against NULL
- Rust: `read_line(&mut buffer)`, or `read_line(&mut buffer[..size])` when
  the size is not the array's length: a helper that reads up to the end of
  the line or until one byte is left, stores a NUL after it and returns
  whether there was anything to read, which is the test against NULL.
  `read_line_within(&mut buffer, size)` where the size is not a constant
  that the array is known to hold: `fgets` is right with a size beyond its
  buffer as long as the line fits, so that helper reads into as much of the
  buffer as there is
- Needs: the helper `read_byte` of R11, so that `scanf` and `fgets` share
  what is left of the input
- Motivation: `g2_009_stack_buffer_overflow`, `g2_016_divide_by_zero_float`,
  `g2_017_signed_length_confusion` and `g3_023_struct_and_errno` read their
  input with it
- Changes: none

## R17: `strtoul` and `strtoull`

- C: `strtoul` and `strtoull` in base 10 with a NULL end pointer, of text
- Rust: `parse_uint(&args[1])`, a helper that reads as `strtoull` does:
  white space, a sign, digits, 0 without digits, the greatest value when
  out of range, and counted down from zero after a `-`
- Needs: nothing
- Motivation: `g1_rev16` was refused for `strtoull`, which the benchmark's
  own `main` reads each unsigned argument with
- Changes: extends R4; R13's analysis counts the `strtol` family among the
  functions that read text as a whole

## R18: `strtod`, `strtof` and `atof`

- C: `strtod` and `strtof` with a NULL end pointer, and `atof`, of text
- Rust: `parse_float::<f64>(&args[1])` or `::<f32>`, a helper that finds
  what `strtod` reads as a decimal number (white space, a sign, digits with
  a point and an exponent, `inf`, `nan`) and parses it with the standard
  library, which rounds as C does; 0 without a number
- Needs: nothing
- Motivation: `g2_016_divide_by_zero_float` was refused for `atof`, and
  `g2_ldexp_q2` and `g2_pow43` for `strtof`, which the benchmark's own
  `main` reads each `float` argument with
- Changes: extends R4

## R19: `%e` and `%g`

- C: `printf`'s `%e` and `%g` with a precision, a width and `-`
- Rust: `format_e(value, 6)` and `format_g(value, 6)` as the value of `{}`,
  two helpers that write the exponent with a sign and two digits and, for
  `%g`, choose between the two forms and drop the zeros at the end as C
  does; the standard library has neither form
- Needs: nothing
- Motivation: `g2_ldexp_q2` and `g2_pow43` were refused for `%.9g`, which
  the benchmark's own `main` prints each `float` result with
- Changes: extends R4

## R20: arrays of a computed length

- C: a local array whose length is one variable, `int out[len]`
- Rust: `let mut out: Vec<i32> = vec![0; len as usize];`, which is indexed
  and borrowed as an array is. Clang prints the length only as text in the
  type, so it is the variable of that name in scope where the array is
  declared; a length that is an expression is refused
- Needs: the front end reads the length into `Var::len`
- Motivation: `g3_031_disjoint_arrays` and `g3_048_mutable_buffer_overlap2`
  were refused for an array whose length is not a constant
- Changes: extends R6

## R21: where a number ends

- C: `strtol` and `strtoll` in base 10 of text, with `&end` for a local
  `char *end`, as a statement, a declaration's value or what an assignment
  stores; `end == text` and `end != text`, `end - text`; `*end`, and
  `*end == '\0'`; `end` as text, and as the text of the next call;
  `argv[i]` handed to a pointer that holds text
- Rust: `let (n, end) = parse_int_prefix(&args[1]);`, a helper that also
  returns the index where the number ends, 0 when the text starts with
  none; each call declares an `end` of its own. The end is a cursor (R24)
  on the text that was read: `end == 0`, `end == args[1].len()` for
  `*end == '\0'`, `args[1].as_bytes().get(end).copied().unwrap_or(0)` for
  the byte at `*end`, `&args[1][end..]` for the text from there, and
  `buf[end]` in an array of bytes. A call that reads from an index gives
  `let (n, read) = parse_int_prefix(&text[end..]); let end = end + read;`.
  A call whose end nothing reads is `parse_int`. `&args[i]` for `argv[i]`,
  and `args.get(i).map(String::as_str)` for a pointer that may be NULL,
  which `argv[argc]` is. Refused: an end that is read where not one call
  is the last before it on every path (before the first call, after an
  `if` or a loop that holds a call); a call inside a larger expression; a
  text that is no array, no pointer that keeps its value and no
  `argv[constant]`; text from an index that is not such an end; a test for
  NULL of `argv[i]` or of a pointer that holds it and nothing that is
  NULL; a comparison of two addresses of which one may be NULL, or which
  may both be in arguments (two empty arguments are two places in C and
  one in Rust)
- Needs: `normalise::ends`, which gives each call its own variable for the
  end and makes the call the statement `Stmt::Parse`; `cursors` (R24),
  which makes that variable an index and keeps a base that only such ends
  index as text, `text + index` in the tree; the analysis counts `argv[i]`
  as a source of text (`Source::Argument`) and does not count `*p` or an
  offset by an index as reading bytes; the fact `argument` for a slot that
  `argv[i]` reaches
- Motivation: `g1_005_static_loop` and `g1_011_static_dag` were refused for
  `strtol` with an end pointer, which they compare with the argument to see
  whether it held a number
- Changes: extends R4 and R13

## R22: `memset` and `strncpy`

- C: `memset` of bytes with any value, of numbers with zero for a constant
  size, and of one variable with zero for its own size; `strncpy` into bytes
- Rust: `source[..19].fill(b'A')`, `numbers.fill(0)` when the size is the
  whole array's, `point = Point::default()`;
  `copy_text(&mut target[..n as usize], &source)`, a helper that copies the
  text of `source` and fills the rest of the slice with NUL bytes, or cuts
  the text short without one, as `strncpy` does
- Needs: nothing
- Motivation: `g2_017_signed_length_confusion` was refused for `memset`,
  and copies with `strncpy`
- Changes: none

## R23: bit-fields

- C: a bit-field of an integer type or of `_Bool`, read, assigned and
  given a value by an initialiser
- Rust: a field of the declared type; a store keeps what the bit-field
  holds, `flags.level = value & 7` for three unsigned bits and
  `value << 28 >> 28` for four signed ones, so that a read needs nothing;
  a number is stored as the bits it leaves, `flags.level = -1` for 7.
  `+=`, `++` and the like on a bit-field are refused, and so is the size of
  its record
- Needs: nothing
- Motivation: `g3_033_bitfield` was refused for the bit-field `x`
- Changes: extends R5

## R24: cursors

- C: a local pointer or a parameter that walks one array: `p++`, `p--`,
  `p += n`, `p -= n`, `p = q + n`; `*p`, `p[i]`, `p->field`; `p < end`,
  `p == q` and `p - q` for two pointers into that array; `p` handed on,
  returned or tested for NULL
- Rust: the slice stays where it is and the pointer is an index beside it:
  `let mut p: usize = 0;`, `a[p]`, `a[p + i]`, `a[p].field`, `p += 1`,
  `p < end`, `p as i64 - q as i64`, `&a[p..]` where the pointer itself is
  needed, also where the slot it goes to is one element, since the cursor
  may be at the end of the array, and the test of `a` for a test of `p`. A parameter that
  the function moves stays the slice and gets a local `s_at`. A signed
  offset that is no constant is added with `wrapping_add_signed`. Left as
  they are, and so refused: pointers that walk no single array, whose
  array is named by an expression that may come to mean other memory, or
  whose address is taken; a comparison of pointers into two arrays
- Needs: the rewrite `normalise::cursors` and `Type::Index`; the analysis
  counts text that a library function reads as an array
- Motivation: `g3_maxnmin` was refused for `++` on a pointer, `g3_jumpnode`
  and `g4_dataentry` for a comparison of two addresses
- Changes: extends R7; `p = p + 1` was the rest of the slice, `&p[1..]`

## R25: a number compared with itself

- C: `x != x` and `x == x` for a `float` or `double` that is read twice
  in the same way without an effect: a variable, a field, an element
- Rust: `x.is_nan()` and `!x.is_nan()`, which is what the comparison
  asks; the compiler's lints refuse `x != x`. Refused, since the same
  lints refuse them: any other comparison of a value with itself, and the
  same value on both sides of `-`, `/`, `&`, `|` or `^`
- Needs: nothing
- Motivation: `g3_maxnmin` tests `d != d` before it converts `d` to `int`,
  and did not compile
- Changes: extends R2

## R26: `fprintf`, `fputs` and `fputc` on `stdout` and `stderr`

- C: `fprintf` with a literal format, `fputs`, `fputc` and `putc`, each on
  `stdout` or `stderr`, as statements; another stream is refused
- Rust: `print!` and `println!` on `stdout`, `eprint!` and `eprintln!` on
  `stderr`, with the conversions of R4 and R19 for `fprintf`, the text as
  `puts` writes it for `fputs` (`println!` for a literal that ends the
  line), and the character as `putchar` writes it for `fputc`. Rust's
  `stdout` is buffered by line where C's is buffered fully when it is no
  terminal, so the two programs may interleave `stdout` and `stderr`
  differently, which only the output on `stderr` shows
- Needs: R13's analysis counts `fprintf` and `fputs` among the functions
  that read text as a whole
- Motivation: `g1_007_errno_pow`, `g1_016_switch_arith` and
  `g4_memcpy_fun_buffers` were refused for `fprintf(stderr, …)`, which
  `g3_envy` uses as well
- Changes: extends R4

## R27: `sprintf` and `snprintf` into bytes

- C: `sprintf` and `snprintf` with a literal format into a `char` array, a
  pointer into one, a field or `buffer + 2`, as a statement or for the
  value they return; `snprintf` with a constant size or one in a variable
- Rust: `store_text(&format!("Entry_{}", id), &mut buffer)`, a helper that
  stores the text and a NUL and stops the program where they do not fit;
  `store_text_cut(…, &mut buffer)` for `snprintf` with the array's length,
  and `&mut buffer[..4]` for a smaller constant size, which stores what
  leaves room for the NUL; `store_text_within(…, &mut *out, size)` for any
  other size, which stops the program only where C would write beyond the
  buffer. Each returns C's result as an `i32`. The text comes first, so its
  values may read the buffer. The format is R4's; refused, since the text
  passes through a `String`: `%s` of bytes, which `text_of` would change
  where they are not UTF-8, and `%c` of a value that may be above 127; also
  a constant size beyond the array
- Needs: R13's analysis reads every argument of `sprintf` and `snprintf`
  but the first as text, and takes every `char` pointer that a library
  function is handed for an array (`many`), text or bytes
- Motivation: `g3_jumpnode`, `g4_dataentry`, `g5_findrep`, `g5_hm_geti`,
  `g5_intput`, `g5_sh_puts` and `g5_str_put` call `sprintf`, and `g3_envy`,
  `g5_cleanup` and `g5_memchra2` call `snprintf`; each is refused earlier
  for another construct
- Changes: extends R4

## R28: `strcpy`, `strcat` and `strncat`

- C: `strcpy`, `strcat` and `strncat` into bytes, as statements
- Rust: `copy_string(&mut name, &source)`, a helper that copies the text
  of `source` and its NUL to the start of the target;
  `append_string(&mut line, b"text\0")`, which copies them to the NUL of
  the text in the target; `append_at_most(&mut line, &word, n as usize)`,
  which copies at most `n` bytes of the text and a NUL after them, and
  needs no NUL in the source within them. The target is all the memory
  the pointer reaches, `&mut line[3..]` for `line + 3`, so a target that
  is too short, or text without a NUL, stops the program where C's
  behaviour is undefined. The pointer they return is refused where it is
  used, and so is a copy from one part of an array to another, which
  borrows the array twice
- Needs: nothing
- Motivation: `g4_betagamma` was refused for `strcpy`, which
  `g3_strcmp`, `g4_dataentry`, `g5_charinbuf` and `g5_findrep` use too
- Changes: none

## R29: `memcpy` and `memmove`

- C: `memcpy` and `memmove` as statements, between pointers to one element
  type that assignment copies, of a size that is any expression for
  elements of one byte, and otherwise a constant that is a multiple of the
  element's size or `count * sizeof(element)`
- Rust: `b[1..][..2].copy_from_slice(&a[3..][..2])`, with the whole array
  where the count is its length, `b.copy_from_slice(&a)`, and
  `&b"text\0"[..3]` for a string literal; `backup = now` and
  `c.data[i as usize] = s[0]` for one element where a side points to one;
  `s.copy_within(3..3 + n as usize, 0)` where both sides are in the same
  variable or field, which is right where they overlap, for `memcpy` as
  for `memmove`. A count beyond either side stops the program where C's
  behaviour is undefined. Refused: a copy between two types, which would
  reinterpret memory, of a type with pointers, of part of an element, of a
  size that is no count of elements or that calls or assigns, since it is
  written for each side, and from an offset with effects in one array. Two pointers into one array that the rule cannot tell apart
  are two borrows, which `rustc` refuses
- Needs: nothing
- Motivation: no development program was refused for them, but
  `g3_envy` copies a record with `memcpy`, `g4_memcpy_fun_buffers` copies
  bytes between and within buffers, and `g5_str_put`, `g5_intput`,
  `g5_hm_geti` and `g5_sh_puts` move elements with `memmove`
- Changes: none

## R30: `strcmp` and `strncmp`

- C: `strcmp(a, b)` and `strncmp(a, b, n)` where the program reads only the
  sign of the result or whether it is zero: compared with 0 by any of the
  six operators, on either side, negated with `!` or used as a test by
  itself, in `if`, `while`, `&&`, `||` and `?:`
- Rust: a comparison of the two texts with the test's operator, since Rust
  orders `&str` and `&[u8]` as unsigned bytes, as C does: `mode == "fast"`
  and `args[1].as_str() < args[2].as_str()` when both sides are `&str`;
  otherwise the bytes without the NUL, `until_nul(&word) == b"fast"`,
  `text.as_bytes()`, `args[1].as_bytes()`, and `b"fat".as_slice()` for an
  ordering; `text.is_empty()` against `""`. `strncmp` compares
  `text_within(&word, n as usize)`, a helper that gives the bytes before
  the first NUL among the first `n`, or all `n`, or the whole slice where
  it ends before either (C reads beyond it there unless the texts have
  differed by then), with `first_bytes(text.as_bytes(), n)` for
  a `&str`, an argument or a literal, which end where they end. Two literals,
  a side compared with itself and a count of 0 give `true` or `false` here.
  Refused: any other use of the value, which C fixes only in its sign and
  C libraries give differently, and a count of `strncmp` that calls or
  assigns, since it is read for each side
- Needs: `strcmp` and `strncmp` among the readers of text as a whole
  (`TEXT_READERS`), so that a pointer they read stays a `&str`
- Motivation: `g5_cleanup` was refused for `strncmp`; `g3_strcmp`,
  `g5_hm_geti`, `g5_intput`, `g5_memchra2`, `g5_sh_puts` and `g5_str_put`
  call `strcmp` or `strncmp` and are refused first for other constructs
- Changes: extends R13 and R15

## R31: `memcmp`

- C: `memcmp(a, b, size)` tested against 0 as R30's functions are, of two
  arrays of one integer type once the conversions to `void *` are taken off,
  with a size of whole elements: any expression for bytes, otherwise a
  constant multiple of the element's size or `count * sizeof(element)`
- Rust: a comparison of the first elements with the test's operator,
  `a[..n as usize] < b[..n as usize]`, `a == b` for two whole arrays,
  `x == y` for one element each, and `b"hel\0"[..3]` for a string literal,
  whose NUL C compares too. Slices of `u8` order as `memcmp` does; other
  integers only compare for equality, so `<` and the other orderings of
  them are refused, since `memcmp` orders their bytes, not their values.
  Refused as well: floating-point numbers and records, where `memcmp`
  compares padding, tells `0.0` from `-0.0` and finds a NaN equal to itself;
  two different element types; a size that is no whole number of elements
  or that calls or assigns, since it is written for each side;
  and any other use of the value
- Needs: nothing
- Motivation: `g5_hm_geti`, `g5_intput`, `g5_sh_puts` and `g5_str_put`
  call `memcmp` and are refused first for other constructs
- Changes: extends R30

## R32: a `break` in the middle of a case

- C: `break` inside an `if` or a block of a `case`, which leaves the
  `switch` from there
- Rust: a block with a label around the `match`, `'switch: { match … }`,
  and `break 'switch;`; `'switch_2` for a `switch` inside a case of
  another. A `switch` without such a `break` stays a bare `match`. Refused:
  a `continue` in such a `switch`, which Rust wants to name its loop, and
  loops have no labels
- Needs: nothing
- Motivation: `g4_dataentry` was refused for a `break` that leaves a case
  early when an allocation failed
- Changes: extends R3

## R33: a union that holds the bits of a number

- C: a `union` whose members are all integers and floating-point numbers
  of one size, such as `union { float flt; uint32_t num; }`, with or
  without a tag, as a local, a global, a parameter, a result, a field, an
  array element or behind a pointer; a store into a member and a read of
  one
- Rust: the unsigned integer of that size, with no type for the union.
  `u.flt = x` is `u = x.to_bits()` (`f32::to_bits(…)` for a value whose
  type a method call would not know), `u.i = k` is `u = k as u32`, and a
  store into the unsigned member is a plain store; `u.flt` is
  `f32::from_bits(u)`, `u.i` is `u as i32` and `u.num` is `u`, which is
  also a place, so `u.num |= 1` and `&u.num` work on it. The first store
  into a local union declares it (R1)
- Needs: nothing; `normalise::unions` rewrites the tree before every other
  stage
- Refused: a union with members of different sizes or with a member that
  is no number; `+=`, `++` and `&` on a member other than the unsigned
  one; a store into such a member whose value is used; the initialiser of
  a union
- Motivation: `g5_float2half` and `g5_half2float` convert between `float`
  and its bits through a union, as do `g5_memchra2` and `g5_agglom`, which
  are refused next for other constructs
- Changes: nothing

## R36: where `strtoul`'s and `strtod`'s numbers end

- C: `strtoul` and `strtoull` in base 10, `strtod` and `strtof`, with
  `&end` as R21 has it for `strtol`; a first value of the end without an
  effect, `char *end = text;` or `end = NULL;` as a statement, that nothing
  reads before the next call
- Rust: `let (n, end) = parse_uint_prefix(&args[1]);` and
  `let (d, end) = parse_float_prefix::<f64>(&args[1]);`, helpers that also
  return the index where the number ends, 0 when the text starts with
  none; `parse_float_prefix` reads `nan(chars)` to its `)`, and
  `parse_float` is its first result. The store of the first value is
  dropped. Refused: a read of the end before the first call or after such
  a store, a store of it inside a larger expression, another base
- Needs: `normalise::ends`, which takes these calls as it takes `strtol`
- Motivation: `g1_007_errno_pow` and `g1_016_switch_arith` read where
  `strtod` and `strtoul` stopped, and `g3_023_struct_and_errno` gives the end
  the text as its first value
- Changes: extends R17, R18 and R21

## R37: `errno` of a number that is read

- C: `errno = 0;`, then a call of `strtol`, `strtoll`, `strtoul`,
  `strtoull`, `strtod` or `strtof` with `&end` as R21 and R36 take it,
  then reads of `errno`: `errno == ERANGE`, `errno != 0`, `errno == 0`,
  `!errno`, `errno` as a test, `errno == EDOM` and the like, in the
  statements after the call in its block and in the branches of an `if`
  there, up to the next call that may set it
- Rust: `let (n, end, range) = parse_int_checked(&args[1]);`, and
  `parse_uint_checked`, `parse_float_checked::<f64>(text, f64::MIN_POSITIVE)`,
  helpers that also return whether glibc sets `ERANGE`: an integer beyond
  `i64`, digits beyond `u64` (a `-` before them is no error), a `double`
  that is infinite from digits or, with a digit that is not 0, below the
  least normal number of its type. The reads are `range`, `!range` and
  `false` for any other number, the store is dropped, and a call whose
  `errno` nothing reads keeps two results. Refused, with `a read of `errno`
  that is not one call's error`: a read without the store before the call
  in its block, after a call between that may set `errno` (a function of
  the program, any library function but the printers and those of
  `string.h` and the absolute values), after a loop or a branch that holds
  one, after other calls such as `atoi` or `malloc`. Refused too: `errno`
  as a number (`printf("%d", errno)`, `int e = errno;`), any other store,
  and `&errno`
- Needs: `normalise::errno`, run at the end of `ends`, which gives the
  `Stmt::Parse` a variable for the error and rewrites the reads it reaches
- Motivation: `g1_016_switch_arith` and `g3_023_struct_and_errno` test
  `errno` after `strtoul` and `strtol`, and `g1_007_errno_pow` after
  `strtod`
- Changes: extends R21 and R36
