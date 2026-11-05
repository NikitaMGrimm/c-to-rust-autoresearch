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
only if the sanitized C build gives TRACTOR's expected exit code, and its
expected standard output where the vector gives one, twice in a row. Builds
with `clang-18 -O2` and `gcc -O2` must then print the same and exit alike,
since undefined behaviour that the sanitizers miss often changes the output
of an optimised build.

| Not used | Count | Why |
|---|---:|---|
| Library test cases | 169 | They are called through a Rust FFI runner, not run as programs |
| Several source files | 14 | The transpiler reads one file |
| No usable input left | 3 | |
| Inputs marked as undefined behaviour | 25 | Dropped; the rest of the program stays |
| Inputs with undefined behaviour the sanitizers miss | 2 | `atoi` beyond `int`; listed in the intake tool |
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
stored and never change; later runs only assign new families. Three families
build their program with `-fno-strict-aliasing` but otherwise repeat another
family's program; each was assigned by hand to that family's set, so the twins
share a set.

| Group | Dev | Validation | Held-out |
|---|---:|---:|---:|
| G1 | 7 | 3 | 3 |
| G2 | 7 | 4 | 3 |
| G3 | 8 | 5 | 6 |
| G4 | 1 | 1 | 0 |
| G5 | 1 | 1 | 1 |
| Total | 24 | 14 | 13 |

The private sets are fixed by their manifest, whose SHA-256 is
`b7af4d493d0e2ccb4cc89a5dea651b3b0419f7b18f3a1c9c7c80c75ac40480a2`.

TRACTOR's programs are public, so a loop agent with network access could still
find the validation and held-out programs. The loop must run without network.

## Variants

`cargo run -p eval --bin variants -- PRIVATE_DIR` makes three variants of every
program in every set, so that a rule which only memorises a program fails on
them:

- `__r`: every identifier the program declares, including macros, gets a
  made-up name, the same everywhere.
- `__k`: the renamed program with about half of its decimal constants of 2 or
  more changed by a small amount.
- `__i`: the original program on inputs whose numbers are partly replaced by
  others with as many digits.

A variant keeps the inputs on which it passes the same C checks as the
benchmarks. Because the evaluator compares the Rust program with the C
program, a variant needs no expected output. All variants are private,
including those of development programs, so the loop agent cannot memorise
them either.

| Set | Variants | No valid constants change | No numbers in inputs |
|---|---:|---:|---:|
| Dev | 60 | 5 | 7 |
| Validation | 37 | 2 | 3 |
| Held-out | 28 | 7 | 4 |

Renamed variants kept every input of their original.

## Regenerating

```sh
cargo run -p eval --bin intake -- ~/.cache/c-to-rust-autoresearch/tractor ../c-to-rust-autoresearch-holdout
```

The first argument is where the corpus is fetched. Keep it outside this
repository, because it contains the validation and held-out programs too. The
second argument is a checkout of the private repository. Run `variants` after
`intake`, then update the manifest as the private README describes.
