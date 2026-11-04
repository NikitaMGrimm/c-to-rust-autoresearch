//! Decides whether the research loop keeps a candidate commit. Run it from a
//! trusted checkout: it evaluates the base and the candidate transpiler on the
//! development set, the private validation set and the private variants of the
//! development set, appends everything to a private log and prints only what the
//! loop agent may see.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: gate PRIVATE_DIR BASE CANDIDATE [--jobs N] [--usage FILE]\n\
     Exits 0 to keep CANDIDATE, 1 to reject it and 2 on errors.\n\
     --usage adds a usage report (from the usage tool) to the private log.";

/// The only paths a loop change may touch; the evaluator and the benchmarks
/// are fixed.
const TRANSPILER: &str = "crates/transpiler/";
const LOCKFILE: &str = "Cargo.lock";
const RULES: &str = "crates/transpiler/src/";
const CATALOGUE: &str = "crates/transpiler/CATALOGUE.md";
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// Stage of each program, by set and program name.
type Results = BTreeMap<&'static str, BTreeMap<String, &'static str>>;

struct Args {
    private: PathBuf,
    base: String,
    candidate: String,
    jobs: usize,
    usage: Option<PathBuf>,
}

fn parse_args() -> Option<Args> {
    let mut positional = Vec::new();
    let (mut jobs, mut usage) = (2, None);
    let mut rest = std::env::args().skip(1);
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().filter(|value| !value.starts_with('-'));
        match arg.as_str() {
            "--jobs" => jobs = value()?.parse().ok().filter(|&n| n > 0)?,
            "--usage" => usage = Some(value()?.into()),
            _ if arg.starts_with('-') => return None,
            _ => positional.push(arg),
        }
    }
    let [private, base, candidate] = <[String; 3]>::try_from(positional).ok()?;
    Some(Args {
        private: private.into(),
        base,
        candidate,
        jobs,
        usage,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match gate(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

fn gate(args: &Args) -> Result<bool, String> {
    let usage = match &args.usage {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|err| err.to_string()))
            .map_err(|err| format!("cannot read usage report {}: {err}", path.display()))?,
        None => serde_json::Value::Null,
    };
    let base = git(&[
        "rev-parse",
        "--verify",
        &format!("{}^{{commit}}", args.base),
    ])?;
    let candidate = git(&[
        "rev-parse",
        "--verify",
        &format!("{}^{{commit}}", args.candidate),
    ])?;
    let sets: [(&'static str, PathBuf); 3] = [
        ("dev", Path::new(REPO).join("bench/dev")),
        ("validation", args.private.join("validation")),
        ("dev_variants", args.private.join("variants/dev")),
    ];

    let problem = change_problem(&base, &candidate)?;
    let (before, after) = match problem {
        Some(_) => (Results::new(), Results::new()),
        None => (
            results(&base, &sets, args.jobs)?,
            results(&candidate, &sets, args.jobs)?,
        ),
    };
    let passes = |results: &Results, set: &str| {
        results.get(set).map_or(0, |stages| {
            stages.values().filter(|&&s| s == "pass").count()
        })
    };
    let reason = problem.or_else(|| {
        if passes(&after, "dev") <= passes(&before, "dev") {
            Some("the development score did not rise".to_owned())
        } else if passes(&after, "validation") < passes(&before, "validation") {
            Some("the validation score dropped".to_owned())
        } else {
            None
        }
    });

    let summary: serde_json::Map<String, serde_json::Value> = sets
        .iter()
        .filter_map(|(set, _)| Some((*set, before.get(set)?, after.get(set)?)))
        .map(|(set, base_stages, candidate_stages)| {
            let changed: BTreeMap<&str, [&str; 2]> = candidate_stages
                .iter()
                .filter_map(|(name, &stage)| {
                    let old = base_stages.get(name).copied().unwrap_or("missing");
                    (old != stage).then_some((name.as_str(), [old, stage]))
                })
                .collect();
            let json = serde_json::json!({
                "base": passes(&before, set),
                "candidate": passes(&after, set),
                "total": candidate_stages.len(),
                "changed": changed,
            });
            (set.to_string(), json)
        })
        .collect();
    let entry = serde_json::json!({
        "time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_secs()),
        "base": base,
        "candidate": candidate,
        "evaluator": eval::versions(),
        "decision": if reason.is_none() { "keep" } else { "reject" },
        "reason": reason,
        "sets": summary,
        "usage": usage,
    });
    append_log(&args.private.join("loop"), &entry)?;

    // The loop agent sees the development set only, and validation through the
    // decision alone.
    if let Some(dev) = summary.get("dev") {
        println!(
            "dev: {} -> {} of {}",
            dev["base"], dev["candidate"], dev["total"]
        );
        for (name, stages) in dev["changed"].as_object().into_iter().flatten() {
            let stage = |i: usize| stages[i].as_str().unwrap_or_default();
            println!("  {name}: {} -> {}", stage(0), stage(1));
        }
    }
    match &reason {
        None => println!("keep"),
        Some(reason) => println!("reject: {reason}"),
    }
    Ok(reason.is_none())
}

/// Why the change from `base` to `candidate` may not be evaluated, if so.
fn change_problem(base: &str, candidate: &str) -> Result<Option<String>, String> {
    let descends = Command::new("git")
        .args(["merge-base", "--is-ancestor", base, candidate])
        .current_dir(REPO)
        .status()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if !descends.success() {
        return Ok(Some(
            "the candidate does not descend from the base".to_owned(),
        ));
    }
    let changed = git(&["diff", "--no-renames", "--name-only", base, candidate])?;
    let paths: Vec<&str> = changed.lines().collect();
    Ok(check_paths(&paths))
}

fn check_paths(paths: &[&str]) -> Option<String> {
    if paths.is_empty() {
        return Some("the candidate changes nothing".to_owned());
    }
    if let Some(path) = paths
        .iter()
        .find(|&&path| !path.starts_with(TRANSPILER) && path != LOCKFILE)
    {
        return Some(format!(
            "the candidate changes {path}, outside the transpiler"
        ));
    }
    if paths.iter().any(|path| path.starts_with(RULES)) && !paths.contains(&CATALOGUE) {
        return Some(format!("the rules changed but {CATALOGUE} did not"));
    }
    None
}

/// Builds the transpiler at `commit` in the sandbox and evaluates every set.
fn results(commit: &str, sets: &[(&'static str, PathBuf)], jobs: usize) -> Result<Results, String> {
    let scratch = std::env::temp_dir().join(format!("c2r-gate-{}-{commit}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch)
        .map_err(|err| format!("cannot create {}: {err}", scratch.display()))?;
    let evaluated = (|| {
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
        let transpiler = eval::build_transpiler_sandboxed(&workspace)
            .map_err(|err| format!("{commit}: {err}"))?;
        let translator = eval::Translator::Transpiler(transpiler);
        let mut results = Results::new();
        for (set, dir) in sets {
            let programs = eval::c_programs(dir)?;
            if programs.is_empty() {
                return Err(format!("no .c programs in {}", dir.display()));
            }
            let records = eval::evaluate_all(&programs, &translator, jobs)?;
            let stages = records.into_iter().map(|r| (r.name, r.stage)).collect();
            results.insert(set, stages);
        }
        Ok(results)
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    evaluated
}

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(REPO)
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

fn append_log(dir: &Path, entry: &serde_json::Value) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let path = dir.join("log.jsonl");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    writeln!(log, "{entry}").map_err(|err| format!("cannot write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transpiler_changes_with_a_catalogue_entry_are_evaluated() {
        let rule = "crates/transpiler/src/lib.rs";
        assert_eq!(check_paths(&[rule, CATALOGUE, "Cargo.lock"]), None);
        assert_eq!(check_paths(&["crates/transpiler/tests/cli.rs"]), None);
        assert!(check_paths(&[]).is_some());
        assert!(check_paths(&[rule]).unwrap().contains("CATALOGUE"));
        for protected in [
            "crates/eval/src/lib.rs",
            "bench/dev/g1_x.c",
            "Cargo.toml",
            "Cargo.lock.orig",
        ] {
            assert!(check_paths(&[rule, CATALOGUE, protected]).is_some());
        }
        // A prefix match, not a substring: no sibling crate named like it.
        assert!(check_paths(&["crates/transpiler-extra/src/lib.rs"]).is_some());
    }
}
