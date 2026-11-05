//! Running a program under time, file size and memory limits, and what it
//! printed.

use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::sandbox::sandboxed;
use crate::spec::Input;

const RUN_TIMEOUT: &str = "10";

pub(crate) const COMPILE_TIMEOUT: &str = "60";

/// Exit codes of coreutils `timeout` when the limit is reached, and when the
/// command ignored SIGTERM and had to be killed.
const TIMEOUT_EXITS: [i32; 2] = [124, 137];

/// Set for sanitizer reports so they differ from the program's own exit codes.
const SANITIZER_EXIT: i32 = 99;

pub(crate) const DETAIL_LIMIT: usize = 2000;

/// Bytes kept of a program's standard output and error; more is a failure.
pub(crate) const OUTPUT_LIMIT: u64 = 8 << 20;

/// Address space of the transpiler and translated programs. The C reference
/// has none: AddressSanitizer reserves terabytes of address space.
const MEMORY_LIMIT: &str = "--as=4294967296";

/// Largest file any program may write.
const FILE_LIMIT: &str = "--fsize=67108864";

/// What a program printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: Option<i32>,
    /// Whether the program wrote more than the output limit.
    pub overflowed: bool,
}

impl Run {
    pub fn timed_out(&self) -> bool {
        self.exit.is_some_and(|code| TIMEOUT_EXITS.contains(&code))
    }

    /// Whether AddressSanitizer or UndefinedBehaviorSanitizer reported an error.
    pub fn sanitizer_error(&self) -> bool {
        // A program may exit with SANITIZER_EXIT itself; a report also prints one of these.
        let stderr = String::from_utf8_lossy(&self.stderr);
        self.exit == Some(SANITIZER_EXIT)
            && (stderr.contains("ERROR: AddressSanitizer") || stderr.contains("runtime error:"))
    }
}

/// Runs the C program `program` on `input` within the run time limit.
pub fn run_program(program: &Path, input: &Input) -> Run {
    run_in_place(program, input, false)
}

/// Runs `program` from its own directory as `./NAME`, so C and Rust programs
/// with the same file name see the same `argv[0]` and write files apart.
pub(crate) fn run_in_place(program: &Path, input: &Input, memory_limit: bool) -> Run {
    let name = Path::new(".").join(program.file_name().unwrap_or_default());
    let dir = program.parent().unwrap_or(Path::new("."));
    let mut command = limited(RUN_TIMEOUT, memory_limit);
    command.arg(name).args(&input.args).current_dir(dir);
    // Only translations get the memory limit, and only they are agent-written.
    if memory_limit {
        command = sandboxed(&command, &[], dir);
    }
    run(&mut command, &input.stdin)
}

/// A command that runs its arguments for at most `seconds`, killing them two
/// seconds later if they ignore SIGTERM, with a file size limit and, if asked,
/// a memory limit.
pub(crate) fn limited(seconds: &str, memory_limit: bool) -> Command {
    let mut command = Command::new("prlimit");
    command.arg(FILE_LIMIT);
    if memory_limit {
        command.arg(MEMORY_LIMIT);
    }
    command.args(["--", "timeout", "--kill-after=2", seconds]);
    command
}

/// Reads up to the output limit, then discards the rest so the program never
/// blocks on a full pipe. Returns whether anything was discarded.
fn capped(pipe: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut pipe = pipe;
    let mut kept = Vec::new();
    (&mut pipe).take(OUTPUT_LIMIT).read_to_end(&mut kept)?;
    let discarded = std::io::copy(&mut pipe, &mut std::io::sink())?;
    Ok((kept, discarded > 0))
}

pub(crate) fn run(command: &mut Command, stdin: &[u8]) -> Run {
    // The memory limit cannot apply under AddressSanitizer; it watches RSS instead.
    let sanitizers = format!("exitcode={SANITIZER_EXIT}:detect_leaks=0:hard_rss_limit_mb=4096");
    let child = command
        .env("ASAN_OPTIONS", &sanitizers)
        .env("UBSAN_OPTIONS", format!("exitcode={SANITIZER_EXIT}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let output = child.and_then(|mut child| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        // Each pipe has its own thread: a program that prints before reading
        // all of its input would otherwise block on a full pipe.
        std::thread::scope(|scope| {
            // A program may exit without reading its input; that is not an error.
            scope.spawn(move || pipe.write_all(stdin));
            let stdout = scope.spawn(move || capped(stdout));
            let stderr = scope.spawn(move || capped(stderr));
            let status = child.wait()?;
            let (stdout, out_overflowed) = stdout.join().expect("reader panicked")?;
            let (stderr, err_overflowed) = stderr.join().expect("reader panicked")?;
            Ok((status, stdout, stderr, out_overflowed || err_overflowed))
        })
    });
    match output {
        Ok((status, stdout, stderr, overflowed)) => Run {
            stdout,
            stderr,
            // A process killed by a signal has no exit code; use the shell's
            // 128 + signal, as `timeout` itself may be killed after --kill-after.
            exit: status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal)),
            overflowed,
        },
        Err(err) => Run {
            stdout: Vec::new(),
            stderr: format!("cannot run {:?}: {err}", command.get_program()).into_bytes(),
            exit: None,
            overflowed: false,
        },
    }
}

pub(crate) fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let start = text.ceil_char_boundary(text.len().saturating_sub(DETAIL_LIMIT));
    text[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_ignoring_sigterm_times_out() {
        let ran = run(
            limited("1", false).args(["sh", "-c", "trap '' TERM; exec sleep 30"]),
            &[],
        );
        assert!(ran.timed_out(), "{:?}", ran.exit);
    }
}
