//! Paths the tools share, and scratch directories.

use std::io::Read;
use std::ops::Deref;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The repository root, whose rust-toolchain.toml pins rustc and Clippy.
pub const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The public development programs.
pub fn dev_dir() -> PathBuf {
    Path::new(REPO).join("bench/dev")
}

/// The entries of `dir`, sorted.
pub fn entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = std::fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()
        })
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    paths.sort();
    Ok(paths)
}

/// The `.c` programs in `dir`, sorted.
pub fn c_programs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut programs = entries(dir)?;
    programs.retain(|path| path.extension().is_some_and(|ext| ext == "c"));
    Ok(programs)
}

/// A new directory under the system's temporary directory that only this
/// user can open, removed when dropped. It must be new: a directory or
/// symlink someone planted at its path is never used.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(tool: &str) -> Result<Self, String> {
        static CREATED: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "c2r-{tool}-{}-{}",
            std::process::id(),
            CREATED.fetch_add(1, Ordering::Relaxed)
        ));
        Self::create(path)
    }

    /// Like `new`, under a random name: agent code can read the name of this
    /// directory, and a number would tell it how many were made before.
    pub(crate) fn unnumbered(tool: &str) -> Result<Self, String> {
        let mut random = [0; 8];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut random))
            .map_err(|err| format!("cannot read /dev/urandom: {err}"))?;
        let name = format!("c2r-{tool}-{:016x}", u64::from_ne_bytes(random));
        Self::create(std::env::temp_dir().join(name))
    }

    fn create(path: PathBuf) -> Result<Self, String> {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|err| format!("cannot create {}: {err}", path.display()))?;
        Ok(Self(path))
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
