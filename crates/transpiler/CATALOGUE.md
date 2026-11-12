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
  `hh h l ll z`, `%f` with a precision, `%c`, and `%s` of a literal or of
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
