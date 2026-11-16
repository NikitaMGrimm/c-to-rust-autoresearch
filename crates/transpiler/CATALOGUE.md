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
  text that reads back as the same number, the same `f32` for a `float`,
  and as a quotient that is the same number, `314.0 / 1e2`, where the
  lints would take it for a constant such as `PI` written out; the least
  value of a signed type, `-2147483647 - 1`, as `i32::MIN`;
  `as` for conversions, through `i8` when a
  `char` widens; a comparison as a value is `i32::from(…)`, a number as a
  test is `!= 0`; a compound assignment on a type narrower than `int` is
  computed as C does, in the wider type; an assignment, `++` or `--` used
  for its value is a block that ends in the value, and is refused when its
  target does more than name a place, since Rust would do that twice; a
  literal takes its suffix only where Rust cannot tell its type; `let`, with `mut` only when the variable
  changes, and zero for a missing initialiser. An enumerator is a
  constant of type `i32`; one whose value is beyond `int`, which GNU C
  allows, is refused
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
  branch that cannot run: also a constant that is no literal, `if (2 -
  2)`, and `c ? a : b` with a constant `c` and a constant on that side. A `while` whose body leaves it the first time
  round, with a `break` or a `return` as one of its statements and no
  `continue` before it, is an `if`, since the compiler's lints refuse a
  loop that never runs twice; refused as such a loop are a `do` and a
  `for` of that shape, and a `while` with another `break` before that one
  or with a label
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
  64 bits is converted to `usize` directly. Refused: a constant index
  that the array does not have, `a[4]` for `int a[4]`, also where the
  statement never runs, since the compiler refuses it; `&a[4]`, the end
  of the array, is no such index
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
  lends memory once to whoever stores there, unless R53 gives the call a
  copy of the function; and a local pointer to a variable of its function
  that is used after the variable itself was stored to since the pointer
  got its address, or read where the pointer is stored through, since the
  borrow is still in use there; and in the same way a copy of a local
  pointer that owns or is stored through, `q = p`, that is read after `p`
  was stored through or assigned
- Needs: `Facts::pointer` for each slot, with `local` for a pointer into a
  variable of its function; `Facts::in_use` for the first such use, in the
  order of the statements, where a loop's body follows itself; `Facts::stored_through` for `mut` on the
  binding of an owner or an `Option`. A store through `&s->items[i]`, an
  element behind a pointer that a record holds, is a store through the
  pointer to the record as well, which Rust reaches the element through
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
  not return NULL. Refused: an owner, a local or a global, that is read
  after it gave its memory to another owner (a field, a global, another
  local or a parameter) and before anything stored to it, since Rust moved
  it there, or took it and left `None` in the global; and a function
  that the program names and that returns an owner out of a record it
  only borrows, `return p->next;`, which would leave `None` in the record
- Needs: `Facts::pointer` (`owned`, `many`); `Facts::truth`;
  `Facts::moved` for the first such read, in the order of the statements,
  where a loop's body follows itself
- Motivation: skeleton
- Changes: none

## R9: globals

- C: global variables and `static` locals
- Rust: `const`, or `static` for an array or a record, in upper case when the
  program never changes the variable; otherwise a field of `struct Globals`,
  which `main` creates and every function that uses a global, itself or
  through a call, takes as its first parameter: `globals: &Globals`, or
  `&mut Globals` when it changes one. A `static` may call no function,
  so the zero of a record in one is written out, `Pair { left: 0, right:
  0 }`; a global that never changes and whose zero only a function gives
  (an array of records that are not `Copy`, a field that owns one value)
  is refused. Refused: a call that hands such a
  function a pointer into a global that changes, unless the function only
  reads the globals and nothing is stored through the pointer, since the
  call borrows `Globals` as a whole beside it
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
  a value that ends with its statement. A borrow that may be NULL and is
  stored through walks on through itself, `p = p->next`, as `p =
  p.take().unwrap().next.as_deref_mut();`: a reborrow of `p` would still
  be in use where `p` is assigned
- Needs: `Facts::pointer` (`nullable`); the rewrite `normalise::narrow`,
  which gives the tested pointer a variable of its own
- Motivation: skeleton
- Changes: none

## R11: `scanf` of numbers and characters

- C: `scanf`, and `fscanf` on `stdin`, with a literal format of white space
  and `%d`, `%u` with `hh` or `h`, `%ld`, `%lld` and `%c`, each into a
  variable of the conversion's size; the call as a statement, its result
  compared with its number of conversions, and its result as any other
  value. A pointer whose expression
  assigns or calls is refused, and so is a pointer after the first with an
  index that is computed: C finds every pointer before it reads, and the
  translation each one when it stores
- Rust: `if let Some(scanned) = scan_int() { x = scanned as i32; }`, one
  inside the other for several conversions, since `scanf` stops at the first
  that finds nothing; `let Some(scanned) = scan_int() else { … };` for
  `if (scanf(…) != 1)` that leaves the block; `read_byte()` for `%c` and
  `skip_space()` for white space in the format that no number follows. The
  helpers read `std::io::stdin().lock()` a byte at a time and leave what a
  conversion does not take, as C does. The result as another value is the
  count: a `match` on `scan_int_or_end()`, whose `Err(ScanError::End)` is
  -1 and `Err(ScanError::NoMatch)` is 0, or `read_byte()` with -1, for the
  first conversion, and `if let Some(…) … else { k }` for each later one,
  with the `k` conversions stored before it
- Needs: nothing
- Motivation: `g1_040_storage_class_auto`, `g1_010_integer_overflow` and
  `g3_024_struct_and_static` were refused for `scanf` or `fscanf`; seven more
  development programs read their input this way. No development program
  needs the count as another value; it was refused, as in `int n =
  scanf(…)` and `scanf(…) != EOF`
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
  what `strtod` reads (white space, a sign, digits with a point and an
  exponent, the same in hexadecimal after `0x` with an exponent `p` of two,
  `inf`, `nan`) and parses it with the standard library, which rounds as C
  does; a hexadecimal number as its exact decimal text (`hex_float`,
  `exact_decimal`), without the bit after the type's digits where it is
  below the least normal number, which glibc drops there; 0 without a
  number
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
  whose address is taken; a comparison of pointers into two arrays.
  Refused: a function whose statements step a cursor before the start of
  its array, where the index is known to be 0, or test whether it is
  before the start (`p >= a`), which compiled C runs through and an index
  of Rust does not
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
  same value on both sides of `-`, `/`, `&`, `|` or `^`, and `x & 0`,
  `0 * x` and `0 / x`, which are 0 whatever `x` is; an unsigned `*`, a
  method, and a 0 that is an enumerator are no such case. Two constants
  are no such case: a comparison of two constants is its value as C's
  conversions give it, `true` or `false` in a test, which then goes with
  its dead branch, and `1` or `0` as a number; `4 - 4` and `8 / 8` are the
  number. The same test on both sides of `||` or `&&` is that test once.
  A comparison that the type of one side decides against a constant, as
  `u < 0` and `u >= 0` for an unsigned `u` or `c > 300` for an `unsigned
  char`, is decided in the same way, and refused where the other side
  has an effect; one that leaves a single value, `n >= INT_MAX` or
  `u <= 0`, is `==`. The lints refuse each of these as written
