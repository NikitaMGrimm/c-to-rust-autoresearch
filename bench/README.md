# Benchmarks

The programs come from DARPA's
[TRACTOR test corpus](https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus)
at revision `37960ee08a7c644ee66e29855eca33903f702e67`, under the MIT licence in
[LICENSE-TRACTOR](LICENSE-TRACTOR). Only the development set is here. The
validation and held-out sets are in a private repository, so the research
loop never sees them.

Agent-written and hand-written programs are not part of any set for now.

## Programs

`cargo run -p eval --bin intake` converts the standalone programs of batteries
B01 and B02, from both the public tests and the formerly hidden tests:

- `gN_name.c` is TRACTOR's `main.c`, unchanged.
- `gN_name.json` names the source and lists the inputs. Each input holds the
  command-line arguments and standard input of one TRACTOR test vector.

The evaluator runs the C and the Rust program on every input. An input is kept
only if the sanitized C build gives TRACTOR's expected exit code and standard
output, twice in a row.

| Not used | Count | Why |
|---|---:|---|
| Library test cases | 169 | They are called through a Rust FFI runner, not run as programs |
| Several source files | 14 | The transpiler reads one file |
| Extra compiler flags | 3 | `-fno-strict-aliasing` |
| No usable input left | 3 | |
| Inputs marked as undefined behaviour | 25 | Dropped; the rest of the program stays |
| Inputs with a pattern as expected output | 28 | Dropped; the expected output cannot be checked exactly |

## Difficulty groups

The intake tool assigns each program the highest group whose C features
appear in it, using Clang's syntax tree. This is a heuristic, and the
`signals` field in each JSON file records what it found.

| Group | Features detected |
|---|---|
| G5 | `union`, `goto`, function pointers, `void *` |
| G4 | `malloc`, `calloc`, `realloc`, `free`, `strdup` |
| G3 | `->`, `&`, other pointer variables |
| G2 | arrays, `char *` strings, `string.h` functions |
| G1 | everything else |

Command-line parsing does not count: `main`'s parameters, `char *` variables
in `main`, and `&` passed to `scanf` or `strtol`.

## Split

A family is a test case together with its library form, if one exists. Within
each group, families go to the development, validation and held-out sets in
the ratio 2:1:1, ordered by a secret seed. Families listed in
[inspected.txt](inspected.txt) always go to the development set, because
agents named or read them during earlier feasibility probes. Assignments are
stored and never change; later runs only assign new families.

| Group | Dev | Validation | Held-out |
|---|---:|---:|---:|
| G1 | 7 | 3 | 3 |
| G2 | 7 | 4 | 3 |
| G3 | 7 | 4 | 4 |
| G4 | 1 | 1 | 0 |
| G5 | 2 | 1 | 1 |
| Total | 24 | 13 | 11 |

The private sets are fixed by their manifest, whose SHA-256 is
`8d02c3d62f9db5879f8406801a0fb82b89380a512096a616f2617f28b3fc5aea`.

TRACTOR's programs are public, so a loop agent with network access could still
find the validation and held-out programs. The loop must run without network.

## Regenerating

```sh
cargo run -p eval --bin intake -- ~/.cache/c-to-rust-autoresearch/tractor ../c-to-rust-autoresearch-holdout
```

The first argument is where the corpus is fetched. Keep it outside this
repository, because it contains the validation and held-out programs too. The
second argument is a checkout of the private repository.
