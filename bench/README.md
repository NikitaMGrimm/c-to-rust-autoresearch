# Benchmarks

The programs come from DARPA's
[TRACTOR test corpus](https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus)
at revision `37960ee08a7c644ee66e29855eca33903f702e67`, under the MIT licence in
[LICENSE-TRACTOR](LICENSE-TRACTOR). Only the development set is here. The
validation and held-out sets are in a private repository, so the research
loop never sees them. So is a transfer set from IBM's Project CodeNet, which
shows whether the loop's rules carry over to programs from another source.

Agent-written and hand-written programs are not part of any set for now.

## Programs

`cargo run -p bench --bin intake` converts the standalone programs of batteries
B01 and B02, from both the public tests and the formerly hidden tests:

- `gN_name.c` is TRACTOR's `main.c`, unchanged.
- `gN_name.json` names the source and lists the inputs. Each input holds the
  command-line arguments and standard input of one TRACTOR test vector; for
  a library program, the arguments are the vector's state, one per field.

A library test case becomes a program too when its family has no program
from an executable test case and TRACTOR's Rust runner does nothing but one
plain call: its state holds only numbers, `bool`s and strings, and its `run`
passes fields, or a string's first byte, to the library's function and may
store the result in a field. The program is the library's source with its header inlined and a
generated `main`, marked by a comment, that reads the state from the
command line, one argument per field, makes the call and prints the result
field. An input then also needs the expected result, with floats compared as
TRACTOR's runner does: equal within 1e-6, or within the type's machine
epsilon relative to the larger value. Since the program prints only the
result, TRACTOR must expect every other field to stay as it was.

The evaluator runs the C and the Rust program on every input. An input is kept
only if the sanitized C build gives TRACTOR's expected exit code, and its
expected standard output where the vector gives one, twice in a row. Builds
with `clang-18 -O2` and `gcc -O2` must then print the same and exit alike,
since undefined behaviour that the sanitizers miss often changes the output
of an optimised build. A MemorySanitizer build must also find no read of
uninitialised memory, which an optimised build shows only now and then.

| Not used | Count | Why |
|---|---:|---|
| Library forms of a family that has a program | 46 | They repeat its code |
| Library runners that do more than one plain call | 67 | Their state or call needs more than a generated `main` |
| Several source files | 22 | The transpiler reads one file |
| No usable input left | 6 | |
| Inputs marked as undefined behaviour | 44 | Dropped; the rest of the program stays |
| Inputs with undefined behaviour the sanitizers miss | 2 | `atoi` beyond `int`; listed in the intake tool |
| Inputs with a pattern as expected output | 48 | Dropped; the expected output cannot be checked exactly |
| Inputs in a format the intake tool does not know | 1 | Dropped |

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
| G1 | 10 | 4 | 3 |
| G2 | 9 | 4 | 3 |
| G3 | 11 | 5 | 6 |
| G4 | 3 | 2 | 2 |
| G5 | 17 | 9 | 8 |
| Total | 50 | 24 | 22 |

Most library programs land in G5: libraries use `void *`, `goto` cleanup,
unions and function pointers, and a program's group is the highest feature
anywhere in its file, although its test calls one function.

The private sets, the transfer set included, are fixed by their manifest,
whose SHA-256 is
`633bf00653a6196b5cd99de2ae867f6aed09cd1db0b674649cf4111ffbedcc50`.

TRACTOR's programs are public, so a loop agent with network access could still
find the validation and held-out programs. The loop must run without network.

## Variants

`cargo run -p bench --bin variants -- PRIVATE_DIR` makes three variants of every
program in every set, so that a rule which only memorises a program fails on
them:

- `__r`: every identifier the program declares, including macros, gets a
  made-up name, the same everywhere.
- `__k`: the renamed program with about half of its decimal constants of 2 or
  more changed by a small amount. The generated `main` of a library program
  stays as it is, since a changed argument count would make every input fail
  before the library runs.
- `__i`: the original program on inputs whose numbers are partly replaced by
  others with as many digits. For a program that calls `atoi`, `atol`,
  `atoll` or the `scanf` family, inputs with a number beyond `int` are left
  out, as with fuzzed inputs.

A variant keeps the inputs on which it passes the same C checks as the
benchmarks. Because the evaluator compares the Rust program with the C
program, a variant needs no expected output. All variants are private,
including those of development programs, so the loop agent cannot memorise
them either.

| Set | Variants | No valid constants change | No numbers in inputs |
|---|---:|---:|---:|
| Dev | 127 | 15 | 8 |
| Validation | 61 | 8 | 3 |
| Held-out | 54 | 8 | 4 |