- Needs: `Facts::truth`, which decides a comparison of constants
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
  leaves room for the NUL; `store_text_within(…, out, size)` for any
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
  element's size or `count * sizeof(element)`; `sizeof` of the element's
  own type is one element also for a record with bit-fields, whose size the
  translation does not compute
- Rust: `b[i..][..2].copy_from_slice(&a[3..5])`, with one range where the
  offset and the count are constants and the whole array
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
  none; `parse_float_prefix` reads `nan(chars)` to its `)` and keeps a
  number among the chars in the NaN's lowest bits, as glibc does (`Float`,
  a trait of `f32` and `f64`), and `parse_float` is its first result. The store of the first value is
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
  `parse_uint_checked`, `parse_float_checked::<f64>(text)`,
  helpers that also return whether glibc sets `ERANGE`: an integer beyond
  `i64`, digits beyond `u64` (a `-` before them is no error), a `double`
  that is infinite from digits, or one that is inexact and whose text is
  below the least normal number of its type after rounding to the type's
  digits with no bound on the exponent; both found by comparing exact
  decimal digits. The reads are `range`, `!range` and
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
## R39: `getenv`

- C: `getenv(name)` for a `name` that is text, bytes or a program argument;
  its result kept in a pointer, tested for NULL, read as text by `atoi`,
  `strlen`, `strcmp`, `printf`'s `%s` and the other readers of R13, its
  first byte `*v`, and a string literal stored in the same pointer
- Rust: `std::env::var(name).ok()`, an `Option<String>` that the slot owns,
  with `&*text_of(&name)` for a name in bytes; a reader takes
  `v.as_deref()` or `&v`, a literal stored beside it is
  `String::from("…")`, and a call that is only tested is
  `std::env::var(name).is_ok()` or `.is_err()`
- Needs: `Source::Environment` and `Pointer::environment` in the analysis,
  which a result takes on as it does `owned` and a parameter does not, so
  that `f(getenv(name))` lends `….ok().as_deref()` for the call; a borrow
  of a local's `String` counts as a pointer into the local
- Refused: a store through the text, a move of the pointer, `free` and a
  byte read at an index, `v[i]`, which make it bytes; the text kept in a
  field or a global; a borrow of it returned; a program argument stored in
  the same pointer; `setenv`, `putenv` and `environ`
- Motivation: `g3_envy` was refused for the pointer that `getenv` returns
- Changes: extends R10 and R13

## R40: a search of text or bytes

- C: `strchr`, `strrchr`, `memchr`, `strstr` and `strpbrk` of a `char`
  array, a slice, text or a program argument, where the result is only
  tested: `if (strchr(s, c))`, `!= NULL`, `== NULL`, `!`, also inside `&&`,
  `||` and `?:`; and a local pointer that only ever holds such results or
  NULL and is only tested
- Rust: the place found as an `Option<usize>`, an index into the bytes of
  the first argument: `s.find('x')` and `s.rfind('x')` of text for a
  constant ASCII character other than NUL, `s.find(t)` of two texts, and
  for the rest `find_byte`, `rfind_byte`, `find_bytes` and `find_any` on
  the bytes without their NUL, which find the NUL itself at the end and an
  empty needle at 0; `memchr` is
  `a.iter().take(n as usize).position(|&found| found == c)`. The character
  is the byte C converts it to, `c as u8`. A test is `.is_some()`, or
  `s.contains('x')` and `s.contains(t)` for text; the local is
  `let found: Option<usize> = …;`
- Needs: the rewrite `normalise::found`, which gives the local
  `Type::Found`; `strchr`, `strrchr`, `strstr` and `strpbrk` read their
  arguments as text, so a `&str` stays one
- Refused: the result used as a pointer, by the analysis's refusal of a
  pointer from a library function; `memchr` of text, of an array of other
  than `char` or `unsigned char`, and of a byte with a side effect, which
  the closure would repeat
- Motivation: `g3_envy` tests whether text from the environment holds a
  character
- Changes: none

## R38: what a function of `math.h` reports through `errno`

- C: `errno = 0; r = pow(x, y);` or `double r = sqrt(x);`, then reads of
  `errno` as R37 takes them, for a `double` `r` and arguments that are
  `double` variables or constants, none of whose address the function
  takes; a store to `r`, `x` or `y` ends what the reads may follow
- Rust: the call as before, `x.powf(y)`, and each read a test of the
  values as glibc reports: for `pow`, `EDOM` where `r` is NaN and neither
  argument is, `ERANGE` where both are finite and `r` is infinite, or `r`
  is zero and `x` is not; for `sqrt`, `EDOM` where `x` is below zero.
  `r.is_nan() && (!x.is_nan() && !y.is_nan())`, `x.is_finite() &&
  y.is_finite() && (r.is_infinite() || r == 0.0 && x != 0.0)`;
  a test of a constant is decided. Refused, as R37 refuses a read: a
  result that is converted, an argument that is an expression, a read
  after a store to one of them; `log`, `exp` and the other functions of
  `math.h` have no rule of their own yet
- Needs: `normalise::errno`, which writes the tests with `isnan`, `isinf`
  and `isfinite` as `bool` calls of `__builtin_isnan` and its like
- Motivation: `g1_007_errno_pow` tests `errno` after `pow`
- Changes: extends R37; a constant on which `pow`, `sqrt` and their like
  call a method has its type, `10.0f64.powf(x)`, which Rust does not infer

## R44: `div`

- C: `div(x, y)`, `ldiv` and `lldiv`, and their results `div_t`, `ldiv_t`
  and `lldiv_t` with the members `quot` and `rem`, as a local, a
  parameter, a result or a field
