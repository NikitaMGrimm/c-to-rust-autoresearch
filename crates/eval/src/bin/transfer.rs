//! Evaluates the private transfer set, from Project CodeNet, on the commits a
//! run kept, as the gate evaluates the other sets. No loop sees the set and
//! the gate never evaluates it, so it shows whether rules learned on TRACTOR
//! carry over to programs from another source. The results go to the run's
//! private directory only: they stay sealed like held-out, so the tool prints
//! nothing but its progress.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use eval::Stage;

const USAGE: &str = "usage: transfer PRIVATE_DIR RUN [--jobs N]\n\
     Evaluates PRIVATE_DIR/transfer on the start of run RUN and on every commit\n\
     that PRIVATE_DIR/runs/RUN/log.jsonl shows kept and the branch run/RUN holds,\n\
     and appends the results to PRIVATE_DIR/runs/RUN/transfer.jsonl, skipping\n\
     commits already there. Prints no results. Run it from a clean checkout of\n\
     main while no run is going: its load would change a gate's timings.";

struct Args {
    private: PathBuf,
    run: String,
    jobs: usize,
}

/// The tool's arguments, without the program name, or `None` for a usage error.
fn parse_args(args: impl IntoIterator<Item = String>) -> Option<Args> {
    let mut positional = Vec::new();
    let mut jobs = 2;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--jobs" => jobs = rest.next()?.parse().ok().filter(|&n| n > 0)?,
            _ if arg.starts_with('-') => return None,
            _ => positional.push(arg),
        }
    }
    let [private, run] = <[String; 2]>::try_from(positional).ok()?;
    eval::run_id(&run).then(|| Args {
        private: private.into(),
        run,
        jobs,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match transfer(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

fn transfer(args: &Args) -> Result<(), String> {
    if let Some(problem) = eval::untrusted(Path::new(eval::REPO))? {
        return Err(problem);
    }
    let dir = args.private.join("runs").join(&args.run);
    let log = dir.join("log.jsonl");
    let text = std::fs::read_to_string(&log)
        .map_err(|err| format!("cannot read the gate's log {}: {err}", log.display()))?;
    let branch = format!("refs/heads/run/{}", args.run);
    let tip = eval::git(&["rev-parse", "--verify", &format!("{branch}^{{commit}}")])?;
    let commits = kept_commits(&text, &tip, |commit| is_on(commit, &tip))
        .map_err(|err| format!("{}: {err}", log.display()))?;
    let out = dir.join("transfer.jsonl");
    let done = match std::fs::read_to_string(&out) {
        Ok(text) => evaluated(&text).map_err(|err| format!("{}: {err}", out.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(err) => return Err(format!("cannot read {}: {err}", out.display())),
    };
    let sets = [("transfer", args.private.join("transfer"))];
    for (i, commit) in commits.iter().enumerate() {
        let progress = format!("{} ({} of {})", &commit[..7], i + 1, commits.len());
        if done.contains(commit) {
            println!("transfer: {progress} was evaluated before");
            continue;
        }
        let (results, fuzz_failures) = eval::evaluate_commit(commit, &sets, args.jobs)?;
        let stages = &results["transfer"];
        let passes = stages.values().filter(|&&s| s == Stage::Pass).count();
        let named: BTreeMap<&str, &str> = stages
            .iter()
            .map(|(name, stage)| (name.as_str(), stage.as_str()))
            .collect();
        let entry = serde_json::json!({
            "time": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs()),
            "run": args.run,
            "commit": commit,
            "evaluator": eval::versions(),
            "passes": passes,
            "total": stages.len(),
            "stages": named,
            "passes_failing_fuzz": fuzz_failures["transfer"],
        });
        eval::append_log(&out, &entry)?;
        println!("transfer: {progress} evaluated");
    }
    println!(
        "transfer: {} commits of run {} in {}",
        commits.len(),
        args.run,
        out.display()
    );
    Ok(())
}

/// The commits to evaluate: the run's start, the base of the first round in
/// the gate's log `text`, and every candidate the gate kept that the run's
/// branch, at `tip`, holds, in order. The gate logs before the round moves the
/// branch, so a round stopped in between leaves a kept candidate that never
/// joined the run, and a round run again judges the same base twice. A
/// candidate rejected when its round ran again still counts if a later kept
/// commit builds on it, which the agent could do only from its object store.
fn kept_commits(
    text: &str,
    tip: &str,
    on_branch: impl Fn(&str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let mut commits: Vec<String> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let at = |problem: &str| format!("line {}: {problem}", number + 1);
        let entry: serde_json::Value =
            serde_json::from_str(line).map_err(|err| at(&err.to_string()))?;
        let commit = |field: &str| {
            let value = entry[field].as_str().unwrap_or_default();
            let hex = value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit());
            hex.then(|| value.to_owned())
                .ok_or_else(|| at(&format!("no commit in {field}")))
        };
        if commits.is_empty() {
            let start = commit("base")?;
            if !on_branch(&start)? {
                return Err(at("the run's start is not on its branch"));
            }
            commits.push(start);
        }
        if entry["decision"] == "keep" {
            let candidate = commit("candidate")?;
            if !commits.contains(&candidate) && on_branch(&candidate)? {
                commits.push(candidate);
            }
        }
    }
    if commits.is_empty() {
        return Err("the log is empty".to_owned());
    }
    if !commits.iter().any(|commit| commit == tip) {
        return Err(format!(
            "the run's branch is at {tip}, which the log does not show kept"
        ));
    }
    Ok(commits)
}

/// Whether `commit` is `tip` or one of its ancestors.
fn is_on(commit: &str, tip: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["merge-base", "--is-ancestor", commit, tip])
        .current_dir(eval::REPO)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!("cannot tell whether {commit} is in {tip}'s history")),
    }
}

