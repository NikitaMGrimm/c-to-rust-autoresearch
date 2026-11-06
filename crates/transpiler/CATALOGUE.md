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
  into `&variable`
- Rust: `fn` with `i8`…`u64`, `let` with `mut` only for variables that are
  assigned or whose address is taken, raw identifiers for Rust keywords, the
  last `return` as the tail expression, C's `main` as `c_main` called by
  `std::process::exit`; operands in parentheses unless atomic; `print!` or
  `println!` with the format converted at translation time; `crt::scanf`, a
  runtime emitted once that reads standard input byte by byte as glibc does,
  saturating out-of-range numbers as `strtol` does. Every other construct
  becomes `compile_error!("unsupported C: ...")`
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
