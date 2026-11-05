# Research loop instructions

You improve a mechanical C-to-Rust transpiler, one rule per iteration. A
translation passes when it compiles as safe Rust and, with overflow checks on,
prints the same output and exits with the same code as the C program on every
input. The goal is the number of programs that pass, on programs you will never
see.

## What you may change

Only `crates/transpiler/` and `Cargo.lock`. The evaluator, the benchmarks and
these instructions are fixed; the gate rejects any other change. Keep to
`AGENTS.md`.

## What you may not see

The development programs come from DARPA's TRACTOR test corpus, which also
holds the private programs the loop is measured on. Do not fetch, clone or
search for that corpus or any copy of it, and stay inside this checkout: no
other directories, no Claude Code files such as transcripts or settings, no
web and no network except cargo fetching crates. One look at a private program
invalidates the experiment and cannot be undone. Your settings block these,
and every session is audited afterwards. When a construct needs more examples
than the development programs give, write small C programs of your own as
tests. Use Claude Code's own tools and subagents; MCP tools, such as T3
Code's, are blocked, and the only skills are those in `loop/skills`.

## One iteration

1. Read `ROUNDS.md`, if there is one: the earlier iterations, with the gate's
   results for each. If there is a `REVIEWS.md`, handle it first, as
   [Reviews](#reviews) says. Otherwise run `cargo run -p eval -- --quiet` and
   pick one failing development program, or several that fail for the same
   reason.
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

## Reviews

Kept commits go to a pull request on GitHub, where CI and CodeRabbit check
them and the owner may comment. A round writes what is open there into
`REVIEWS.md`: review comments, each with its number, and failed checks.

- Fix one comment or failed check per iteration, in a commit of its own whose
  subject starts with `fix(transpiler):`. Update the catalogue entry of each
  rule it changes.
- Answer comments in `REPLIES.json` at the checkout's root, which no commit
  includes:

  ```json
  [{"comment": 123, "reply": "The rule now ...", "fixed": true}]
  ```

  `fixed: true` says that this iteration's commit fixes the comment; the
  reply is posted with the commit's hash once the gate keeps it. Answer with
  `fixed: false` when a comment needs no change or asks for one outside the
  transpiler, and say why. Replies are posted publicly as written, so write
  plain sentences without mentions.

## The gate

After each iteration the gate, run outside your session, builds the
transpiler from your commit and evaluates it on the development set, a private
validation set and private variants of the development programs with renamed
identifiers, changed constants and new inputs. It keeps the commit when the
development score rises, or for a fix does not drop, and the validation score
does not drop, and otherwise drops it. The next iteration starts in a fresh
checkout of main, whose `ROUNDS.md` adds the commit, the development results
and the decision. Do not run the gate or the benchmark tools yourself.
Overfitting the development programs fails on the rest.
