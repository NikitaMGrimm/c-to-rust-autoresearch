//! Differential evaluator. A benchmark passes when its translation compiles as
//! safe Rust and, with overflow checks on, prints the same standard output and
//! exits with the same code as the C original for every input.

mod evaluate;
mod idioms;
mod reference;
mod run;
mod sandbox;
mod spec;
mod translate;

pub use evaluate::{Record, c_programs, classify, evaluate, evaluate_all, fuzz_counts, versions};
pub use idioms::idiom_counts;
pub use reference::{References, build_c, c_problem};
pub use run::{Run, run_program};
pub use spec::{C_FLAGS, FUZZ_EXTENSION, Input, Spec, fuzzed_inputs, spec};
pub use translate::{Translator, build_transpiler, build_transpiler_sandboxed};

/// Directory whose rust-toolchain.toml pins rustc and Clippy.
pub const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
