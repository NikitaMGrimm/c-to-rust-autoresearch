//! Differential evaluator. A benchmark passes when its translation compiles as
//! safe Rust and, with overflow checks on, prints the same standard output and
//! exits with the same code as the C original for every input.

mod evaluate;
mod files;
mod idioms;
mod reference;
mod run;
mod sandbox;
mod spec;
mod translate;

pub use evaluate::{Record, Stage, evaluate, evaluate_all, fuzz_counts, versions};
pub use files::{REPO, Scratch, c_programs, dev_dir, entries};
pub use reference::References;
pub use run::{Run, run_program};
pub use spec::{C_FLAGS, FUZZ_EXTENSION, Input, Spec, spec};
pub use translate::{Translator, build_transpiler, build_transpiler_sandboxed};