/// The commits that the transfer log `text` already holds.
fn evaluated(text: &str) -> Result<BTreeSet<String>, String> {
    let entries = text.lines().enumerate().map(|(number, line)| {
        let entry: serde_json::Value =
            serde_json::from_str(line).map_err(|err| format!("line {}: {err}", number + 1))?;
        let commit = entry["commit"].as_str();
        commit
            .map(str::to_owned)
            .ok_or_else(|| format!("line {}: no commit", number + 1))
    });
    entries.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Option<Args> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn takes_the_private_directory_and_a_run() {
        let args = parsed(&["private", "wp2-1"]).unwrap();
        assert_eq!(
            (args.private, args.run, args.jobs),
            ("private".into(), "wp2-1".into(), 2)
        );
        assert_eq!(
            parsed(&["private", "wp2-1", "--jobs", "4"]).unwrap().jobs,
            4
        );
        for wrong in [
            &["private"][..],
            &["private", "wp2-1", "extra"],
            &["private", "wp2-1", "--jobs", "0"],
            &["private", "wp2-1", "--dev-only"],
            &["private", "../loop"],
            &["private", ".hidden"],
        ] {
            assert!(parsed(wrong).is_none(), "{wrong:?}");
        }
    }

    fn hash(n: u8) -> String {
        format!("{n:040x}")
    }

    fn round(base: u8, candidate: u8, decision: &str) -> String {
        serde_json::json!({"base": hash(base), "candidate": hash(candidate), "decision": decision})
            .to_string()
    }

    /// Commits 0 to 9 with those in `branch` on the run's branch.
    fn on(branch: &[u8]) -> impl Fn(&str) -> Result<bool, String> {
        let branch: Vec<String> = branch.iter().map(|&n| hash(n)).collect();
        move |commit| Ok(branch.iter().any(|on| on == commit))
    }

    #[test]
    fn evaluates_the_start_and_each_kept_commit() {
        let log = [
            round(0, 1, "keep"),
            round(1, 2, "reject"),
            round(1, 3, "keep"),
            round(3, 4, "keep"),
        ]
        .join("\n");
        assert_eq!(
            kept_commits(&log, &hash(4), on(&[0, 1, 3, 4])),
            Ok(vec![hash(0), hash(1), hash(3), hash(4)])
        );
        // A run whose rounds all rejected still has its start.
        assert_eq!(
            kept_commits(&round(0, 1, "reject"), &hash(0), on(&[0])),
            Ok(vec![hash(0)])
        );
    }

    #[test]
    fn leaves_out_a_kept_candidate_that_never_joined_the_run() {
        // The round stopped after the gate kept 1; the run went on from 0.
        let resumed = [round(0, 1, "keep"), round(0, 2, "keep")].join("\n");
        assert_eq!(
            kept_commits(&resumed, &hash(2), on(&[0, 2])),
            Ok(vec![hash(0), hash(2)])
        );
        // The same round, finished by hand, judged and kept 1 again.
        let again = [round(0, 1, "keep"), round(0, 1, "keep")].join("\n");
        assert_eq!(
            kept_commits(&again, &hash(1), on(&[0, 1])),
            Ok(vec![hash(0), hash(1)])
        );
        // The last round stopped after the gate kept 2.
        let last = [round(0, 1, "keep"), round(1, 2, "keep")].join("\n");
        assert_eq!(
            kept_commits(&last, &hash(1), on(&[0, 1])),
            Ok(vec![hash(0), hash(1)])
        );
        // A gate run by hand on an earlier round adds nothing.
        let checked = [
            round(0, 1, "keep"),
            round(1, 2, "keep"),
            round(0, 1, "keep"),
        ]
        .join("\n");
        assert_eq!(
            kept_commits(&checked, &hash(2), on(&[0, 1, 2])),
            Ok(vec![hash(0), hash(1), hash(2)])
        );
    }

    #[test]
    fn refuses_a_log_that_does_not_match_the_branch() {
        let log = round(0, 1, "keep");
        let tip = kept_commits(&log, &hash(2), on(&[0, 1, 2])).unwrap_err();
        assert!(tip.contains("does not show kept"), "{tip}");
        let start = kept_commits(&log, &hash(1), on(&[1])).unwrap_err();
        assert!(start.contains("start"), "{start}");
        assert!(
            kept_commits("", &hash(0), on(&[0]))
                .unwrap_err()
                .contains("empty")
        );
        let malformed = kept_commits("not json", &hash(0), on(&[0])).unwrap_err();
        assert!(malformed.contains("line 1"), "{malformed}");
        let short = r#"{"base": "abc", "candidate": "def", "decision": "keep"}"#;
        assert!(
            kept_commits(short, &hash(0), on(&[0]))
                .unwrap_err()
                .contains("base")
        );
        let kept_nothing = format!(r#"{{"base": "{}", "decision": "keep"}}"#, hash(0));
        assert!(
            kept_commits(&kept_nothing, &hash(0), on(&[0]))
                .unwrap_err()
                .contains("candidate")
        );
    }

    #[test]
    fn skips_the_commits_already_evaluated() {
        let log = format!(
            "{{\"commit\": \"{}\"}}\n{{\"commit\": \"{}\"}}\n",
            hash(0),
            hash(3)
        );
        assert_eq!(evaluated(&log), Ok(BTreeSet::from([hash(0), hash(3)])));
        assert_eq!(evaluated(""), Ok(BTreeSet::new()));
        assert!(evaluated("{}").unwrap_err().contains("line 1"));
    }
}
