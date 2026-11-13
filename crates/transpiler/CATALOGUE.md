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
- Rust: `u8` for `char`, `i8` to `u64`, `bool`, `f32`, `f64`, a `const` for
  each enumerator used; Rust's operators for signed arithmetic, so that an
  overflow stops the program, and `wrapping_add`, `wrapping_sub` and
  `wrapping_mul` for unsigned; `as` for conversions, through `i8` when a
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
  the methods of `i32` and `f64` for the mathematics
- Needs: nothing
- Motivation: skeleton; `g1_div_euclid` and `g1_encode_quant` pass with R1 to
  R4
- Changes: none

## R5: records

- C: `struct` definitions, values of them, member access with `.` and `->`,
  initialiser lists, assignment of a whole record
- Rust: `struct` in CamelCase with `#[derive(Clone)]`, plus `Copy` when no
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
  the lifetime `'a`
- Needs: `Facts::pointer` for each slot; `Facts::stored_through` for `mut` on
  the binding of an owner or an `Option`
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
  other uses
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
  and borrowed as an array is. A length that is an expression, or a name
  that two variables of the function share, is refused: Clang prints the
  length only as text in the type
- Needs: the front end reads the length into `Var::len`
- Motivation: `g3_031_disjoint_arrays` and `g3_048_mutable_buffer_overlap2`
  were refused for an array whose length is not a constant
- Changes: extends R6

## R21: where a number ends

- C: `strtol` and `strtoll` in base 10 of text, with `&end` for a local
  `char *end`; `end == text` and `end != text`; `*end`, and `*end == '\0'`;
  `argv[i]` handed to a pointer that holds text
- Rust: `let mut end: &str = "";` and `parse_int_rest(&args[1], &mut end)`,
  a helper that sets `end` to the text after the number, or to all of the
  text when it starts with none; `end.as_ptr() == args[1].as_ptr()`, which
  is C's comparison of the two addresses; `end.is_empty()`, and
  `end.bytes().next().unwrap_or(0)` for the byte at `*end`; `&args[i]` for
  `argv[i]`, and `args.get(i).map(String::as_str)` for a pointer that may
  be NULL, which `argv[argc]` is. Refused: a test for NULL of `argv[i]` or
  of a pointer that holds it and nothing that is NULL; a comparison of two
  addresses of which one may be NULL, or which may both be arguments (two
  empty arguments are two places in C and one in Rust); an `end` that also
  holds NULL
- Needs: the analysis lets the text flow into `end`, counts `argv[i]` as a
  source of text (`Source::Argument`) and no longer counts `*p` as reading
  bytes; the fact `argument` for a slot that `argv[i]` reaches;
  `Facts::addressed`, for the first value of `end`
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
