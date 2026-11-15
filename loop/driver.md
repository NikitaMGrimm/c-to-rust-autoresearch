# Driver instructions

You drive the work on a mechanical C-to-Rust transpiler from one thread: you
decide what it needs next, build it, and have a gate judge each step. The aim
is the best transpiler these models can make. Correct comes first: a
translation compiles as safe Rust and, with overflow checks on, prints the
same output and exits with the same code as the C program on every input.
Then idiomatic: Rust that a Rust programmer would write for the same program,
as `crates/transpiler/DESIGN.md` describes. Read that file before anything
else, and keep it true as the transpiler changes.

## Where you work

`loop/drive` prepared your checkout, a worktree with a branch of its own,
and left `local/judge` in it, which names your run, RUN. Without that file
the checkout is not prepared: stop and say so. The commits the gate kept are
on `run/RUN`, from the first judging on. Change only `crates/transpiler/` and
`Cargo.lock`; the evaluator, the benchmarks and these instructions are fixed,
and the gate rejects any other change. Do not edit the main checkout, push or
open a pull request unless the owner asks. Keep to `AGENTS.md`.

The transpiler, its tests and its build are agent code. Build, test and run
them only in the sandbox:

```sh
loop/cargo test -p transpiler
loop/cargo clippy -p transpiler --all-targets -- -D warnings
loop/cargo test -p transpiler --test dev -- --nocapture   # why each dev program is refused
loop/cargo run -q -p transpiler -- bench/dev/NAME.c -o local/NAME.rs
cargo run -p eval -- --quiet                               # the dev scores; sandboxes the transpiler itself
```

Never run plain `cargo` with `--workspace` or `-p transpiler`. `cargo fmt` is
fine. `local/` is ignored by git and is the place for your own files.

## What you may not see

The development programs come from DARPA's TRACTOR test corpus, which also
holds the private programs the transpiler is measured on. Never read, list or
search the private directory, the corpus or any copy of it, here or on the
web, and do not use the web at all. Your settings block these and your
transcripts are audited after each stretch. The held-out and transfer sets
are sealed and stay so until the work is over. One look at a private program
invalidates the results and cannot be undone. When a construct needs more
examples than the development programs give, write small C programs of your
own as tests.

## One step

1. Choose what to do. The refusal reasons of the development programs, the
   weak spots and the list of what is not built in `DESIGN.md`, and
   `local/rounds.md`, the gate's results so far, which the first judging
   writes, say what is missing. Prefer what unblocks the most, and what
   later steps will build on. Build a construct as a family, as the next
   section says, not as the one shape a development program has.
2. Make one step, in the stage where the design puts it:
   - a **rule**: one C construct gets a translation, or an earlier rule
     covers more. Add its entry to `crates/transpiler/CATALOGUE.md`, in the
     form that file gives; the gate rejects a rule without one. Add a test
     in `crates/transpiler/tests/translate/` that runs the C program and its
     translation and asserts the Rust it should read as.
   - a **refactoring**: a fact in the analysis, a rewrite, a node in the C
     tree, or code made simple again. It passes no new program by itself.
     Update `DESIGN.md` where it changes what the design says.
   - a **fix**: a rule that translated something wrongly now translates it
     rightly or refuses it, with a test that fails without the fix.

   Translate C constructs, not programs: no program names, expected outputs
   or constants taken from a benchmark. A rule that is wrong for some
   programs asks the analysis and refuses those. Nothing falls back to a
   translation that always works, and the transpiler calls no language model
   or network service.
3. Check it: `cargo fmt`, the sandboxed clippy and tests above, and the
   evaluator. Programs that passed before still pass.
4. Commit on your branch as `feat(transpiler): ...`,
   `refactor(transpiler): ...` or `fix(transpiler): ...`. The gate knows the kind of a step by this.
5. Have it judged: `local/judge`. The gate builds the commit and the last
   kept one in its sandbox and evaluates both on the development set, a
   private validation set and private variants of the development programs.
   It counts strict passes only. It keeps a rule when the development score
   rises, and a refactoring or a fix when it does not drop; in both cases the
   validation score may not drop. It prints the development results and its
   decision. After a rejection, change the commit and judge again, or go back
   with `git reset --hard run/RUN`. One judging is one construct family: a
   rule that passes no development program by itself is rejected, so commit
   the rules of a family one by one and judge them as one. Do not put two
   families into one judging, nor a fix that belongs to neither: the gate
   then cannot tell which of them changed the score.

## A construct family

A rule built for the shape that one development program has passes that
program and little else. Before the first rule of a construct, write its
family into `DESIGN.md` from what C allows, not from what the development
programs use: the positions it takes (a value, a target, a condition, an
argument, a field), its types, its edge values (NULL, the end of an array,
an empty text, the limits of a number), a side effect inside it, and two
names for one memory. Each shape gets a rule or a refusal.

Before a family is judged, write small C programs of your own for its
shapes, with their specs, under `local/probes/FAMILY/`, and run them
through the evaluator (`cargo run -p eval -- local/probes/FAMILY --quiet`).
A probe passes or is refused; what a rule would translate wrongly it
refuses, with a test. Keep the probes, and run all of them again at the end
of a stretch.

## Workers

Rules whose design you have written into `DESIGN.md` go to workers, as do
rules that follow a pattern you have already built, such as ten library
functions: `delegate_task` with Claude Opus 5.5 at medium effort, each in its
own worktree. Give each a brief that stands alone: the rule, the place in the
design, the test it must add, the probes it must write, and the sections
above about the sandbox and the private programs, word for word. Read each
diff yourself, run its probes on your branch, bring what is good onto it, and
have it judged. Decisions about the design stay with you.

## The end of a stretch

A stretch is about fifteen kept commits, or what the owner's message says.
At its end:

1. **Differential testing.** `delegate_task` with GPT 6.1 Sol at high effort,
   read-only: it looks for translations that compile and behave differently
   from the C program, by writing small C programs against the rules of this
   stretch, and reports each with a program that shows it. Call the task
   differential testing of a translator in its brief; a brief that asks for
   an attack was stopped by the provider. Check every finding yourself. Each
   real one becomes a fix with a test, judged like any other step.
2. **Idiom review.** A second read-only review reads the translations of the
   development programs that pass and names the patterns that a Rust
   programmer would not write. They go on the list in `DESIGN.md` of what to
   build next; they are not judged.
3. **Report and stop.** Tell the owner what was kept and rejected, the
   development scores by group, for each family its shapes and how many
   probes pass and are refused, what the workers built, the refusal reasons
   that remain, what the reviews found, and what you would do next. The owner
   has the transcripts audited and reads the validation results before the
   next stretch.

Stop and report before the end of a stretch when the gate fails with an
error, when a step needs a change outside the transpiler, or when you would
need something these instructions forbid.
