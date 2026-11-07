//! Evaluates the private transfer set, from Project CodeNet, on the commits a
//! run kept, as the gate evaluates the other sets. No loop sees the set and
//! the gate never evaluates it, so it shows whether rules learned on TRACTOR
//! carry over to programs from another source. The results go to the run's
//! private directory only: they stay sealed like held-out, so the tool prints
//! nothing but its progress.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use eval::Stage;

const USAGE: &str = "usage: transfer PRIVATE_DIR RUN [--jobs N]\n\
     Evaluates PRIVATE_DIR/transfer on the start of run RUN and on every commit\n\
     the gate kept in it, as PRIVATE_DIR/runs/RUN/log.jsonl lists them, and\n\
     appends the results to PRIVATE_DIR/runs/RUN/transfer.jsonl, skipping\n\
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
    let commits = kept_commits(&text).map_err(|err| format!("{}: {err}", log.display()))?;
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

/// The commits to evaluate, in the gate's log `text`: the run's start, the
/// base of its first round, and every candidate the gate kept, in order.
fn kept_commits(text: &str) -> Result<Vec<String>, String> {
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
        let base = commit("base")?;
        if commits.is_empty() {
            commits.push(base);
        } else if commits.last() != Some(&base) {
            // Each round judges against the last kept commit, so a log that
            // does not is not a single run's.
            return Err(at("the base is not the last kept commit"));
        }
        if entry["decision"] == "keep" {
            commits.push(commit("candidate")?);
        }
    }
    if commits.is_empty() {
        return Err("the log is empty".to_owned());
    }
    Ok(commits)
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
            kept_commits(&log),
            Ok(vec![hash(0), hash(1), hash(3), hash(4)])
        );
        // A run whose rounds all rejected still has its start.
        assert_eq!(kept_commits(&round(0, 1, "reject")), Ok(vec![hash(0)]));
    }

    #[test]
    fn refuses_a_log_that_is_not_one_runs() {
        let mixed = [round(0, 1, "keep"), round(0, 2, "keep")].join("\n");
        assert!(kept_commits(&mixed).unwrap_err().contains("line 2"));
        assert!(kept_commits("").unwrap_err().contains("empty"));
        assert!(kept_commits("not json").unwrap_err().contains("line 1"));
        let short = r#"{"base": "abc", "candidate": "def", "decision": "keep"}"#;
        assert!(kept_commits(short).unwrap_err().contains("base"));
        let kept_nothing = format!(r#"{{"base": "{}", "decision": "keep"}}"#, hash(0));
        assert!(
            kept_commits(&kept_nothing)
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
