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
