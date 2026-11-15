//! Each test translates a small C program, runs it beside its translation
//! and checks the Rust that a rule should give.

mod common;
mod compare;
mod copies;
mod cursors;
<<<<<<< HEAD
mod environment;
=======
mod functions;
>>>>>>> 498e22d (feat(transpiler): hold a pointer to a function as an fn value)
mod globals;
mod heap;
mod input;
mod output;
mod pointers;
mod records;
mod refusals;
mod scalars;
mod search;
mod text;
mod unions;