- Rust: `DivT { quot: x / y, rem: x % y }`, a record that the translation
  writes once where the program names the type, `struct DivT { quot: i32,
  rem: i32 }` with `Clone`, `Copy` and `Default`. C divides towards zero, as
  Rust does. A division by zero and `INT_MIN / -1`, undefined in C, stop
  the program. Refused: an argument with a side effect, which the two
  divisions would repeat
- Needs: the front end reads the members of a record from a header whose
  members are `quot` and `rem`
- Motivation: `g1_022_stdlib_div` was refused for a member of a record from
  a header
- Changes: nothing

## R46: `rand` and `srand`

- C: `rand()` and `srand(seed)` where one function of the program holds
  every call of both
- Rust: `let mut random = Random::new(seed);` for the `srand` that is a
  statement of the function's body before every other call of the two,
  `random = Random::new(seed)` for any other, and `random.next()` for
  `rand()`. `Random` is a helper with glibc's numbers: each is the sum of
  the numbers 3 and 31 before it without its lowest bit, from 34 sums that
  `new` fills from the seed and steps 310 times. Without a first `srand`
  the function starts with `Random::new(1)`, as a C program does
- Refused: calls in more than one function, which would share one
  `Random`, and in a function other than `main`, which would start its
  `Random` again each time it runs; `srand` in a program without `rand`;
  `rand_r`, `random` and `drand48`, as before
- Motivation: `g1_016_switch_arith` was refused for `srand`
- Changes: nothing

## R47: a place that a search found, used where it was found

- C: a local pointer that only holds what `strchr`, `strrchr`, `memchr`,
  `strstr` or `strpbrk` found in one array, or NULL, and that is used as a
  pointer only in the branch of `if (p)`, `if (p != NULL)`, or the `else`
  of `if (!p)`, `if (p == NULL)`: `*p`, `p[i]`, `p - array`, `p + n`, and
  `p` handed on or returned
- Rust: the local is the `Option<usize>` of R40, the test
  `if let Some(p) = p`, and in that branch `array[p]`, `array[p + i]`,
  `p as i64` and `&array[p + n..]`: an index beside the array, as a cursor
  is (R24), so that the array stays usable beside it
- Needs: `normalise::found`, which writes the `if` as `Stmt::IfSome` and the
  uses as places in the array. The array is a local array or a pointer
  that nothing assigns after its declaration, so that it is the same memory
  at every use
- Refused, as before, with `a place that a search found, used as a
  pointer`: a use outside such a branch or under a test with `&&` or `||`,
  a store to the pointer or `p++` in the branch, searches of two arrays or
  of an offset (`strchr(s + 2, c)`) in one pointer, a search of a program
  argument, a search that goes on from the result in a loop
- Motivation: `g3_envy` reads where a colon was found, `g5_findrep` stores
  through what `memchr` found
- Changes: extends R40
## R51: a function that returns what a search found

- C: a function of the program whose every `return` gives NULL or what
  `strchr`, `strrchr`, `memchr`, `strstr`, `strpbrk` or another such
  function found in its first parameter, which the function never stores
  to: `char *find(const char *b, size_t n, char c) { if (!b) return NULL;
  return memchr(b, c, n); }`; and its result kept in a local as R40 and R47
  keep a search's: `p = find(buffer, n, c); if (p) { at = p - buffer; }`
- Rust: the function returns `Option<usize>`, the place counted from its
  argument: `return None;`, and the search itself as R40 writes it. The
  caller's local is the `Option<usize>` of R40 and an index beside the
  argument where its test holds (R47)
- Needs: `normalise::found` names such functions in `Program::searches`
  before it types the locals, and runs again without one whose result is
  used in another way anywhere. The array of R47 may now be a local pointer
  that the program stores to, where the test of the place follows its
  search at once and the branch does not store to the pointer
- Refused, as before, with `a place that a search found, used as a
  pointer`: a result that is tested without a local, used where no test
  holds or after `if (!p) return;`, or called through a pointer to the
  function; a function that searches an offset of its parameter, another
  parameter than the first, moves the parameter or also returns another
  pointer; a store to the searched pointer between the search and the use
- Motivation: `g5_charinbuf` wraps `memchr` in a function and takes the
  position from the result
- Changes: extends R40 and R47
## R54: two elements of one array in one call

- C: a call with `&a[i]` and `&a[j]` as two arguments, elements of one
  array of which the function stores through at least one, where `i` is
  below `j` on every run: both are constants, or one is a constant and
  the other the counter of a `for` loop that starts above it, `for (int i
  = 1; …; i++) copy(&a[0], &a[i]);`, with a step that adds to it and a
  body and a condition that leave it alone; the counter is an `int` or
  wider and signed, so that it may not wrap back to the constant, and it
  is the index itself, not cut to a narrower type; or one index is the
  other and a constant more or less, `swap(&a[i], &a[i + 1])`, `&a[k - 2]`
  beside `&a[k]`, for an `i` that is an `int` or a `long` read without an
  effect, whose sum does not wrap
- Rust: the array is split before the later element, so that each
  argument borrows a part of its own: `let (lower, upper) =
  a.split_at_mut(i as usize); copy(&lower[0], &mut upper[0]);`, in a
  block where the call's value is used. `lower` and `upper` are names
  that no C name keeps
- Needs: `Lower::counters`, the counters of the open `for` loops with the
  constant they start at
- Refused, as before, with `two arguments that point into the same
  memory, one of them to store through`: indices that may be equal (a
  counter that starts at the constant, a variable, a loop that counts
  down or whose body moves the counter, two different variables, an
  unsigned or narrow index beside itself and a constant more, which may
  wrap), more than two such arguments,
  and a parameter that reads on from its element as an array, which
  would reach the other argument
- Motivation: `g4_memcpy_fun_buffers` copies its first buffer to each of
  the others
- Changes: R7 no longer refuses these calls
## R52: a loop that searches on from what it found

- C: a local pointer that starts at an array and that the condition of a
  `for` or `while` loop stores the next find in: `for (s = text; (s =
  strchr(s, c)); s++)`, `while ((s = strstr(s + 1, part)) != NULL)`, with
  `strchr`, `strrchr`, `strstr` or `strpbrk`; the step and the body move
  it by statements of their own, `s++;`, `s += n;`, and use it as a place,
  `*s`, `s[i]`, `s - text`, `s` handed on
