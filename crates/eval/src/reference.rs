//! C reference builds and runs.

use std::path::{Path, PathBuf};

use crate::run::{COMPILE_TIMEOUT, OUTPUT_LIMIT, Run, limited, run, run_program, tail};
use crate::spec::Input;

/// Builds `c_file` with AddressSanitizer and UndefinedBehaviorSanitizer.
pub(crate) fn build_c(c_file: &Path, program: &Path, c_flags: &[String]) -> Run {
    run(
        limited(COMPILE_TIMEOUT, false)
            .arg("clang-18")
            .args([
                "-g",
                "-fsanitize=address,undefined",
                "-fno-sanitize-recover=all",
            ])
            .args(c_flags)
            .arg("-o")
            .arg(program)
            .arg(c_file)
            .arg("-lm"),
        &[],
    )
}

/// Builds of a C program that must agree before its runs serve as reference.
/// Optimised builds without sanitizers often expose undefined behaviour that
/// the sanitizers miss, by printing something else. Reads of uninitialised
/// memory do so only now and then, so MemorySanitizer looks for them.
pub struct References {
    sanitized: PathBuf,
    memory: PathBuf,
    optimised: Vec<PathBuf>,
}

impl References {
    /// Builds `c_file` sanitized, with MemorySanitizer, and with
    /// `clang-18 -O2` and `gcc -O2`, in subdirectories of `work`.
    pub fn build(c_file: &Path, c_flags: &[String], work: &Path) -> Result<Self, String> {
        let mut programs = Vec::new();
        for (name, command) in [
            ("sanitized", None),
            (
                "memory",
                Some(
                    [
                        "clang-18",
                        "-g",
                        "-O1",
                        "-fsanitize=memory",
                        "-fno-sanitize-recover=all",
                        "-w",
                    ]
                    .as_slice(),
                ),
            ),
            ("clang_O2", Some(["clang-18", "-O2", "-w"].as_slice())),
            ("gcc_O2", Some(["gcc", "-O2", "-w"].as_slice())),
        ] {
            let dir = work.join(name);
            std::fs::create_dir_all(&dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
            let program = dir.join("program");
            let built = match command {
                None => build_c(c_file, &program, c_flags),
                Some(command) => run(
                    limited(COMPILE_TIMEOUT, false)
                        .args(command)
                        .args(c_flags)
                        .arg("-o")
                        .arg(&program)
                        .arg(c_file)
                        .arg("-lm"),
                    &[],
                ),
            };
            if built.exit != Some(0) {
                return Err(format!("c_build: {name}"));
            }
            programs.push(program);
        }
        let sanitized = programs.remove(0);
        let memory = programs.remove(0);
        Ok(Self {
            sanitized,
            memory,
            optimised: programs,
        })
    }

    /// The sanitized run of `input`, if it is valid, the same twice, free of
    /// uninitialised reads, and the same as the optimised builds' runs;
    /// otherwise why not.
    pub fn check(&self, input: &Input) -> Result<Run, &'static str> {
        let first = run_program(&self.sanitized, input);
        if first.timed_out() {
            return Err("c_timeout");
        }
        if first.sanitizer_error() {
            return Err("c_sanitizer_error");
        }
        if c_problem(&first).is_some() {
            return Err("c_unusable");
        }
        if run_program(&self.sanitized, input) != first {
            return Err("c_nondeterministic");
        }
        // Only the report counts: what this build prints may change from run
        // to run where the program reads outside its objects.
        let memory = run_program(&self.memory, input);
        if String::from_utf8_lossy(&memory.stderr).contains("WARNING: MemorySanitizer:") {
            return Err("c_uninitialised_read");
        }
        for program in &self.optimised {
            let other = run_program(program, input);
            if (&other.stdout, other.exit) != (&first.stdout, first.exit) {
                return Err("c_builds_disagree");
            }
        }
        Ok(first)
    }
}

/// Why a C run is not a valid reference, if it is not.
pub(crate) fn c_problem(c: &Run) -> Option<String> {
    if c.exit.is_none() {
        // For example an argument containing a NUL byte; the Rust side would fail alike.
        Some(format!("cannot run:\n{}", tail(&c.stderr)))
    } else if c.overflowed {
        Some(format!("C output exceeds {OUTPUT_LIMIT} bytes"))
    } else if c.timed_out() {
        Some("C program timed out".to_owned())
    } else if c.sanitizer_error() {
        Some(format!("sanitizer error:\n{}", tail(&c.stderr)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_of_uninitialised_memory_is_no_reference() {
        let work = crate::Scratch::new("reference").unwrap();
        let input = Input {
            name: "t".to_owned(),
            args: Vec::new(),
            stdin: Vec::new(),
        };
        // AddressSanitizer fills new memory with a pattern and glibc's malloc
        // hands out zeroed pages, so every other build prints 1 here.
        for (name, read, expected) in [
            ("unset", "*x != 12345", Some("c_uninitialised_read")),
            ("set", "(*x = 7) != 12345", None),
        ] {
            let c_file = work.join(format!("{name}.c"));
            let source = format!(
                "#include <stdio.h>\n#include <stdlib.h>\n\nint main(void) {{\n    int *x = malloc(sizeof *x);\n    printf(\"%d\\n\", {read});\n    free(x);\n    return 0;\n}}\n"
            );
            std::fs::write(&c_file, source).unwrap();
            let references = References::build(&c_file, &[], &work.join(name)).unwrap();
            assert_eq!(references.check(&input).err(), expected, "{name}");
        }
    }
}
