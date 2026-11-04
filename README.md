# c-to-rust-autoresearch

Bachelor thesis: improving a mechanical C-to-Rust translator through an LLM-driven research loop.

The transpiler starts empty; translation rules are added by the research loop.

## Setup

Linux only. Needs [rustup](https://rustup.rs) and, on Ubuntu 24.04:

```sh
sudo apt install clang-18 clang-tidy-18 libclang-rt-18-dev llvm-18 jq
```

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

The evaluator builds the transpiler, then for every `.c` program in the
benchmark directory compares the C original with its translation. A `.json`
file with the same name lists the inputs, each with command-line arguments and
standard input; without it the program runs once without input. It may also
list `c_flags` for the C build, from a fixed set. Program names start with
their difficulty group, for example `g1_gcd.c`. It prints one stage per
program, details for failures, pass counts per group and the total.

`--json` also writes tool versions, the commit, and per-program results. Each
result has warning counts by lint and, for a translation that compiled,
idiomaticity counts from its syntax tree: `clones`, `raw_pointers`,
`rc_refcell`, `lint_allows` (`allow` and `expect` attributes, which hide
warnings) and non-blank `lines`.

A translation passes when:

- the C original builds and runs on every input without an AddressSanitizer
  or UndefinedBehaviorSanitizer error within 10 s (otherwise `c_invalid`);
- the transpiler writes a Rust file within 30 s (`transpile_fail`);
- the Rust file compiles with overflow checks and debug assertions
  (`compile_fail`);
- the Rust program finishes within 10 s without a panic (`timeout`, `panic`);
- its standard output and exit code equal the C program's on every input
  (`mismatch`);
- it contains no `unsafe` code (`unsafe`).

Standard error is not compared. Both programs run as `./program` from their
own directories. Every program may write files of at most 64 MiB and print at
most 8 MiB; the transpiler and the Rust program may use 4 GiB of address
space, and the sanitized C program 4 GiB of memory.

## Benchmarks

[bench/](bench/README.md) holds the development set, converted from DARPA's
TRACTOR test corpus. The validation and held-out sets are private.

## Model usage

```sh
cargo run -p eval --bin usage -- ~/.claude/projects/-home-$USER-src-c-to-rust-autoresearch
cargo run -p eval --bin usage -- --since 2026-11-04T20:00:00Z --until 2026-11-05T00:00:00Z DIR
```

`usage` sums the tokens that Claude Code records in its session transcripts,
per model and per session, and counts requests by reasoning effort. T3 Code
runs Claude through Claude Code, so its Claude chats are included; Codex runs
are not. Claude Code names the transcript directory after the working
directory, so a worktree has its own directory. `--since` and `--until` limit
the sums to one loop iteration or another time window.