- Rust: the pointer is an index beside the array that starts at 0, and the
  loop is `while let Some(at) = find_byte(until_nul(&text[s..]), c) { s +=
  at; … }`, the search of R40 in the rest of the array; with `s + 1` the
  rest is `&text[s + 1..]` and the index moves by `1 + at`. The body's uses
  are those of R47, the step comes last in the body
- Needs: `normalise::found` rewrites the loop to one without a condition
  whose body starts with `Stmt::LetSome` of the search and `break`; the
  rule for `for` writes that start as `while let`. The array is an array
  or a pointer that the function does not store to. The pointer is declared
  in the `for`, or just before the `while` and not named after the loop,
  where it is NULL
- Refused, as before, with `a place that a search found, used as a
  pointer`: a pointer that is read after the loop, that starts at an
  offset, that another pointer is stored in, or whose step has a value
  that is read (`*s++`); a search that reads the pointer twice
  (`memchr(s, c, n - (s - text))`). `continue` with a step leaves the
  body as in any loop with a step (R58)
- Motivation: `g2_028_strchr` counts the finds of a character
- Changes: extends R40 and R47
## R42: pointers to functions

- C: a pointer to a function, `R (*)(A, B)`, also through a `typedef` of
  the pointer or of the function type, and a parameter written as a
  function; as a local, a parameter, a result, a field, a global and an
  element of an array; its value: a function's name, `&f`, `*f`, another
  such pointer, `c ? f : g`, a call's result; a call through it: `p(x)`,
  `(*p)(x)`, `s.op(x)`, `ops[i](x)`, `choose(k)(x)`
- Rust: the type `fn(A, B) -> R`, `fn(A)` for `void`, in every place; a
  record that holds one stays `Copy` and has no `Default`; a global that
  never changes is a `const`, an array of them a `static`; the value is
  the function's path, and the call `p(x)`, `(s.op)(x)`,
  `ops[i as usize](x)`, `choose(k)(x)`, the callee evaluated first. One
  that NULL may reach is `Option<fn(A) -> R>`, with `None`, `Some(f)`, and
  the tests and the narrowing of R10: `if let Some(f) = f`, `let Some(f) =
  f else { return; }`; a call where no test holds is `f.unwrap()(x)`, as
  C's behaviour is undefined there. The functions whose address is taken
  and that share a C function type take one `globals` parameter, the
  strongest that one of them needs (`&mut Globals` over `&Globals`), as
  `_globals` in one that has it only for the others; the `fn` type has
  it first, `fn(&mut Globals, i32) -> i32`, and a call through such a
  pointer passes `globals`, so that its caller takes them too
- Needs: `Facts::pointer`, whose slots hold a pointer to a function as
  any other pointer but get no fact from it but `nullable`; a function
  is never NULL, and nor is an element of an array of them; the rewrite
  `normalise::narrow`; `Facts::access`, which reaches its fixed point
  with the classes of function types, `Facts::class_access` and
  `Facts::idle`
- Refused: a pointer to a function that takes or returns a pointer, a
  comparison of two functions, a cast to another function type, to
  `void *` or to an integer, a function with variable arguments, the
  address of a library function and one handed
  to the library, a pointer to a pointer to a function, `sizeof` of one,
  an array of them into which a pointer that may be NULL is stored, and a
  call through one whose class changes the globals with an argument that
  uses them too, `f(f(x))`, since Rust lends `globals` to the outer call
  while its arguments run
- Motivation: `g5_mathop` and `g5_charinbuf` call functions through
  pointers of a `typedef`; they are refused next for `time` and for the
  result of `memchr`
- Changes: R1, whose calls by name stay as they are; the front end reads
  a function used as a value as a pointer to it. R10 holds a pointer to a
  function that may be NULL, and R9 passes the globals through one
## R43: `void` pointers that hold one type

- C: `void *` locals, parameters, results, fields and globals that hand the
  pointer on to each other through assignments, initialisers, arguments,
  `return` and conditional expressions, and that every pointer stored into
  them and every conversion of them, with or without a cast, gives one
  type: `unsigned char *d = (unsigned char *)p;`, `int *q = p;`,
  `*(T *)p`, `((T *)p)->field`; NULL in them; `malloc` and `calloc` kept in
  one before the conversion
- Rust: the slots with that type and its rules (R7, R8, R10), without the
  conversions: `p: &[u8]` for bytes, `ctx: &mut Acc` for one record,
  `Box<i32>` for an allocation. The arguments of the library's `void *`
  parameters keep their own types
- Needs: the rewrite `normalise::voids`, which runs after `unions` and
  before every other stage
- Refused: a group whose type nothing says; a group that holds two types,
  naming them, where `char`, `signed char` and `unsigned char` are three;
  a function kept in a `void` pointer; arithmetic on one, `sizeof(void)`
  and a read through one; `void **` and a `void *` in an array
- Motivation: `g5_siphash` hashes the bytes of an array through a `void *`
  parameter and is refused next for a `case` that falls through;
  `g5_aabb` and `g5_agglom` hold one of several records, and
  `g5_call_predict` and `g5_get_predict_func` keep functions
- Changes: extends R7, R8 and R10

## R45: a `case` that falls through

- C: a `switch` whose cases run into the case after them without `break`,
  up to the case that leaves the `switch` with `break`, `return` or
  `continue`, or to the end of the `switch`; `default` first, in the middle
  or last among them; `__attribute__((fallthrough));` and
  `[[fallthrough]];`, which say nothing more
- Rust: one arm for all their labels, `entry @ (7 | 6 | 5) => { … }`, that
  binds the value, which is evaluated once. Each part but the last runs
  only for the labels at or before it, `if entry == 7 { … }`,
  `if matches!(entry, 7 | 6) { … }`; from `default` on, for every value but
  the labels after it, `if entry != 3 { … }`, and the arm is `entry => …`,
  last in the `match` as `_` is. A `break` inside a part leaves the arm as
  in R32
- Needs: nothing
- Refused: a variable that one case declares and a later case of the
  `switch` uses, falling into it or not, whose scope in Rust would end with
  its `if` or its arm; a `case` inside a loop or block of another case, as
  in Duff's device
- Motivation: `g5_siphash` reads the last bytes of a hash through cases
  that fall through; `g5_cleanup` has such cases beside a `goto`
- Changes: extends R3 and R32; the name `entry` is kept for the translation

## R48: `goto` to a label that follows

