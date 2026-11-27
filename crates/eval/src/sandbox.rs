//! The bubblewrap sandbox for agent-written code: the transpiler, its build,
//! the compile of its translations and their runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::run::Run;

/// Wraps agent-written code: no network, a read-only file system except
/// `writable`, and a hidden home directory, where the private benchmarks and
/// credentials live, except `readable`. The temporary directory is hidden
/// too: the evaluator's scratch directories there hold copies of the private
/// programs and the translations of other programs. Paths must be absolute. /run is
/// hidden too: the sockets of the user's systemd and D-Bus, Docker and WSL's
/// interop there would each start a process outside the sandbox.
pub(crate) fn sandboxed(inner: &Command, readable: &[&Path], writable: &Path) -> Command {
    // TMPDIR may put the temporary directory elsewhere than /tmp.
    let temp = std::env::temp_dir();
    hiding(
        &temp.canonicalize().unwrap_or(temp),
        inner,
        readable,
        writable,
    )
}

/// `sandboxed` for the temporary directory `temp`.
fn hiding(temp: &Path, inner: &Command, readable: &[&Path], writable: &Path) -> Command {
    let mut command = Command::new("bwrap");
    command.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
    command.args(["--tmpfs", "/tmp", "--tmpfs", "/run"]);
    if let Some(home) = std::env::var_os("HOME") {
        command.arg("--tmpfs").arg(home);
    }
    // A readable directory around the temporary directory comes first, or
    // its bind would show the temporary directory again. Where its path is
    // a link, the temporary directory has a second path under it.
    let inside = |path: &&&Path| {
        let resolved = path.canonicalize().unwrap_or(path.to_path_buf());
        temp.strip_prefix(resolved).ok().map(|rest| path.join(rest))
    };
    for path in readable {
        if let Some(temp) = inside(&path) {
            command.arg("--ro-bind").arg(path).arg(path);
            command.arg("--tmpfs").arg(temp);
        }
    }
    if temp != Path::new("/tmp") {
        command.arg("--tmpfs").arg(temp);
    }
    for path in readable.iter().filter(|path| inside(path).is_none()) {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command.arg("--bind").arg(writable).arg(writable);
    if let Some(dir) = inner.get_current_dir() {
        command.arg("--chdir").arg(dir);
    }
    command.args(["--unshare-all", "--die-with-parent", "--new-session", "--"]);
    command.arg(inner.get_program()).args(inner.get_args());
    for (key, value) in inner.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
}

/// The run of a sandboxed command, or an error when the sandbox did not
/// start: bubblewrap could not be run, or it stopped before the command,
/// which it reports under its own name with the exit code 1. Such a run says
/// nothing about the code in it, so it must not become a result. Agent code
/// can end the same way; that stops the evaluation as well, and the error
/// leaves out what the run printed, which agent code may have written.
pub(crate) fn started(run: Run) -> Result<Run, String> {
    let stopped = run.exit == Some(1) && run.stderr.starts_with(b"bwrap: ");
    if run.exit.is_none() || stopped {
        return Err("the sandbox did not start (bubblewrap failed); nothing was evaluated".into());
    }
    Ok(run)
}

/// What cargo and rustc need inside the sandbox. Not all of CARGO_HOME: it
/// holds the crates.io token.
pub(crate) fn toolchain() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cargo_home = std::env::var_os("CARGO_HOME").map_or(home.join(".cargo"), PathBuf::from);
    let rustup_home = std::env::var_os("RUSTUP_HOME").map_or(home.join(".rustup"), PathBuf::from);
    ["bin", "registry", "git"]
        .iter()
        .map(|dir| cargo_home.join(dir))
        .chain([rustup_home])
        .filter(|path| path.exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readable_directory_around_the_temporary_directory_does_not_show_it() {
        let dir = crate::Scratch::new("test").unwrap();
        let (around, link) = (dir.join("around"), dir.join("link"));
        let (temp, out) = (around.join("tmp"), dir.join("out"));
        for dir in [&around, &temp, &out] {
            std::fs::create_dir(dir).unwrap();
        }
        std::os::unix::fs::symlink(&around, &link).unwrap();
        for file in [temp.join("program.c"), around.join("tool")] {
            std::fs::write(file, "").unwrap();
        }
        // By its own path and by a link to it, as a toolchain may be installed.
        for readable in [&around, &link] {
            let reads = |file: &str| {
                let mut cat = Command::new("cat");
                cat.arg(readable.join(file));
                let status = hiding(&temp, &cat, &[readable], &out).status().unwrap();
                status.success()
            };
            assert!(reads("tool"), "{}", readable.display());
            assert!(!reads("tmp/program.c"), "{}", readable.display());
        }
    }

    #[test]
    fn a_sandbox_that_does_not_start_is_an_error_and_a_failing_command_is_not() {
        let dir = crate::Scratch::new("test").unwrap();
        let run = |writable: &Path, script: &str| {
            let mut inner = Command::new("sh");
            inner.args(["-c", script]);
            started(crate::run::run(&mut sandboxed(&inner, &[], writable), &[]))
        };
        // bubblewrap cannot bind a directory that is not there.
        let error = run(&dir.join("missing"), "true").unwrap_err();
        assert!(!error.contains("missing"), "{error}");
        let failed = run(&dir, "echo no rule >&2; exit 1").unwrap();
        assert_eq!(failed.exit, Some(1));
        // The same ending from the code in the sandbox stops the evaluation
        // too, and its output stays out of the error.
        let error = run(&dir, "echo bwrap: a private line >&2; exit 1").unwrap_err();
        assert!(!error.contains("private"), "{error}");
        let mut absent = Command::new("/no/such/bwrap");
        assert!(started(crate::run::run(&mut absent, &[])).is_err());
    }
}
