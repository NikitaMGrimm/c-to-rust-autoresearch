//! Decides whether the research loop keeps a candidate commit. It runs only
//! from a clean checkout of main, since it trusts its own code: it evaluates
//! the base and the candidate transpiler on the development set, the private validation set and the private variants of the
//! development set, appends everything to a private log and prints only what the
//! loop agent may see.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use eval::Stage;

const USAGE: &str = "usage: gate PRIVATE_DIR BASE CANDIDATE [--jobs N] [--usage FILE]\n\
     Exits 0 to keep CANDIDATE, 1 to reject it and 2 on errors.\n\
     --usage adds a report of the transcripts usage tool to the private log.\n\
     Run it from a clean checkout of main, not from the loop agent's checkout.";

/// The only paths a loop change may touch; the evaluator and the benchmarks
/// are fixed.
const TRANSPILER: &str = "crates/transpiler/";
const LOCKFILE: &str = "Cargo.lock";
const RULES: &str = "crates/transpiler/src/";
const CATALOGUE: &str = "crates/transpiler/CATALOGUE.md";

/// Stage of each program, by set and program name.
type Results = BTreeMap<&'static str, BTreeMap<String, Stage>>;
/// Passes that fail their fuzzed inputs, by set.
type FuzzFailures = BTreeMap<&'static str, BTreeSet<String>>;

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
    let head = git(&["rev-parse", "HEAD"])?;
    let main = git(&["rev-parse", "--verify", "main^{commit}"])?;
    let changes = git(&["status", "--porcelain", "--untracked-files=no"])?;
    if let Some(problem) = untrusted(&head, &main, &changes) {
        return Err(problem);
    }
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
        ("dev", eval::dev_dir()),
        ("validation", args.private.join("validation")),
        ("dev_variants", args.private.join("variants/dev")),
    ];

    let near_copies = near_copies(&args.private)?;
    let problem = change_problem(&base, &candidate)?;
    let ((before, fuzz_before), (after, fuzz_after)) = match problem {
        Some(_) => Default::default(),
        None => (
            results(&base, &sets, args.jobs)?,
            results(&candidate, &sets, args.jobs)?,
        ),
    };
    let failing_fuzz =
        |failures: &FuzzFailures, set: &str| failures.get(set).map_or(0, BTreeSet::len);
    let reason = problem.or_else(|| rejection(&before, &after));

    let summary: serde_json::Map<String, serde_json::Value> = sets
        .iter()
        .filter_map(|(set, _)| Some((*set, before.get(set)?, after.get(set)?)))
        .map(|(set, base_stages, candidate_stages)| {
            let changed: BTreeMap<&str, [&str; 2]> = candidate_stages
                .iter()
                .filter_map(|(name, &stage)| {
                    let old = base_stages.get(name).map_or("missing", |old| old.as_str());
                    (old != stage.as_str()).then_some((name.as_str(), [old, stage.as_str()]))
                })
                .collect();
            let mut json = serde_json::json!({
                "base": passes(&before, set),
                "candidate": passes(&after, set),
                "total": candidate_stages.len(),
                "changed": changed,
                "passes_failing_fuzz": {
                    "base": failing_fuzz(&fuzz_before, set),
                    "candidate": failing_fuzz(&fuzz_after, set),
                },
            });
            if let Some(copies) = near_copies.get(set) {
                let (base, _) = passes_without(base_stages, copies);
                let (candidate, total) = passes_without(candidate_stages, copies);
                json["without_near_copies"] =
                    serde_json::json!({"base": base, "candidate": candidate, "total": total});
            }
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
        let fuzz = &dev["passes_failing_fuzz"];
        println!(
            "dev passes failing fuzzed inputs: {} -> {}",
            fuzz["base"], fuzz["candidate"]
        );
    }
    match &reason {
        None => println!("keep"),
        Some(reason) => println!("reject: {reason}"),
    }
    Ok(reason.is_none())
}

/// Private programs that nearly repeat a development program, by set, from the
/// near-copies tool. Without its file there are none.
fn near_copies(private: &Path) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let path = private.join("near-copies.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let sets = json["sets"].as_object().into_iter().flatten();
    Ok(sets
        .map(|(set, programs)| {
            let names = programs.as_object().into_iter().flatten();
            (set.clone(), names.map(|(name, _)| name.clone()).collect())
        })
        .collect())
}