- C: `goto label;` where the label is a statement of a block and every
  `goto` to it is in the statements of that block before it, at any depth:
  the jump to the cleanup at the end of a function, out of nested loops,
  past the rest of a loop's body
- Rust: the statements from the first that holds such a `goto` to the
  label are a block, `'cleanup: { … }`, and each `goto` is
  `break 'cleanup;`. The label's name is in snake case, with `_label`
  where Rust keeps the name for itself
- Needs: `narrow` leaves a guard, `if (!p) goto out;`, a plain test where
  a label follows it in its block, since the label is reached without it
- Refused: a `goto` into a loop or a branch, with
  `` `goto label` into a block from outside it `` (a jump back is R49's);
  a `break` or `continue` of a loop around the block inside it, which Rust
  does not allow in a labelled block; a local declared in the block and
  used after the label; two labels of a function whose Rust names are the
  same, such as `a_b` and `a__b`
- Motivation: `g5_cleanup` jumps to its cleanup
## R34: `assert`

- C: `assert(e)` as glibc's `assert.h` expands it, as a statement or on the
  left of a comma; with `NDEBUG`, `((void) (0))`
- Rust: `assert_holds(e, "…");`, a helper that prints the message to
  standard error and calls `std::process::abort()` where `e` is false. The
  message is ``FILE:LINE: FUNCTION: Assertion `E' failed.`` with the texts
  of the expansion in one literal, and the condition is written by the
  rule for conditions (R3), evaluated once; nothing
  where the analysis decides that it holds, or with `NDEBUG`. `abort`
  rather than `assert!`, which ends the program with another exit code
- Needs: `Facts::truth`
- Refused: an `assert` where a value is needed
- Motivation: the development programs `g5_hm_geti`, `g5_intput`,
  `g5_sh_puts` and `g5_str_put` were refused for Clang's `StmtExpr`
- Changes: nothing

## R50: `fread` of bytes from `stdin`

- C: `fread(buffer, size, count, stdin)` into a `char`, `unsigned char` or
  `uint8_t` array, a pointer into one (`buffer + 2`), a parameter that
  points to bytes or an allocation; any size and count, also 0; its result
  used, tested or discarded
- Rust: `read_items(&mut buffer[..size * count], size)`, the whole array
  where that is its length and `&mut buffer[2..9]` for an offset: a helper
  that reads bytes until the slice is full or the input ends and returns
  the number of whole items as a `u64`, 0 for a size of 0.
  `read_items_within(buffer, size, count)` where the bytes are not
  constants that the array is known to hold: C is right with more bytes
  than its buffer as long as the input ends within it, so that helper
  reads into as much of the buffer as there is and stops the program where
  the input goes on beyond it, where C stores beyond the buffer; a size
  times a count beyond `usize` is more than any buffer
- Needs: the helper `read_byte` of R11, so that `scanf`, `fgets` and
  `fread` share what is left of the input
- Refused: elements that are not bytes, such as `int` or a record, with
  `` `fread` into elements that are not bytes ``, `signed char`, whose
  bytes would be `i8`, and a stream other than
  `stdin`, with `` `fread` on a file other than `stdin` ``
- Motivation: `g2_028_strchr` reads its input with
  `fread(in, 1, sizeof(in), stdin)`; it is refused next for a pointer that
  `strchr` found and that steps on
- Changes: none
## R49: `goto` back, or past another label

- C: the labels of a block that R48 cannot write as blocks that end at
  them: a `goto` from below the label, such as `retry:` at the top of a
  function or a jump back inside a loop's body, or a jump forward into the
  part after one label from before another; every `goto` to them is inside
  the block
- Rust: the statements before the first that holds such a `goto` stay as
  they are, as they run once; the rest, split at the labels into parts, is
  `'entry: loop { … }`, where each part but the last is
  `if entry <= k { … }` (`entry == 0` for the first) and the last ends with
  `break;` unless it leaves on every path. A `goto` is
  `entry = k; continue 'entry;`, with `let mut entry = 0;` before the loop
  where a part tests it. A `break` or `continue` in a part of a loop around
  the block names the loop, `'outer: while …` with `break 'outer;`, one
  inside another `'outer_2`; a C label of such a name gets `_label`
- Needs: nothing
- Refused: a `goto` into a loop or a block from outside it,
  `` `goto label` into a block from outside it ``; to a label under a
  `case`; from a `switch` inside the block, whose arm may have an `entry`
  of its own (R45); a block entered at a label inside another; a local
  declared in one part and used in a later one or by the function's last
  `return`, and a `goto` over a declaration with an initialiser into its
  scope; a computed `goto`
- Motivation: `g5_021_complex_goto` jumps back to a label in a loop's body,
  and forward past it to the next
- Changes: R48 keeps the blocks whose labels it writes; a block with one
  label that it cannot write takes R49 for all its labels, so a jump back is
  no longer refused, and R48's refusals of a jump into a loop or a block
  name the jump. A C label `outer` or `outer_2` gets `_label` in R48's block
  too. A `main` with an exit code that ends with a `goto` gets no
  `ExitCode::SUCCESS` after it (R1), as the loop does not end
## R53: one pointer handed in several arguments

- C: a call of one of the program's functions that hands the same pointer
  in several arguments, `f(out, out, out, len)`: reads of one variable, one
  array or the address of one variable, under any conversions, in a call
  whose arguments have no side effects, where the function stores through
  or owns one of the parameters that receive it
- Rust: a copy of the function in which the parameters that receive the
  pointer are one, the first of them, `f_same(out, len)` with
  `fn f_same(out: &mut [i32], len: i32)`; its parameters and
  locals are variables of its own, and a `static` local stays the one
  variable it is. One copy for each grouping of a function's parameters,
  with the function's name and `_same`, `_same2`, … until no function or
  global has it; a call in a copy is rewritten in the same way, so a
  function that calls itself with the same arguments calls its copy. The
  function stays as it is beside its copies
- Needs: `normalise::same`, on the facts of the program as written:
  `Facts::pointer` for whether a parameter is written or owned, which is
  what the lowering of the call refuses, and `Facts::mutated` and
  `Facts::addressed` for a parameter that may stop being the pointer it was
  handed
- Refused: with R7's refusal, a group whose parameters have different
  types, that the function assigns, moves or takes the address of, a call
  with a side effect in any argument, and the same pointer written another
  way: offsets into one array, `f(a, a + 1)`, and the same field,
  `f(s->data, s->data)`
- Motivation: `g3_030_mutable_buffer_overlap_extrahard` hands one buffer
  as the result and every operand of a function
- Changes: narrows R7's refusal of two arguments that point into the same
  memory
## R35: a statement expression

- C: Clang's statement expression `({ stmts; value; })` with a value of an
  integer, floating-point, `_Bool` or enumeration type, in any position of
  a value, nested, or as a statement of its own, with or without a value;
  inside it declarations, expression statements, `if`, loops and `switch`
- Rust: a block `{ stmts; value }` in the same position, and a block
  statement `{ stmts; }` for one whose value nothing reads; one without
  statements is its value. The block is a scope, as in C. A declaration
  inside has its first value as `c::ExprKind::Init` after it, so that the
  walks over expressions reach it, and is `let` with that value
- Needs: the analysis reads the statements inside as those of the
  function; `cursors`, `ends` and `narrow` leave a function that holds one
  with statements as it is
- Refused: a value that is a pointer, a record, an array or a function; no
  value where one is needed; `return`, `goto`, a label, and a `break`,
  `continue` or `case` that leaves the statement expression; a `static`
  local, an array of variable length and a pointer without a first value
  inside
- Motivation: the development programs `g5_hm_geti`, `g5_intput`,
  `g5_sh_puts` and `g5_str_put` expand macros into statement expressions
- Changes: R34 reads its own expansion first


## R57: a documentation comment

- C: a comment that starts with `/**`, `///`, `/*!` or `//!`, or with
  `/**<` or `///<` after what it describes, at a function, a parameter, a
  record and its fields, an enumeration and its values, a `typedef`, a
  global or a local variable. Clang hangs it on the declaration as a
  `FullComment`; an ordinary comment is not in its tree
- Rust: nothing, as for an ordinary comment: the declaration is read as
  it is without the comment
- Needs: nothing
- Motivation: programs outside the development set document their
  functions in this way and were refused for Clang's `FullComment`; no
  development program needs it
- Changes: none


## R58: `continue` in a loop that has something to do after its body

- C: `continue` in a `for` loop with a step that is no count of R3, `for
  (i = 0, j = n; i < j; i += 2, j--)`, and in a `do` loop, where it goes
  on with the step or with the test; also inside an `if`, a block or a
  `switch` of the body
- Rust: the body is a block with a label that the `continue` leaves, and
  the step or the test follows the block: `while i < j { 'next: { …; if
  skip { break 'next; } … } i += 2; j -= 1; }`, and `loop { 'next: { … }
  if !test { break; } }` for `do`. Rust has no plain `break` inside a
  block with a label, so a `break` of such a loop names it, `break
  'outer;`. The labels are `'next` and `'outer`, with the depth of the
  loop from the second on (`'next_2`); a C label of that name gets
  another
- Needs: nothing
- Motivation: programs outside the development set skip the rest of a
  `do` body or of a `for` body with a step; no development program needs
  it
- Changes: R3 refused them (`` `continue` in a `do` loop ``, `` `continue`
  in a `for` loop that is not a count ``)

## R56: records and enumerations without a tag

- C: a record, a union or an enumeration without a tag, as the type of a
  variable, local or global, of several variables of one declaration
  (`struct { int x; } a, b;`), of an array, a pointer or a field
  (`struct s { struct { int a; } in; }`), and an anonymous member of a
  record (C11), `struct s { int kind; struct { int a; int b; }; }`
- Rust: the record is a `struct` named after the variable or field that
  is the first to have it, in camel case (`Point`); where the name is
  taken, a field's record takes the name of the record that holds it in
  front (`ShapeOrigin`) and a variable's an `_` (`Cell_`). A record, with
  a tag or without, that would have a name of Rust's prelude gets an `_`
  as well (`Box_`, `Option_`), since it would hide that type. An anonymous
  member is a field named `anonymous` whose type is the record, and a use
  of one of its members goes through it, `v.anonymous.a`, as Clang writes
  the use. Two records without a tag that look the same are two types. A
  union is translated as R33 translates one with a tag, an enumeration as
  R2 does, and the rules for records apply from there
- Needs: nothing
- Refused: a type without a tag in a function's parameters or result,
  `` a type without a tag in a function's parameters or result ``; a union
  that is not the bits of a number, as with a tag; a type without a tag
  that no declaration after it names, such as one inside `sizeof` or a
  cast, as `` the type `struct (unnamed at …)` ``
- Motivation: programs outside the development set declare variables and
  fields of types without a tag; no development program needs it
- Changes: the front end finds a type without a tag by the place of its
  declaration, which replaces R33's reading of a union without a tag right
  before the declaration that uses it, and no longer refuses a member of an
  anonymous record

## R60: compound literals

- C: a compound literal that is only read, `(struct point){ 1, 2 }`, with
  designators, missing fields and nested records, as an initialiser, the
  value of an assignment, an argument, a returned value, a field of another
  initialiser or a global's initialiser, and `(struct point){ 1, 2 }.x`;
  a number, `(int){ 5 }`
- Rust: the record's literal by R5's rule for an initialiser list of its
  type, `Point { x: 1, y: 2 }`, in parentheses before a field; the number
  converted to the type
- Needs: nothing
- Refused: one that is stored to or used as a place, `` a compound literal
  as a place: no rule yet ``; its address, `` a pointer to a compound
  literal: no rule yet ``; an array, `` a compound literal that is an
  array: no rule yet ``; a string, `(char[]){ "…" }` or
  `(const char *){ "…" }`, `` a compound literal that is a string: no rule
  yet ``; another pointer, `` a compound literal that is a pointer: no rule
  yet ``
- Motivation: programs outside the development set write records as
  compound literals; no development program needs it
- Changes: the front end reads a compound literal that is only read as its
  initialiser list with the literal's type; a place may be the field of
  such a value

## R62: a global pointer that never changes is the place it names

- C: a pointer at file scope, or `static` in a function, that nothing
  assigns, increments or takes the address of, and whose initialiser is the
  address of a global, of a field of one (`&s.in`), of an element at a
  constant index (`&a[2]`), or a global array used as a pointer
  (`int *p = table;`); a record's compound literal as such an initialiser,
  `struct s *p = &(struct s){ 1, 2 };`, with designators and nested records
- Rust: no pointer: every use is the place, so that `*p` is `x`, `p->a` is
  `s.a` and `p[i]` is `table[i]` by R9's constants and the fields of
  `Globals`; an index or an offset through a pointer to an element,
  `p[n]`, `*(p + n)`, `*(p - n)` and `(p + n)->f` for `&a[2]`, is `a[2 +
  n]`, `a[2 - n]` and `a[2 + n].f`, for an `n` that may be negative: a
  constant, an `int` or a `long` is added in its type, and any other
  integer as `long`; the compound literal is a global of its own that holds its
  value, under the pointer's name (or the fallback that the names stage
  gives a global whose name is taken), a constant or a field of `Globals`
  where the program stores through the pointer
