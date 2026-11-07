//! The bubblewrap sandbox for agent-written code: the transpiler, its build,
//! the compile of its translations and their runs.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Wraps agent-written code: no network, a read-only file system except
/// `writable`, and a hidden home directory, where the private benchmarks and
/// credentials live, except `readable`. Paths must be absolute. /run is
/// hidden too: the sockets of the user's systemd and D-Bus, Docker and WSL's
/// interop there would each start a process outside the sandbox.
pub(crate) fn sandboxed(inner: &Command, readable: &[&Path], writable: &Path) -> Command {
    let mut command = Command::new("bwrap");
    command.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
    command.args(["--tmpfs", "/tmp", "--tmpfs", "/run"]);
    if let Some(home) = std::env::var_os("HOME") {
        command.arg("--tmpfs").arg(home);
    }
    for path in readable {
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