Renamed variants kept every input of their original.

## Fuzzed inputs

TRACTOR gives most programs a few inputs, and a translation can pass all of
them while getting C's behaviour wrong elsewhere.
`cargo run -p bench --bin fuzz -- PRIVATE_DIR` changes each program's inputs at
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
| Dev | 410 | 47 | 50 |
| Validation | 175 | 20 | 24 |
| Held-out | 159 | 20 | 22 |
| Transfer | 1,313 | 190 | 200 |

The evaluator reports them as a separate result for now, so the pass rule
stays as it is. Of the four hand translations of dev programs that pass, two
fail a fuzzed input: one where `strtod` accepts an empty argument, and one
where `scanf("%d")` reads `44Z` as 44.

## Near-copies

Some TRACTOR test cases nearly repeat another under a different name, and the
split by family can put the two on different sides. A rule learned on a
development program then passes its private twin almost for free, which looks
like transfer but is not. `cargo run -p bench --bin near-copies -- PRIVATE_DIR`
compares every private program with every development program by the Jaccard
similarity of their token 4-grams, ignoring comments and the `main` that
intake generates for a library test case. A private program
counts as a near-copy from 0.5; no pair from TRACTOR scores between 0.41 and
0.53. The list stays private, and results for the private sets are reported
with and without near-copies. The gate logs both validation scores; its decision uses
all programs.

| Set | Near-copies | Programs |
|---|---:|---:|
| Validation | 10 | 24 |
| Held-out | 4 | 22 |
| Transfer | 0 | 200 |

## Transfer set

All other sets come from TRACTOR, so a rule that fits TRACTOR's style can
score well on each of them. The transfer set holds 200 programs from
[IBM's Project CodeNet](https://github.com/IBM/Project_CodeNet), solutions
that AIZU's and AtCoder's online judges accepted, under the Community Data
License Agreement - Permissive 2.0. It is private and has no development
part: no loop sees it, and the gate does not evaluate it. It is evaluated
afterwards on the commits a run kept, and reported apart from the TRACTOR
sets.

`cargo run -p bench --bin codenet -- CODENET_DIR PRIVATE_DIR` takes the
problems with accepted C solutions whose sample input and output CodeNet
verified, and merges problems that CodeNet lists as identical into one
family. Families come in an order derived from the secret seed, and each
family's program is the first of at most 10 of its accepted C solutions, in
such an order, that passes the same C checks as the benchmarks on the
sample input and prints CodeNet's expected output as a judge compares it,
without trailing spaces and blank lines. Source and input must be UTF-8, and
a program that calls `atoi`, `atol`, `atoll` or the `scanf` family gets no
input with a number beyond `int`. Groups come from the intake tool's feature
detection, the fuzz tool adds inputs to the sample, and `near-copies`
compares the set with the development programs as well.
`PRIVATE_DIR/codenet.json` records each program's source and why every other
solution tried was left out.

| Group | Programs |
|---|---:|
| G1 | 31 |
| G2 | 98 |
| G3 | 24 |
| G4 | 21 |
| G5 | 26 |
| Total | 200 |

128 programs come from AIZU and 72 from AtCoder. They took 207 of CodeNet's
2,864 families with a verified sample, in seeded order:

| Left out | Count |
|---|---:|
| Solutions that do not build with the default C flags | 47 |
| Solutions whose output differs from CodeNet's expected output | 22 |
| Solutions with a sanitizer error on the sample | 13 |
| Solutions that read uninitialised memory | 3 |
| Solutions that parse a number beyond `int` | 2 |
| Families without a program in 10 solutions | 7 |

## Regenerating

```sh
cargo run -p bench --bin intake -- ~/.cache/c-to-rust-autoresearch/tractor ../c-to-rust-autoresearch-holdout
```

The first argument is where the corpus is fetched. Keep it outside this
repository, because it contains the validation and held-out programs too. The
second argument is a checkout of the private repository. Run `variants` after
`intake`, then `fuzz` (`intake` removes the fuzzed inputs), then update the
manifest as the private README describes, and run `near-copies` whenever a set
changes. The transfer set comes from an unpacked
[Project_CodeNet.tar.gz](https://codait-cos-dax.s3.us.cloud-object-storage.appdomain.cloud/dax-project-codenet/1.0.0/Project_CodeNet.tar.gz),
also kept outside this repository, since it contains the transfer programs:

```sh
cargo run -p bench --bin codenet -- ~/.cache/c-to-rust-autoresearch/codenet/Project_CodeNet ../c-to-rust-autoresearch-holdout
```

Run it before `fuzz`, which gives its programs fuzzed inputs too.