- Needs: `normalise::places`, before the first analysis
- Refused: a global pointer that changes or whose address is taken, `` the
  pointer `p`: it owns memory and also points into a variable ``; one that
  is NULL or has no initialiser, `` the global pointer `p` that never
  changes ``, as before; the address of an array's compound literal, `` a
  compound literal that is an array: no rule yet ``, and of a compound
  literal anywhere else, `` a pointer to a compound literal: no rule yet ``
- Motivation: programs outside the development set name a global through a
  pointer that never changes; no development program needs it
- Changes: R9 no longer sees such a pointer; R60's refusal of a compound
  literal's address no longer holds for a global pointer's initialiser

## R59: `realloc` of an owned array

- C: `p = realloc(p, n * sizeof(T))`, also with `sizeof *p`, `sizeof(T) *
  n`, a bare `sizeof` for one element, and a plain count for an array of
  bytes, with or without a cast of the result, where `p` is a local, a
  field or a global that owns its array
- Rust: for a size that is a product of literals and `sizeof`,
  `p.resize(4, zero)` on the `Vec<T>` that the slot is, and
  `p.get_or_insert_with(Vec::new).resize(…)` where it may be NULL, since
  `realloc(NULL, size)` is `malloc(size)`. Any other size may be 0 when
  the program runs. What C does then is the library's choice, and the
  translation does what the library of the reference does, glibc: it
  frees the array and returns NULL, and for a pointer that is NULL
  already it returns an empty array. So NULL may reach the slot, an
  `Option<Vec<T>>`, and the count is found first: `{ let grown_len = n as
  usize; if grown_len == 0 && p.is_some() { p = None; } else {
  p.get_or_insert_with(Vec::new).resize(grown_len, zero); } }`. The
  elements that C keeps are kept; the new ones are zero, where C leaves
  them without a value
