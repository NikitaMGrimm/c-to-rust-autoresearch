# Agent instructions

This repository holds a bachelor thesis project: a mechanical C-to-Rust
transpiler that an LLM agent improves in a research loop. Rust is the
implementation language.

## Rules

- This repository is public. Do not add credentials, private notes, thesis
  drafts, supervisor correspondence or anything from the private holdout
  repository.
- Never read, copy or reference validation or held-out programs. They live
  in a separate private repository on purpose, and seeing them invalidates
  the results.
- Never weaken a test or the evaluator so that a change passes. If a check
  fails, the change is wrong, not the check.
- Make the smallest change that does the job. A new dependency needs a
  reason.
- Write code that reads like the code around it. Comments explain why, not
  what. Every bug fix comes with a test that fails without it.
- Use Conventional Commits, for example `feat(eval): ...`, and LF line endings.
  Avoid unrelated formatting changes. Use one branch per task.
- Before committing Rust changes, run `cargo fmt`,
  `cargo clippy --all-targets -- -D warnings` and `cargo test`. If you could
  not run a check, say so.
- When AI helped with a pull request, end its description with a short note
  naming the actual models, harnesses and roles.
- Do not change visibility, collaborators or licensing without the owner's
  authorization.
