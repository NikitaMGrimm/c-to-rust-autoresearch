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
evaluator runs both on the private programs. The transpiler, its build, the
compile of a translation and the Rust program therefore run in a bubblewrap
sandbox (`sandbox`): no network, a read-only file system except the
translation's directory (the workspace for a build), and an empty home
directory apart from the transpiler, its input and the Rust toolchain. The
gate builds from `git archive` of a commit, the `eval` command from a copy of
its checkout's tracked and unignored files, which leaves out `local/` and keeps
symlinks as links. Scratch directories are new and private to the user
(`files::Scratch`).

The transpiler's tests and build scripts are agent code as well. The
workspace's default members leave the transpiler out, so plain `cargo build`,
`cargo clippy` and `cargo test` neither build nor run it; CI, which has no
private data, checks it with `--workspace`.

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

`cargo run -p eval --bin gate -- PRIVATE_DIR BASE CANDIDATE [--usage FILE] [--dev-only] [--run ID]`
decides whether a loop commit stays. It rejects a candidate that does not
descend from `BASE`, changes anything outside `crates/transpiler/` and
`Cargo.lock`, or changes rules without the
[catalogue](../crates/transpiler/CATALOGUE.md). Otherwise it builds both
transpilers from `git archive` in the sandbox, after fetching dependencies
outside it, and evaluates them on the development set, the private validation
and held-out sets and the private variants of the development set.

The keep rule is provisional: the development score rises, or does not drop
for a candidate whose commits are all fixes with a `fix(transpiler)` subject,
such as for a review comment, and the validation score does not drop. With
`--dev-only`, as in the baseline runs, the development score alone decides
and validation is only logged. Every result goes to
`PRIVATE_DIR/loop/log.jsonl`, or with `--run ID` to
`PRIVATE_DIR/runs/ID/log.jsonl`, with the scores of the private sets without [near-copies](../bench/README.md#near-copies) and
the passes that fail their fuzzed inputs. The gate prints only the
development results and the decision. Exit code 0 means keep, 1 reject and 2
error. The gate trusts its own code and the benchmarks, so it must run from a
checkout the loop agent cannot write. It refuses a checkout that is off main,
has changes or untracked files, or is the loop agent's clone; that catches a
mistaken checkout, but the gate is built before it checks, so it cannot stop a
hostile one.

## A round

`loop/round PRIVATE_DIR LOOP_DIR [DENIED_DIR...]` ends one iteration, after the
loop agent's session. It stops if the audit below finds anything or the clone
has no new commit. Otherwise it fetches the clone's `HEAD` into
`refs/loop/TIME` of the main checkout, runs the gate on it with the
session's model usage, and on keep fast-forwards main. It then deletes the
clone and runs `loop/setup` again, so every session starts from a fresh clone
of main. The new clone's `ROUNDS.md` lists every judged commit with what the
gate printed. The script reads the clone only through `git fetch`, because
git run inside it would apply settings and hooks the agent could have
changed. Its state stays in the main checkout's `local/loop/`, one directory
for each loop checkout, and a lock there lets only one round run at a time.

With `--github`, the round then runs `loop/github`, which reaches GitHub
through `gh` and `git` from the main checkout:

- It posts the replies from the old clone's `REPLIES.json`: an answer at once,
  and a fix's reply, with the commit's hash, once the gate kept the fix and
  GitHub has it. Only comments that the last `REVIEWS.md` offered get a reply,
  and its mentions are broken, so that no reply notifies anyone or commands
  CodeRabbit.
- It keeps the published commits as a stack of batches: each batch is a
  branch `loop/TIME` with a pull request into the batch below it, or into
  main for the bottom one. A review comment's fix therefore lands in a later
  batch than the comment, so the step resolves the thread when it posts the
  fix's reply; CodeRabbit reviews the fix in its own batch, and resolves
  threads itself when it accepts an answer.
- It merges the stack from the bottom, by pushing a head to GitHub's main, as
  far as CodeRabbit's check succeeded on every batch and every thread is
  resolved, up to the highest of those heads whose checks all passed: a
  failed check below may be fixed above. The merged batches and the one above
  them first target main, so that GitHub counts them merged and the rest of
  the stack keeps its base when their branches are deleted.
- It publishes what main has beyond the stack as a new batch on top, at once
  when the stack is empty and otherwise at most every two hours. CodeRabbit
  reviews this repository only on request, so the step asks for each new
  batch, and again at most every two hours for the lowest batch it has not
  reviewed, as when it was rate limited.
- It writes `REVIEWS.md` into the new clone: the unresolved threads that
  CodeRabbit or the owner started on any batch, with only their comments, and
  the log tail of each failed check on the top batch, which holds every
  commit. A thread where CodeRabbit answered the agent's reply
  without resolving it goes back to the agent. The step prints for the owner
  instead a thread where CodeRabbit disputed two replies, a thread with more
  comments than it reads, and a check that ended otherwise without passing,
  such as a cancelled run.

The step overwrites neither main nor a `loop/` branch. It publishes and
merges nothing while GitHub's main or a batch of the stack has commits that
main lacks, or a batch lacks the one below it. Commits on a `loop/` branch
that main lacks did not pass the gate. A stack that GitHub's main moved past,
a loop pull request off the stack and more than 50 open pull requests also
wait for the owner. The step's failures only print a warning.

## Keeping the loop agent out

A look at a private program cannot be undone, and the TRACTOR corpus that
holds them is public. Agents rarely go looking on purpose, but one that wants
more examples of a construct might, so the loop has several layers:

- `loop/program.md` tells the agent where the private programs are and why it
  must not look.
- `loop/setup PRIVATE_DIR LOOP_DIR [DENIED_DIR...]` clones main for the agent,
  so its Claude Code settings are its own, and marks the clone so that the
  gate refuses to run from it. Its settings turn auto memory off, skip every
  CLAUDE.md outside the clone and turn the owner's skills off, so the agent
  follows only the repository's instructions and `loop/skills`. They deny
  Claude Code's file tools the private directory, the corpus cache, the main
  checkout, its transcripts and each `DENIED_DIR`, and edits under
  `~/.claude`; deny the web tools, network commands and T3 Code's tools; and
  install `loop/guard`. That hook blocks any tool call, shell commands
  included, that names those places, by path or by their own name, or the
  corpus, or touches Claude Code's files other than the tool output Claude
  Code saves for the agent. It also blocks every MCP tool, since T3 Code's
  reach other threads, a browser and agents in other directories, and every
  skill outside `loop/skills`.
- `cargo run -p transcripts --bin audit -- PRIVATE_DIR PATH...` reads the
  agent's transcripts afterwards. It reports every private program name in
  what the agent saw or wrote, every tool call that named the private
  directory or the corpus, looked for TRACTOR on the web or touched Claude
  Code's files, and every record it cannot read. Exit code 0 means none; a
  path without transcripts is an error.

`intake` fetches the corpus again when its cache is missing, so the cache can
be deleted while a loop runs. The guard and the audit see only what a tool
call names and what the transcript records. A command that reaches a private
program without naming it, such as a search of the whole home directory,
shows only if the program's name reaches the transcript, and nothing catches
what the model learned in pretraining.
