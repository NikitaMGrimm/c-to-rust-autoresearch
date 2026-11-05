# c-to-rust-autoresearch

Bachelor thesis: improving a mechanical C-to-Rust translator through an LLM-driven research loop.

The transpiler starts empty; translation rules are added by the research loop.
[docs/architecture.md](docs/architecture.md) explains how a translation is
evaluated and a loop commit judged.

## Setup

Linux only. Needs [rustup](https://rustup.rs) and, on Ubuntu 24.04:

```sh
sudo apt install clang-18 clang-tidy-18 libclang-rt-18-dev llvm-18 jq bubblewrap
```

The evaluator runs agent-written code with bubblewrap, which needs
unprivileged user namespaces. Ubuntu 24.04 restricts them by default, but WSL
does not; lift the restriction with
`sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0`.

`rust-toolchain.toml` pins the Rust version, and the transpiler always calls
`clang-18`, so results stay comparable between runs.

## Use

```sh
cargo run -p transpiler -- input.c -o output.rs
scripts/inspect input.c      # declarations, Clang warnings, clang-tidy, sanitizer run
scripts/inspect output.rs    # rustc and Clippy warnings, unsafe code, rustfmt, run
```

`scripts/inspect` passes further arguments to the program and exits 1 when a
tool reports an error.

## Evaluation

```sh
cargo run -p eval                          # all programs in bench/dev
cargo run -p eval -- --only g1 --json results.json
cargo run -p eval -- --translations DIR   # existing translations DIR/NAME.rs
```

The evaluator builds the transpiler and compares every `.c` program in the
benchmark directory with its translation. A translation passes when it
compiles as safe Rust and, with overflow checks on, prints the same standard
output and exits with the same code as the C program on every input. Program
names start with their difficulty group, for example `g1_gcd.c`.

It prints one stage per program, details for failures, pass counts per group
and `SCORE passed/total`. For a pass with fuzzed inputs it adds
`fuzz: STAGE`, and the last line, `FUZZ held/fuzzed`, counts the passes with
fuzzed inputs and how many of them also pass those. Fuzzed inputs do not
change whether a translation passes. `--json` also writes tool versions, the
commit, and per-program results with warning counts by lint and idiomaticity
counts.

## Benchmarks

[bench/](bench/README.md) holds the development set, converted from DARPA's
TRACTOR test corpus. The validation and held-out sets are private.

## Research loop

The loop agent follows [loop/program.md](loop/program.md): one translation rule
per commit, each listed in the
[rule catalogue](crates/transpiler/CATALOGUE.md). The gate decides whether a
commit stays:

```sh
cargo run -p eval --bin gate -- PRIVATE_DIR BASE CANDIDATE [--usage FILE]
```

It keeps a candidate that changes only the transpiler when the development
score rises, or for a candidate of `fix(transpiler)` commits does not drop,
and the validation score does not drop, and prints only the development
results and the decision. Run it from this checkout, which the
loop agent cannot write, at a clean main.

The loop agent works in its own clone, which keeps it away from the private
programs. After each of its sessions, `loop/round` audits the session's
transcripts, runs the gate on its commit, keeps the commit on main or drops
it, and makes a fresh clone for the next session:

```sh
loop/setup PRIVATE_DIR LOOP_DIR [DENIED_DIR...]
~/src/c-to-rust-autoresearch/loop/round [--github] PRIVATE_DIR LOOP_DIR [DENIED_DIR...]
```

Run `loop/round` by this checkout's absolute path, never the clone's copy. In
T3 Code, the clone is a project whose threads run in the clone itself, not in
worktrees, on Claude only, with `loop/round` as a project script.
[docs/architecture.md](docs/architecture.md#keeping-the-loop-agent-out) says
what the scripts do; the audit runs on its own too:

```sh
cargo run -p transcripts --bin audit -- PRIVATE_DIR ~/.claude/projects/LOOP_PROJECT
```

With `--github`, CI and CodeRabbit check the loop's commits as they would a
contributor's. The round publishes main to a pull request from the `loop`
branch, at most every two hours, and merges it by fast-forward once its
checks pass and CodeRabbit has reviewed it with every thread resolved. Open
review comments and failed checks come back to the agent in the next clone,
and the round posts the agent's replies; the agent itself never reaches
GitHub. When CodeRabbit disputes a reply, the thread goes back to the agent;
after the second dispute it waits for the owner. The round prints what waits
for the owner: such threads, a check that ended without failing or passing,
and commits that reach GitHub's `main` or `loop` some other way. Pull those
on `main` into this checkout's main between rounds, then make the clone
afresh with `loop/setup`.

## Model usage

```sh
cargo run -p transcripts --bin usage -- ~/.claude/projects/-home-$USER-src-c-to-rust-autoresearch
cargo run -p transcripts --bin usage -- --since 2026-11-04T20:00:00Z --until 2026-11-05T00:00:00Z DIR
```

`usage` sums the tokens that Claude Code records in its session transcripts,
per model and per session, and counts requests by reasoning effort. T3 Code
runs Claude through Claude Code, so its Claude chats are included; Codex runs
are not. Claude Code names the transcript directory after the working
directory, so a worktree has its own directory. `--since` and `--until` limit
the sums to one loop iteration or another time window.