/// Passes and programs, leaving out `skipped`.
fn passes_without(stages: &BTreeMap<String, Stage>, skipped: &BTreeSet<String>) -> (usize, usize) {
    let kept: Vec<Stage> = stages
        .iter()
        .filter(|(name, _)| !skipped.contains(*name))
        .map(|(_, &stage)| stage)
        .collect();
    let passed = kept.iter().filter(|&&stage| stage == Stage::Pass).count();
    (passed, kept.len())
}

fn passes(results: &Results, set: &str) -> usize {
    results.get(set).map_or(0, |stages| {
        stages.values().filter(|&&s| s == Stage::Pass).count()
    })
}

/// The keep rule, provisional: the development score rises and the validation
/// score does not drop. The variants are only logged.
fn rejection(before: &Results, after: &Results) -> Option<String> {
    if passes(after, "dev") <= passes(before, "dev") {
        Some("the development score did not rise".to_owned())
    } else if passes(after, "validation") < passes(before, "validation") {
        Some("the validation score dropped".to_owned())
    } else {
        None
    }
}

/// Why the gate's own checkout cannot be trusted, if so. The gate is built
/// from it, so a checkout off main or with changes could run altered code,
/// such as the loop agent's.
fn untrusted(head: &str, main: &str, changes: &str) -> Option<String> {
    if head != main {
        Some("the gate's checkout is not at main; run it from a checkout of main".to_owned())
    } else if !changes.is_empty() {
        Some("the gate's checkout has uncommitted changes".to_owned())
    } else {
        None
    }
}

/// Why the change from `base` to `candidate` may not be evaluated, if so.
fn change_problem(base: &str, candidate: &str) -> Result<Option<String>, String> {
    let descends = Command::new("git")
        .args(["merge-base", "--is-ancestor", base, candidate])
        .current_dir(eval::REPO)
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
fn results(
    commit: &str,
    sets: &[(&'static str, PathBuf)],
    jobs: usize,
) -> Result<(Results, FuzzFailures), String> {
    let scratch = eval::Scratch::new("gate")?;
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
        eval::build_transpiler_sandboxed(&workspace).map_err(|err| format!("{commit}: {err}"))?;
    let translator = eval::Translator::Transpiler(transpiler);
    let (mut results, mut fuzz_failures) = (Results::new(), FuzzFailures::new());
    for (set, dir) in sets {
        let programs = eval::c_programs(dir)?;
        if programs.is_empty() {
            return Err(format!("no .c programs in {}", dir.display()));
        }
        let records = eval::evaluate_all(&programs, &translator, jobs)?;
        let failing = records.iter().filter(|r| r.fails_fuzz());
        fuzz_failures.insert(set, failing.map(|r| r.name.clone()).collect());
        let stages = records.into_iter().map(|r| (r.name, r.stage)).collect();
        results.insert(set, stages);
    }
    Ok((results, fuzz_failures))
}

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(eval::REPO)
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

    fn results(dev: &[Stage], validation: &[Stage]) -> Results {
        let stages = |stages: &[Stage]| {
            let named = stages.iter().enumerate();
            named.map(|(i, &stage)| (format!("p{i}"), stage)).collect()
        };
        Results::from([("dev", stages(dev)), ("validation", stages(validation))])
    }

    #[test]
    fn keeps_a_dev_gain_unless_validation_drops() {
        use Stage::{Mismatch, Panic, Pass};
        let before = results(&[Pass, Mismatch], &[Pass, Panic]);
        let gain = results(&[Pass, Pass], &[Pass, Panic]);
        let trade = results(&[Mismatch, Pass], &[Pass, Pass]);
        let overfit = results(&[Pass, Pass], &[Panic, Panic]);
        assert_eq!(rejection(&before, &gain), None);
        assert!(rejection(&before, &trade).unwrap().contains("development"));
        assert!(rejection(&before, &overfit).unwrap().contains("validation"));
    }

    #[test]
    fn only_a_clean_checkout_of_main_is_trusted() {
        assert_eq!(untrusted("a1", "a1", ""), None);
        assert!(untrusted("b2", "a1", "").unwrap().contains("not at main"));
        let changed = untrusted("a1", "a1", " M crates/eval/src/lib.rs").unwrap();
        assert!(changed.contains("uncommitted"));
    }

    #[test]
    fn near_copies_are_left_out_of_both_counts() {
        let stages = results(&[], &[Stage::Pass, Stage::Pass, Stage::Panic])["validation"].clone();
        let skipped = BTreeSet::from(["p0".to_owned()]);
        assert_eq!(passes_without(&stages, &skipped), (1, 2));
        assert_eq!(passes_without(&stages, &BTreeSet::new()), (2, 3));
    }

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
