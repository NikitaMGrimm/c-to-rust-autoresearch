# Agent instructions

This repository holds a bachelor thesis project: a mechanical C-to-Rust
transpiler that an LLM agent improves in a research loop, and the evaluator,
benchmarks and gate that measure it. Rust is the implementation language. The
priorities are fixed, in this order:

1. **Keep the private sets unseen.** Validation and held-out results only
   count while no agent has seen those programs.
2. **Keep the measurement fixed.** The evaluator, the benchmarks and the gate
   define progress; a change to them changes every result after it.
3. **Keep agent-written code in the sandbox.** It runs on the private
   programs, so nothing it sees may leave.
4. Features come after all three.

The loop agent follows [loop/program.md](loop/program.md), and a driver
[loop/driver.md](loop/driver.md); both limit it to the transpiler. You are
the driver when the owner's message tells you to drive, such as "drive", in
a checkout that has `local/judge`. The smallest change that does the job is
the right one, and deleting code is welcome. A new dependency needs a reason.

## Words

- **dev**, **validation**, **held-out**: the benchmark sets. Dev is public in
  `bench/dev`; the others are in the private holdout repository, the
  **private directory** (`PRIVATE_DIR`).
- **program**: a benchmark `NAME.c` with its **spec** `NAME.json` (inputs and
  C flags) and maybe **fuzzed inputs** `NAME.fuzz.json`. The name starts with
  its difficulty **group**, `g1` to `g5`.
- **reference**: the C program's run on an input, which a translation must
  match. The bench tools keep an input only when its reference passes stricter
  checks (`eval::References`).
- **stage**: how far a translation got (`eval::Stage`); only `pass` succeeds.
- **transfer**: a fourth private set, from IBM's Project CodeNet rather than
  TRACTOR, that no loop sees or is judged on.
- **variant**: a private copy of a program with renamed identifiers, changed
  constants or new inputs. **near-copy**: a private program that nearly
  repeats a dev program.
- **rule**: one translation rule of the transpiler, with an entry in the
  **catalogue**, `crates/transpiler/CATALOGUE.md`.
- **gate**: decides whether a loop commit is kept or rejected.

## Ways to hurt yourself

1. **Opening a private program.** Never read, copy or reference validation,
   held-out or transfer programs, their variants or their fuzzed inputs, and
   never open what `loop/seal` closed while work on the transpiler goes on. They
   live in a separate private repository on purpose, and seeing them
   invalidates the results. The TRACTOR corpus that `intake` fetches and the
   CodeNet archive that `codenet` reads contain them as well; open only
   programs in `bench/dev`. Tools that read the private sets print names and
   counts only, and so must your own commands.
2. **Publishing private data.** This repository is public. Do not add
   credentials, private notes, thesis drafts, supervisor correspondence or
   anything from the private holdout repository. Keep unpublished benchmarks
   and outputs in the ignored `local/` directory.
3. **Running agent code outside the sandbox.** The transpiler, its build in
   the gate, the compile of a translation and the translated program all run
   through `sandbox::sandboxed`, the sessions of an unattended run, which
   run agent code as well, through `loop/run`'s sandbox, and a driver's
   builds and tests of the transpiler through `loop/cargo`. A new way to run any
   of them must be sandboxed too, with a test that fails without the sandbox.
4. **Changing the measurement on the side.** Pass rules, limits, inputs and
   the keep rule change every result after them. The bench tools must write
   the same files from the same seed, and a change to the private sets needs
   a new manifest hash in `bench/README.md`.
5. **Trusting C behaviour the sanitizers miss.** The bench tools keep an
   input only when the sanitized C program runs the same twice, reads no
   uninitialised memory under MemorySanitizer and agrees with `clang -O2` and
   `gcc -O2`. Generated inputs also avoid undefined behaviour
   these checks miss, such as numbers beyond `int` for `atoi`.

## Layout

| Path | Role |
| --- | --- |
| `crates/transpiler/` | The transpiler, the only code the loop changes |
| `crates/eval/src/` | Evaluator library: `spec`, `reference`, `translate`, `run`, `sandbox`, `evaluate`, `idioms`, `files`, and `commits` for the gate and `transfer` |
| `crates/eval/src/main.rs`, `src/bin/gate.rs`, `src/bin/transfer.rs` | The `eval` command, the loop's gate, and the transfer set's evaluation after a run |
| `crates/bench/` | Builds the sets: `intake`, `codenet`, `variants`, `fuzz`, `near-copies` |
| `crates/transcripts/` | Reads Claude Code transcripts: model `usage` and the loop's `audit` |
| `bench/` | The dev programs, and how all sets were made |
| `loop/` | Instructions for the loop agent and for a driver, the setup of their checkouts, the seal, the judging of a commit, the GitHub step and unattended runs |
| `scripts/inspect` | A C or Rust file's warnings, lints and run result |
| `docs/architecture.md` | How a program is evaluated and a commit judged |

## Checks

Before committing Rust changes, run:

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests need the C tools and bubblewrap from the README. If you could not
run a check, say so. These commands leave out the transpiler: it is agent
code, so on a machine with the private directory never build, test or run it
outside the sandbox, such as with `--workspace` or `-p transpiler`; `loop/cargo`
runs cargo for it in a sandbox. CI checks
it. In the loop agent's own clone, `loop/program.md` says what to run. For an evaluator change, compare the output of
`cargo run -p eval -- --json FILE` before and after; for a bench tool change,
rerun it and check that both repositories have no changes you did not intend.

## Conventions

- Write code that reads like the code around it. Comments explain why, not
  what. Every bug fix comes with a test that fails without it.
- Update the README, `bench/README.md` or `docs/` when behaviour changes. They
  say how things work; the reasons behind decisions are kept elsewhere.
- Use Conventional Commits, for example `feat(eval): ...`, and LF line endings.
  Avoid unrelated formatting changes. Use one branch per task.
- Pull requests follow the template. When AI helped, end the description with
  a short note naming the actual models, harnesses and roles.
- Do not change visibility, collaborators or licensing without the owner's
  authorization.
