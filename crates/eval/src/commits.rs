//! What the gate and the transfer tool share: both trust only a clean checkout
//! of main outside the loop agent's clone, build the transpiler of a commit in
//! the sandbox, evaluate sets of programs with it and append the results to a
//! private log.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{REPO, Scratch, Stage, Translator};

/// Stage of each program, by set and program name.
pub type Results = BTreeMap<&'static str, BTreeMap<String, Stage>>;
/// Passes that fail their fuzzed inputs, by set.
pub type FuzzFailures = BTreeMap<&'static str, BTreeSet<String>>;

/// Whether `id` can name a run's log directory: letters, digits, `.`, `-`
/// and `_`, starting with a letter or digit, so that it is neither hidden nor
/// taken for an option.
pub fn run_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Why the checkout at `dir` cannot be trusted to run the gate, if so. The
/// gate is built from it, so a checkout off main, with changes or untracked
/// files such as a build script, or the loop agent's own clone could run the
/// agent's code. The check runs after that build, so it catches a mistaken
/// checkout, not a hostile one.
pub fn untrusted(dir: &Path) -> Result<Option<String>, String> {
    let git = |args: &[&str]| git_in(dir, args);
    // loop/setup marks the loop agent's clone, whose main the agent can move.
    let problem = if git(&["config", "--bool", "--default", "false", "c2r.loop"])? == "true" {
        "the gate's checkout is the loop agent's; run it from the main checkout"
    } else if git(&["rev-parse", "HEAD"])? != git(&["rev-parse", "--verify", "main^{commit}"])? {
        "the gate's checkout is not at main; run it from a checkout of main"
    } else if !git(&["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        "the gate's checkout has uncommitted changes or untracked files"
    } else {
        return Ok(None);
    };
    Ok(Some(problem.to_owned()))
}

/// Builds the transpiler at `commit` in the sandbox and evaluates every set.
pub fn evaluate_commit(
    commit: &str,
    sets: &[(&'static str, PathBuf)],
    jobs: usize,
) -> Result<(Results, FuzzFailures), String> {
    let scratch = Scratch::new("gate")?;
    let workspace = scratch.join("workspace");
    std::fs::create_dir(&workspace).map_err(|err| err.to_string())?;
    let archive = scratch.join("workspace.tar");
    let archive = archive.to_str().ok_or("temporary directory is not UTF-8")?;
    git(&["archive", "--output", archive, commit])?;
    let unpacked = Command::new("tar")
        .args(["-xf", archive, "-C"])
        .arg(&workspace)
        .status()
        .map_err(|err| format!("cannot run tar: {err}"))?;
    if !unpacked.success() {
        return Err(format!("cannot unpack {commit}"));
    }
    let transpiler =
        crate::build_transpiler_sandboxed(&workspace).map_err(|err| format!("{commit}: {err}"))?;
    let translator = Translator::Transpiler(transpiler);
    let (mut results, mut fuzz_failures) = (Results::new(), FuzzFailures::new());
    for (set, dir) in sets {
        let programs = crate::c_programs(dir)?;
        if programs.is_empty() {
            return Err(format!("no .c programs in {}", dir.display()));
        }
        let records = crate::evaluate_all(&programs, &translator, jobs)?;
        let failing = records.iter().filter(|r| r.fails_fuzz());
        fuzz_failures.insert(set, failing.map(|r| r.name.clone()).collect());
        let stages = records.into_iter().map(|r| (r.name, r.stage)).collect();
        results.insert(set, stages);
    }
    Ok((results, fuzz_failures))
}

/// Runs git in this checkout.
pub fn git(args: &[&str]) -> Result<String, String> {
    git_in(Path::new(REPO), args)
}

pub fn git_in(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Appends `entry` as a line to the log at `path`, making its directory.
pub fn append_log(path: &Path, entry: &serde_json::Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    writeln!(log, "{entry}").map_err(|err| format!("cannot write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_checkout_of_main_outside_the_loop_is_trusted() {
        let dir = Scratch::new("test").unwrap();
        let git = |args: &[&str]| git_in(&dir, args).unwrap();
        let commit = || {
            let author = ["-c", "user.name=t", "-c", "user.email=t@t"];
            let unsigned = ["-c", "commit.gpgsign=false", "commit", "-q", "--no-verify"];
            git(&[&author[..], &unsigned, &["--allow-empty", "-m", "c"]].concat());
        };
        git(&["init", "-q", "-b", "main"]);
        commit();
        let problem = || untrusted(&dir).unwrap().unwrap_or_default();
        assert_eq!(untrusted(&dir), Ok(None));
        std::fs::write(dir.join("build.rs"), "").unwrap();
        assert!(problem().contains("untracked"));
        std::fs::remove_file(dir.join("build.rs")).unwrap();
        git(&["checkout", "-q", "-b", "candidate"]);
        commit();
        assert!(problem().contains("not at main"));
        git(&["checkout", "-q", "main"]);
        git(&["config", "c2r.loop", "true"]);
        assert!(problem().contains("loop agent's"));
    }

    #[test]
    fn a_log_and_its_directory_are_made_on_first_use() {
        let dir = Scratch::new("test").unwrap();
        let path = dir.join("runs/r/log.jsonl");
        append_log(&path, &serde_json::json!({"a": 1})).unwrap();
        append_log(&path, &serde_json::json!({"a": 2})).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n"
        );
    }
}
