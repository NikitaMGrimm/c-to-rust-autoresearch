# Architecture

The research loop changes the transpiler. Everything else measures it and
stays fixed while a loop runs.

| Crate | Role | Changed by |
|---|---|---|
| `transpiler` | Translates one C file to Rust | the loop |
| `eval` | The evaluator library, the `eval` command and the loop's `gate` | the owner |
| `bench` | Builds the benchmark sets: `intake`, `variants`, `fuzz`, `near-copies` | the owner |
| `transcripts` | Reads Claude Code transcripts: model `usage` and the loop's `audit` | the owner |

## Evaluating a program

`eval::evaluate` takes one program `NAME.c` through these stages and stops at
the first that fails:

1. **Spec.** `NAME.json` lists the inputs, each with command-line arguments
   and standard input, and may list `c_flags` for the C build from a fixed
   set. Without it the program runs once without input (`spec`).
2. **C reference.** The C program is built with AddressSanitizer and
   UndefinedBehaviorSanitizer and run on every input. An invalid spec, a build
   failure, a sanitizer error, a timeout or too much output makes it
   `c_invalid` (`reference`).
3. **Translation.** The transpiler writes `translation.rs` within 30 s
   (`transpile_fail`), or `--translations` copies an existing one
   (`translate`).
4. **Compile.** `clippy-driver` compiles it with overflow checks and debug
   assertions (`compile_fail`). Warnings are counted by lint, and `unsafe_code`
   is forced on so an `allow` cannot hide it.
5. **Runs.** The Rust program runs on every input within 10 s, without a panic
   (`timeout`, `panic`), and must print the same standard output and exit with
   the same code as the C program (`mismatch`). Standard error is not
   compared.
6. **Safety.** A translation with `unsafe` code that behaves correctly is
   `unsafe`; otherwise it is a `pass`.

A pass then runs the fuzzed inputs in `NAME.fuzz.json`, if there are any, the
same way. That result is reported separately and does not change the stage.

The stage names are those of `eval::Stage`. For a translation that compiled,
the record also holds idiomaticity counts from its syntax tree (`idioms`):
`clones`, `raw_pointers`, `rc_refcell`, `lint_allows` (`allow` and `expect`
attributes, which hide warnings) and non-blank `lines`.

Both programs run as `./program` from their own directories, so they see the
same `argv[0]`. Every program may write files of at most 64 MiB and print at
most 8 MiB; the transpiler and the Rust program may use 4 GiB of address
space, and the sanitized C program 4 GiB of memory (`run`).

## The sandbox

The research agent writes the transpiler and so every translation, and the
evaluator runs both on the private programs. The transpiler, its build in the
gate, the compile of a translation and the Rust program therefore run in a
bubblewrap sandbox (`sandbox`): no network, a read-only file system except the
translation's directory (the unpacked workspace for the gate's build), and an
empty home directory apart from the transpiler, its input and the Rust
toolchain. Scratch directories are new and
private to the user (`files::Scratch`).

## Benchmark data

```text
TRACTOR corpus ──intake──> bench/dev            (public)
                      └──> validation, held-out (private)
each set ──variants──> PRIVATE_DIR/variants/SET
each set ──fuzz──────> NAME.fuzz.json next to each program
private sets ──near-copies──> PRIVATE_DIR/near-copies.json
```

[bench/README.md](../bench/README.md) describes each step, the split and the
counts. The tools derive everything from a fixed seed, so a rerun writes the
same files, and the private sets are fixed by a manifest whose hash is in
bench/README.md. Tools that read private programs print names and counts only.

## The gate

`cargo run -p eval --bin gate -- PRIVATE_DIR BASE CANDIDATE [--usage FILE]`
decides whether a loop commit stays. It rejects a candidate that does not
descend from `BASE`, changes anything outside `crates/transpiler/` and
`Cargo.lock`, or changes rules without the
[catalogue](../crates/transpiler/CATALOGUE.md). Otherwise it builds both
transpilers from `git archive` in the sandbox, after fetching dependencies
outside it, and evaluates them on the development set, the private validation
set and the private variants of the development set.

The keep rule is provisional: the development score rises and the validation
score does not drop. Every result goes to `PRIVATE_DIR/loop/log.jsonl`, with
the validation score without [near-copies](../bench/README.md#near-copies) and
the passes that fail their fuzzed inputs. The gate prints only the
development results and the decision. Exit code 0 means keep, 1 reject and 2
error. The gate trusts its own code and the benchmarks, so it refuses to run
from anything but a clean checkout of main.

## Keeping the loop agent out

A look at a private program cannot be undone, and the TRACTOR corpus that
holds them is public. Agents rarely go looking on purpose, but one that wants
more examples of a construct might, so the loop has several layers:

- `loop/program.md` tells the agent where the private programs are and why it
  must not look.
- `loop/setup PRIVATE_DIR LOOP_DIR [DENIED_DIR...]` clones main for the agent,
  so its Claude Code settings and memory are its own. Its settings deny
  Claude Code's file tools the private directory, the corpus cache, the main
  checkout's `local/` and transcripts, and each `DENIED_DIR`; deny fetches
  from GitHub and network commands; and install `loop/guard`. That hook blocks
  any tool call, shell commands included, that names those places or the
  corpus, or changes Claude Code settings.
- `cargo run -p transcripts --bin audit -- PRIVATE_DIR PATH...` reads the
  agent's transcripts afterwards. It reports every private program name the
  agent saw or wrote, and every tool call that named the private directory or
  the corpus, looked for TRACTOR on the web, or touched Claude Code settings.
  Exit code 0 means none.

`intake` fetches the corpus again when its cache is missing, so the cache can
be deleted while a loop runs. Only the audit catches a command that reaches a
private program without naming it, such as a search of the whole home
directory, and nothing catches what the model learned in pretraining.