- Needs: `Facts::pointer` (`owned`, `many`, `nullable`): the result of
  `realloc` is a fresh array, or NULL unless `facts::fixed_size` knows
  the size, and its first argument owns what it frees, as `free`'s does
- Refused: the result stored anywhere but the pointer it grows, tested or
  not stored (`` `realloc` whose result is not stored back in the pointer
  it grows: no rule yet ``), which includes `q = realloc(p, size)` and the
  test of a second pointer for NULL before it replaces the first; a size
  that is no count of elements written with `sizeof` (`` a size of
  `realloc` with no rule ``); a constant size of 0 or one that is not such
  a product; a count that is not constant and 64 bits wide, since C keeps
  the low 64 bits of the product, which may be a small array; the array
  of a parameter, which the caller still holds
- Motivation: programs outside the development set grow an array with
  `realloc`; no development program needs it alone
- Changes: R8's slots owned only memory from `malloc` and `calloc`

## R61: a local pointer that only names one place

- C: a local pointer whose address is not taken, that gets one value, in
  its declaration or in a statement of the block that declares it before
  anything reads it, and that value is the address of a place that stays
  the same place: a variable that is no pointer, a field of one, an element
  at a constant index, or such an array used as a pointer, `T *p = &x;`,
  `int *e = &m[1][2];`, `char *c = buf;`, `int (*r)[4] = rows;`
- Rust: every use of the pointer names the place, `*p` is `x`, `p->f` is
  `x.f`, `p[i]` is `buf[i]`, `f(p)` is `f(&mut x)`, and for `e = &m[2]` an index or an offset, `e[n]`
  and `*(e - n)`, is `m[2 + n]` and `m[2 - n]`; the pointer and its store
  are left out
- Needs: `Facts::in_use_by`; only a pointer that `Facts::in_use` would
  refuse is rewritten, so a borrow that compiles stays a borrow
- Refused: a pointer stored to twice, in a branch or a loop, moved, a
  parameter or with its address taken, one to a place through another
  pointer or at an index that is no constant, and one used where a variable
  with the name of the place hides it, stay refused by R7's reason
- C, too: a local `T **pp` with one value as above that is `&p` for a
  pointer variable `p`, a local or a parameter, used only as `*pp` and
  `pp[0]`, `int **pp = &p; **pp = 4; if (*pp)`, `(*pp)->f`, `*pp = e`
- Rust, too: `*pp` and `pp[0]` are `p`, `**pp` is `*p`, `(*pp)->f` is
  `p->f`, `*pp = e` is `p = e`, wherever `pp` is and whatever `p` holds
  meanwhile; `pp` and its store are left out, and the older rewrite or a
  cursor may then rewrite `p`
- Refused, too: `pp` stored to twice, moved, indexed at another place than
  0, compared, handed to a function, returned, cast, with its address taken
  or a parameter; `&s.f`, `&q->f`, `&a[i]`, `&global`; `void **`, `T ***`;
  a use where another `p` hides it: all stay refused as before. `sizeof pp`
  and `sizeof *pp` are 8 either way
- Motivation: programs outside the development set use a variable beside a
  pointer to it, or a second name for a pointer; no development program
  needs it
- Changes: `normalise::alias` runs after `cursors`, and the facts after it;
  `normalise::pointers` runs after `found`, before the first analysis, and
  shares the search for the one store with `alias`

## R63: a pointer to a function without a prototype

- C: a pointer to a function without a prototype, `int (*f)()`, in a
  global, a local, a field, a parameter, a result, a `typedef` and an
  element of an array; the functions that reach it, defined with no
  parameters (`int zero()`, `int zero(void)`) or with parameters, by name,
  through `c ? f : g` or another pointer; calls through it
- Rust: R42's `fn` type with the parameters of those functions,
  `fn() -> i32`, `fn(i32, f64, Span) -> f64`; `Option<fn() -> i32>` for
  one that only holds NULL and is only tested
- Needs: the rewrite `normalise::protos`, which runs after `voids`: all
  such pointers with one result type are one group, whose parameters are
  those of every function converted to it or from it, and which every
  call through it must pass, as C's default promotions leave them
