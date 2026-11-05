//! Claude Code transcripts. Claude Code writes one JSON record per line to
//! `~/.claude/projects/PROJECT/SESSION.jsonl`, and subagents' transcripts to
//! directories below it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The transcripts in `paths`, each a transcript or a directory searched for
/// them. A transcript reached twice, through a repeated path or a link, is
/// listed once.
pub fn find(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let (mut files, mut visited) = (Vec::new(), BTreeSet::new());
    for path in paths {
        collect(path, &mut files, &mut visited)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    }
    let mut seen = BTreeSet::new();
    files.retain(|file| seen.insert(file.canonicalize().unwrap_or_else(|_| file.clone())));
    Ok(files)
}

/// Collects transcripts under `path`, entering each directory once so that a
/// link back to an ancestor cannot repeat or loop.
fn collect(
    path: &Path,
    files: &mut Vec<PathBuf>,
    visited: &mut BTreeSet<PathBuf>,
) -> std::io::Result<()> {
    if path.is_file() {
        files.push(path.to_owned());
        return Ok(());
    }
    if !visited.insert(path.canonicalize()?) {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            collect(&entry, files, visited)?;
        } else if entry.is_file() && entry.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_directories_and_repeated_paths_are_read_once() {
        let dir = eval::Scratch::new("test").unwrap();
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::write(dir.join("project/s1.jsonl"), "{}\n").unwrap();
        std::os::unix::fs::symlink(&*dir, dir.join("project/loop")).unwrap();
        let files = find(&[dir.to_path_buf(), dir.join("project")]).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
    }

    #[test]
    fn only_files_are_transcripts() {
        let dir = eval::Scratch::new("test").unwrap();
        std::fs::write(dir.join("s1.jsonl"), "{}\n").unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("s2.jsonl")).unwrap();
        assert_eq!(find(&[dir.to_path_buf()]).unwrap(), [dir.join("s1.jsonl")]);
    }
}
