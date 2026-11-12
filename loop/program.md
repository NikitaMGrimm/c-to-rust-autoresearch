# Research loop instructions

You improve a mechanical C-to-Rust transpiler, one step per iteration. A
translation passes when it compiles as safe Rust and, with overflow checks on,
prints the same output and exits with the same code as the C program on every
input. The goal is the number of programs that pass, on programs you will never
see.

The translation should be Rust that a Rust programmer would write, not C kept
alive in safe Rust: values that Rust owns and frees (`Box`, `Vec`, `String`),
borrows where the program allows them, and the standard library instead of a
copy of C's. A pass is *strict* when the translation switches no lint off
(`allow`, `expect`), never gives up freeing memory (`leak`, `forget`,
`ManuallyDrop`) and keeps no scalars as bytes (`from_le_bytes`, `to_le_bytes`
and the like), as a program over an emulated C memory does. The evaluator
marks passes that are not strict and prints the strict score as `STRICT`. Meet
the intent, not the list: an arena that never shrinks or C's memory under
another name is no better than what the list names.

The transpiler has a design, in `crates/transpiler/DESIGN.md`: stages that
each do one thing, the Rust each C construct should become, and what is not
built yet. Read it before anything else and keep to it. It refuses a program
it has no rule for rather than translating it badly, so most development
programs fail with `unsupported: …`; those reasons are the work.

## What you may change

Only `crates/transpiler/` and `Cargo.lock`. The evaluator, the benchmarks and
these instructions are fixed; the gate rejects any other change. Keep to
`AGENTS.md`.

## What you may not see

The development programs come from DARPA's TRACTOR test corpus, which also
holds the private programs the loop is measured on. Do not fetch, clone or
search for that corpus or any copy of it, and stay inside this checkout, the
loop's own clone, made afresh for each iteration: no other directories, no
Claude Code files such as transcripts or settings, no web and no network.
Cargo works offline with the crates already fetched, so add no dependencies.
One look at a private program invalidates the experiment and cannot be undone.
Your settings block these, and every session is audited afterwards. When a
construct needs more examples than the development programs give, write small
C programs of your own as tests. Use Claude Code's own tools and subagents;
MCP tools, such as T3 Code's, are blocked, and the only skills are those in
`loop/skills`.

## One iteration

1. Read `ROUNDS.md`, if there is one: the earlier iterations, with the gate's
   results for each. If there is a `REVIEWS.md`, handle it first, as
   [Reviews](#reviews) says. Otherwise run `cargo run -p eval -- --quiet` for
   the scores and
   `cargo test -p transpiler --test dev -- --nocapture` for the reason each
   development program is refused. Pick one reason, best one that several
   programs share.
2. Study a program that has it with `scripts/inspect bench/dev/NAME.c` and
   the evaluator's details (`cargo run -p eval -- --only NAME`).
3. Make one step in `crates/transpiler/src/`, in the stage where the design
   puts it:
   - a **rule**: one C construct gets a translation, or an earlier rule
     covers more;
   - a **refactoring**: something that later rules need and that passes no
     program by itself, such as a fact in the analysis, a rewrite or a node
     in the C tree, or code that has grown hard to follow made simple again.

   Translate C constructs, not programs: no program names, expected outputs
   or constants taken from a benchmark. A rule that is wrong for some
   programs asks the analysis and refuses those; it never falls back to a
   translation that always works. Rules must stay mechanical; the transpiler
   may not call a language model or any network service.
4. For a rule, add its entry to `crates/transpiler/CATALOGUE.md` and a test
   in `crates/transpiler/tests/translate/` that asserts the Rust it should
   read as. For a refactoring, update `DESIGN.md` where it changes what the
   design says.
5. Run `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` and the evaluator again. Programs that passed
   before should still pass.
6. Commit a rule as `feat(transpiler): ...` and a refactoring as
   `refactor(transpiler): ...`, and stop with a short report: the step, the
   programs it fixed and any it broke.

## Reviews

Kept commits go to pull requests on GitHub, a batch at a time, where CI and
CodeRabbit check them and the owner may comment. A round writes what is open there into
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
- A comment comes back with CodeRabbit's answer when it disputes your reply.
  Fix it after all, or answer again with better reasons; after its second
  dispute the owner decides.

Answers alone do not end an iteration: a round without a commit posts
nothing, so every iteration still ends with a fix or a new rule.

## The gate

After each iteration the gate, run outside your session, builds the
transpiler from your commit and evaluates it on the development set, a private
validation set and private variants of the development programs with renamed
identifiers, changed constants and new inputs. It keeps the commit when the
development score rises, or for a fix or a refactoring does not drop, and
otherwise drops it;
it may also require that the validation score does not drop, and it may count
only strict passes in its scores. The next
iteration starts in a fresh checkout of the kept commits, whose `ROUNDS.md`
adds the commit, the development results and the decision. Do not run the
gate or the benchmark tools yourself. Overfitting the development programs
fails on the rest.
