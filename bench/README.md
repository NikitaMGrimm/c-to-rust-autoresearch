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
`a03d8f01cf81045cd8ba4496d4c00518138bb9256ab18993305217e063d3a44a`.

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

## Fuzzed inputs

TRACTOR gives most programs a few inputs, and a translation can pass all of
them while getting C's behaviour wrong elsewhere.
`cargo run -p eval --bin fuzz -- PRIVATE_DIR` changes each program's inputs at
random: numbers become edge values such as `2147483648`, neighbours or other
digits; arguments and lines of standard input are dropped, repeated or cut
off; single characters are deleted, replaced or inserted. A candidate is kept
when it passes the same C checks as the benchmarks and the C program prints or
exits unlike on every input kept before, up to 10 per program. Programs that
call `atoi`, `atol`, `atoll` or the `scanf` family only get numbers that fit
an `int`: these functions have undefined behaviour on larger numbers, which
the sanitizers do not catch. Development programs get `NAME.fuzz.json` here;
the private sets keep theirs. The seed is fixed, so a rerun writes the same
files.

| Set | Fuzzed inputs | Programs with them | Programs |
|---|---:|---:|---:|
| Dev | 198 | 21 | 24 |
| Validation | 100 | 10 | 14 |
| Held-out | 95 | 11 | 13 |

The evaluator reports them as a separate result for now, so the pass rule
stays as it is. Of the four hand translations of dev programs that pass, two
fail a fuzzed input: one where `strtod` accepts an empty argument, and one
where `scanf("%d")` reads `44Z` as 44.

## Near-copies

Some TRACTOR test cases nearly repeat another under a different name, and the
split by family can put the two on different sides. A rule learned on a
development program then passes its private twin almost for free, which looks
like transfer but is not. `cargo run -p eval --bin near-copies -- PRIVATE_DIR`
compares every private program with every development program by the Jaccard
similarity of their token 4-grams, ignoring comments. A private program
counts as a near-copy from 0.5; no pair scores between 0.41 and 0.53. The
list stays private, and results for the private sets are reported with and
without near-copies. The gate logs both validation scores; its decision uses
all programs.

| Set | Near-copies | Programs |
|---|---:|---:|
| Validation | 4 | 14 |
| Held-out | 2 | 13 |

## Regenerating

```sh
cargo run -p eval --bin intake -- ~/.cache/c-to-rust-autoresearch/tractor ../c-to-rust-autoresearch-holdout
```

The first argument is where the corpus is fetched. Keep it outside this
repository, because it contains the validation and held-out programs too. The
second argument is a checkout of the private repository. Run `variants` after
`intake`, then `fuzz` (`intake` removes the fuzzed inputs), then update the
manifest as the private README describes, and run `near-copies` whenever a set
changes.
