# Research loop instructions

You improve a mechanical C-to-Rust transpiler, one rule per iteration. A
translation passes when it compiles as safe Rust and, with overflow checks on,
prints the same output and exits with the same code as the C program on every
input. The goal is the number of programs that pass, on programs you will never
see.

## What you may change

Only `crates/transpiler/` and `Cargo.lock`. The evaluator, the benchmarks and
these instructions are fixed; the gate rejects any other change. Keep to
`AGENTS.md`, and do not try to reach anything outside this repository: the
network, other directories or private benchmarks.

## One iteration

1. Run `cargo run -p eval -- --quiet` and pick one failing development program,
   or several that fail for the same reason.
2. Study it with `scripts/inspect bench/dev/NAME.c` and the evaluator's
   details (`cargo run -p eval -- --only NAME`).
3. Add or generalise one rule in `crates/transpiler/src/`. Translate C
   constructs, not programs: no program names, expected outputs or constants
   taken from a benchmark. Rules must stay mechanical; the transpiler may not
   call a language model or any network service.
4. Add the rule's entry to `crates/transpiler/CATALOGUE.md` and a test in
   `crates/transpiler/tests/`.
5. Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
   and the evaluator again. Programs that passed before should still pass.
6. Commit as `feat(transpiler): ...` and stop with a short report: the rule,
   the programs it fixed and any it broke.

## The gate

After each iteration the gate builds the transpiler from your commit and
evaluates it on the development set, a private validation set and private
variants of the development programs with renamed identifiers, changed
constants and new inputs. It keeps the commit when the development score rises
and the validation score does not drop, and otherwise reverts it. You see the
development results and the decision. Overfitting the development programs
fails on the rest.