- Refused: functions with different parameters in one group, `` a
  pointer to a function without a prototype that holds functions of
  several types ``; a call with another number of arguments, an argument
  of another type or a parameter that the promotions change (`char`,
  `short`, `float`), `` a call through a pointer to a function without a
  prototype with arguments its functions do not take ``; a call that no
  function reaches, `` a call through a pointer to a function without a
  prototype that no function reaches ``; a variadic pointer, `` a pointer
  to a variadic function ``
- Motivation: programs outside the development set keep functions in
  pointers without a prototype; no development program needs it
- Changes: R42 refused every pointer without a prototype together with
  the variadic ones; a variadic one keeps its own refusal

## R64: a pointer field that never changes is the place it names

- C: a field of pointer type, not to a function, of a record whose every
  variable is a global or a `static` local with an initialiser that gives
  the field a place as R62 allows (`&x`, `&s.f`, `&a[2]`, an array), by
  position or designator; nothing stores to the field (`v.p = …`,
  `q->p = …`, `v.p++`), takes its address, stores a whole record of the
  type, converts a pointer to one (`memcpy`, `malloc`), takes its size, or
  makes one otherwise (a local, a parameter, a result, an element, a field
  of another record, a compound literal)
- Rust: every read `v.p`, and `q->p` for a pointer `q` that R62 replaced
  by `&v`, is `v`'s place, so that `*v.p` is `x`; two globals give their
  own places; the record has no such field
- Needs: `normalise::places`, after R62's replacement and before the first
  analysis
- Refused: a field that is NULL, missing or anything else in some
  initialiser, that changes or whose address is taken, of a record that
  is reached through a pointer, copied, sized or not a global, or that
  points into a record of its own type, `` the field `p`: it owns memory
  and also points into a variable ``, as before; a compound literal's
  address as the field's initialiser, `` a pointer to a compound literal:
  no rule yet ``, as before
- Motivation: programs outside the development set name a global through
  a pointer field of a global record that never changes; no development
  program needs it
- Changes: extends R62 from global pointers to such fields

## R65: a pointer to a pointer borrows the place it names

- C: a parameter `T **pp`, `T` a record, a number or `char`, whose
  arguments are `&p` for a local or a parameter `p`, `&s.f`, `&s->f`, or
  the caller's own such parameter; in the function `*pp = e;`, `*pp` as a
  value, `(*pp)->f`, `(*pp)[i]`, `if (*pp)`, `*pp == NULL`, `!*pp`,
  `free(*pp)` and `*pp = (*pp)->next;`
- Rust: `pp: &mut P` where the function stores to `*pp` or through it,
  moves out of it or frees it, and `&P` otherwise, for the type `P` of
  the place behind (`Option<Box<T>>`, `Vec<i32>`, `Option<Vec<u8>>`); the
  argument `&mut p`, `&p`, or the parameter by its name; `*pp` is the
  place `*pp` with the rules of a pointer in a field: `(*pp).take()` for a
  move out of it, `(*pp).as_deref_mut().unwrap().f`, `(*pp).is_some()`
- Needs: `Slot::Behind` in the analysis: the facts of the place behind
  `pp` are equal to those of every place that an argument names (`pp =
  &p`, `pp = qq`, `&s->f`), not ordered by a flow; a store through `*pp`
  marks the place behind written, a store to `*pp` or a move out of it
  `pp`; `same` (R53) leaves a `T **` parameter alone
- Refused: a move out of `*pp` (`t = *pp`, `*to = *from`, `return *pp`)
  unless nothing can read the place before it is stored to: the moving
  statement is an expression or a declaration that names `pp` only in
  the move, and the next statement is `*pp = e` with an `e` that names
  no `pp` and calls no function of the program or through a pointer. C
  still holds the pointer that Rust takes out, `` a move out of the place
  behind a pointer that something may read before it is stored to: no
  rule yet ``; `&p` for a `T **` that is written, while a pointer that
  borrows `p` is used after the call, `` a variable that is used while a
  pointer to it is in use ``; the place behind holds a borrow, as `const char **out` that
  gets a literal, `` the place behind the pointer `out`: a borrow behind
  a pointer: no rule yet ``; the address of a global's pointer as the
  argument, `` the address of a global's pointer for a pointer to a
  pointer: the callee would borrow the globals twice ``; another argument
  of the call that reads a variable of the argument, `f(p, &p)`, `f(&p,
  &p)`, `` two arguments of one call that name the same place, or a
  place and the pointer in it: … ``; `*pp += n`, `(*pp)++`, `` a cursor
  behind a pointer: no rule yet ``; a local, a result, a field or a
  global of type `T **`, `pp[i]`, `pp + n`, `pp++`, a NULL argument, ``
  a pointer to a pointer ``; `T ***`, `void **`, a pointer to a pointer
  to a function, a cast to or from `T **`, `pp == qq`, each with its own
  reason
- Motivation: programs outside the development set hand the address of a
  list's head or of an array's owner to a function that replaces it; no
  development program needs it
- Changes: extends R7 and R8 by the slot behind a pointer, and the test
  for NULL, the move out with `take()` and `free` from a field to `*pp`

## R66: a read of an owner follows its memory

- C: `q = p;` or `T *q = p;` where `p` and the local `q` both own what
  they point to, and reads of `p` after it: `gone = list; total +=
  list->weight; list = list->rest; free(gone);`
- Rust: each read of `p` in the statements that follow in the block names
  `q`, up to the first statement that stores to `p` or `q`; of that one, a
  plain store to `p`, the value names `q` too: `let mut gone = list; total
  += gone.as_deref().unwrap().weight; list = gone.as_deref_mut().unwrap()
  .rest.take(); drop(gone);`
- Needs: `Facts::moved`, which finds the functions with such a read, and
  `owned` of both variables; `normalise::follow` rewrites only those
  functions
- Refused: as before (R8), a read after a statement that stores to one of
  the two in a branch or a loop, after a `case`, in a function with a
  label or a `goto`, where a local hides the name of `q`, where either
  has its address taken, and for a global `p` after a call of a function
  of the program; memory that went to a field, a global or a parameter is
  not followed
- Motivation: programs outside the development set take a node off a list
  through a second pointer and read the first again; no development
  program needs it
- Changes: `normalise::follow` runs after `alias`, and the facts after it;
  R8's refusal of an owner that is used after it was moved covers less
