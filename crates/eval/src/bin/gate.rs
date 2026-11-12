//! Decides whether the research loop keeps a candidate commit. It trusts its
//! own code, so it runs only from a clean checkout of main outside the loop
//! agent's clone. It evaluates the base and the candidate transpiler on the
//! development set, the private validation and held-out sets and the private
//! variants of the development set, appends everything to a private log and
//! prints only what the loop agent may see.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use eval::{FuzzFailures, LaxPasses, Results, Stage, git, git_in, run_id, untrusted};

const USAGE: &str = "usage: gate PRIVATE_DIR BASE CANDIDATE [--jobs N] [--usage FILE]\n\
     \x20           [--dev-only] [--strict] [--run ID]\n\
     Exits 0 to keep CANDIDATE, 1 to reject it and 2 on errors.\n\
     --usage adds a report of the transcripts usage tool to the private log.\n\
     --dev-only keeps CANDIDATE on the development score alone; validation is\n\
     only logged. --strict counts only strict passes in both scores. --run logs to PRIVATE_DIR/runs/ID instead of PRIVATE_DIR/loop.\n\
     Run it from a clean checkout of main, not from the loop agent's checkout.";

/// The only paths a loop change may touch; the evaluator and the benchmarks
/// are fixed.
const TRANSPILER: &str = "crates/transpiler/";
const LOCKFILE: &str = "Cargo.lock";
const RULES: &str = "crates/transpiler/src/";
const CATALOGUE: &str = "crates/transpiler/CATALOGUE.md";
/// How a fix starts its subject, such as one for a review comment: it must
/// keep the scores rather than raise them.
const FIX: &str = "fix(transpiler)";
/// How a refactoring starts its subject: it prepares later rules or tidies
/// earlier ones. It must keep the scores as a fix must, and translates
/// nothing new, so it needs no catalogue entry.
const REFACTOR: &str = "refactor(transpiler)";

struct Args {
    private: PathBuf,
    base: String,
    candidate: String,
    jobs: usize,
    usage: Option<PathBuf>,
    dev_only: bool,
    strict: bool,
    run: Option<String>,
}

/// The gate's arguments, without the program name, or `None` for a usage error.
fn parse_args(args: impl IntoIterator<Item = String>) -> Option<Args> {
    let mut positional = Vec::new();
    let (mut jobs, mut usage, mut dev_only, mut run) = (2, None, false, None);
    let mut strict = false;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().filter(|value| !value.starts_with('-'));
        match arg.as_str() {
            "--jobs" => jobs = value()?.parse().ok().filter(|&n| n > 0)?,
            "--usage" => usage = Some(value()?.into()),
            "--dev-only" => dev_only = true,
            "--strict" => strict = true,
            "--run" => run = Some(value().filter(|id| run_id(id))?),
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
        dev_only,
        strict,
        run,
    })
}

/// Where the gate appends its log: the run's directory, or the loop's.
fn log_dir(args: &Args) -> PathBuf {
    match &args.run {
        Some(run) => args.private.join("runs").join(run),
        None => args.private.join("loop"),
    }
}

fn main() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1)) else {
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
    if let Some(problem) = untrusted(Path::new(eval::REPO))? {
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
    // Held-out results go to the private log only, for the final results.
    let sets: [(&'static str, PathBuf); 4] = [
        ("dev", eval::dev_dir()),
        ("validation", args.private.join("validation")),
        ("held-out", args.private.join("held-out")),
        ("dev_variants", args.private.join("variants/dev")),
    ];

    let near_copies = near_copies(&args.private)?;
    let repo = Path::new(eval::REPO);
    let refactor = commits_are(repo, &base, &candidate, &[REFACTOR])?;
    let problem = change_problem(&base, &candidate, refactor)?;
    let ((before, fuzz_before, lax_before), (after, fuzz_after, lax_after)) = match problem {
        Some(_) => Default::default(),
        None => (
            eval::evaluate_commit(&base, &sets, args.jobs)?,
            eval::evaluate_commit(&candidate, &sets, args.jobs)?,
        ),
    };
    let failing_fuzz =
        |failures: &FuzzFailures, set: &str| failures.get(set).map_or(0, BTreeSet::len);
    let lax = |passes: &LaxPasses, set: &str| passes.get(set).map_or(0, BTreeSet::len);
    let fix = commits_are(repo, &base, &candidate, &[FIX])?;
    let steady = commits_are(repo, &base, &candidate, &[FIX, REFACTOR])?;
    let scores = |set: &str| {
        (
            score(&before, &lax_before, set, args.strict),
            score(&after, &lax_after, set, args.strict),
        )
    };
    let reason =
        problem.or_else(|| rejection(scores("dev"), scores("validation"), steady, args.dev_only));

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
                "passes_not_strict": {
                    "base": lax(&lax_before, set),
                    "candidate": lax(&lax_after, set),
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
        "fix": fix,
        "refactor": refactor,
        "dev_only": args.dev_only,
        "strict": args.strict,
        "decision": if reason.is_none() { "keep" } else { "reject" },
        "reason": reason,
        "sets": summary,
        "usage": usage,
    });
    eval::append_log(&log_dir(args).join("log.jsonl"), &entry)?;

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
        if args.strict {
            let (old, new) = scores("dev");
            println!("dev strict passes: {old} -> {new} of {}", dev["total"]);
            for name in lax_after.get("dev").into_iter().flatten() {
                println!("  {name}: pass, not strict");
            }
        }
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
    // A file from before a set existed would drop that set's score without
    // near-copies from the log.
    if let Some(set) = ["validation", "held-out"]
        .into_iter()
        .find(|set| !json["sets"][set].is_object())
    {
        return Err(format!(
            "{} has no {set} set; run the near-copies tool again",
            path.display()
        ));
    }
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

/// A set's score: its passes, for a strict gate only the strict ones.
fn score(results: &Results, lax_passes: &LaxPasses, set: &str, strict: bool) -> usize {
    let lax = lax_passes.get(set).map_or(0, BTreeSet::len);
    passes(results, set) - if strict { lax } else { 0 }
}

/// The keep rule, on each set's score before and after: the development
/// score rises, or for a `steady` change, one of fixes and refactorings, does
/// not drop, and, unless `dev_only`, the validation score does not drop.
/// Held-out and the variants are only logged.
fn rejection(
    (dev_before, dev_after): (usize, usize),
    (validation_before, validation_after): (usize, usize),
    steady: bool,
    dev_only: bool,
) -> Option<String> {
    if steady && dev_after < dev_before {
        Some("the fix or refactoring lowered the development score".to_owned())
    } else if !steady && dev_after <= dev_before {
        Some("the development score did not rise".to_owned())
    } else if !dev_only && validation_after < validation_before {
        Some("the validation score dropped".to_owned())
    } else {
        None
    }
}

/// Whether the subject of every commit from `base` to `candidate` in the
/// repository at `dir` starts with one of `kinds`.
fn commits_are(dir: &Path, base: &str, candidate: &str, kinds: &[&str]) -> Result<bool, String> {
    // Each subject ends with NUL rather than a newline, so that the trim in
    // `git_in` cannot drop an empty one.
    let range = format!("{base}..{candidate}");
    let subjects = git_in(dir, &["log", "-z", "--format=%s", &range])?;
    Ok(!subjects.is_empty()
        && subjects
            .split_terminator('\0')
            .all(|subject| kinds.iter().any(|kind| subject.starts_with(kind))))
}

/// Why the change from `base` to `candidate` may not be evaluated, if so.
fn change_problem(base: &str, candidate: &str, refactor: bool) -> Result<Option<String>, String> {
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
    Ok(check_paths(&paths, refactor))
}

fn check_paths(paths: &[&str], refactor: bool) -> Option<String> {
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
    let rules = paths.iter().any(|path| path.starts_with(RULES));
    if rules && !refactor && !paths.contains(&CATALOGUE) {
        return Some(format!("the rules changed but {CATALOGUE} did not"));
    }
    None
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

    fn reject(before: &Results, after: &Results, fix: bool, dev_only: bool) -> Option<String> {
        let scores = |set| (passes(before, set), passes(after, set));
        rejection(scores("dev"), scores("validation"), fix, dev_only)
    }

    #[test]
    fn a_strict_gate_scores_only_strict_passes() {
        use Stage::{Mismatch, Pass};
        let all = results(&[Pass, Pass, Mismatch], &[]);
        let lax = LaxPasses::from([("dev", BTreeSet::from(["p1".to_owned()]))]);
        assert_eq!(score(&all, &lax, "dev", false), 2);
        assert_eq!(score(&all, &lax, "dev", true), 1);
        assert!(
            parsed(&["private", "base", "candidate", "--strict"])
                .unwrap()
                .strict
        );
        // A pass that stops being strict lowers a strict score, so a fix that
        // causes it is rejected.
        assert!(rejection((2, 1), (0, 0), true, true).is_some());
    }

    #[test]
    fn keeps_a_dev_gain_unless_validation_drops() {
        use Stage::{Mismatch, Panic, Pass};
        let before = results(&[Pass, Mismatch], &[Pass, Panic]);
        let gain = results(&[Pass, Pass], &[Pass, Panic]);
        let trade = results(&[Mismatch, Pass], &[Pass, Pass]);
        let overfit = results(&[Pass, Pass], &[Panic, Panic]);
        assert_eq!(reject(&before, &gain, false, false), None);
        assert!(
            reject(&before, &trade, false, false)
                .unwrap()
                .contains("development")
        );
        assert!(
            reject(&before, &overfit, false, false)
                .unwrap()
                .contains("validation")
        );
    }

    #[test]
    fn dev_only_keeps_a_dev_gain_even_when_validation_drops() {
        use Stage::{Mismatch, Panic, Pass};
        let before = results(&[Pass, Mismatch], &[Pass, Panic]);
        let overfit = results(&[Pass, Pass], &[Panic, Panic]);
        let trade = results(&[Mismatch, Pass], &[Pass, Pass]);
        let same_overfit = results(&[Mismatch, Pass], &[Panic, Panic]);
        assert_eq!(reject(&before, &overfit, false, true), None);
        assert_eq!(reject(&before, &same_overfit, true, true), None);
        assert!(reject(&before, &trade, false, true).is_some());
        assert!(reject(&before, &overfit, false, false).is_some());
    }

    fn parsed(args: &[&str]) -> Option<Args> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn a_run_logs_to_its_own_directory() {
        let args = parsed(&["private", "base", "candidate", "--run", "wp2-a.1_b"]).unwrap();
        assert_eq!(log_dir(&args), Path::new("private/runs/wp2-a.1_b"));
        let args = parsed(&["private", "base", "candidate", "--dev-only"]).unwrap();
        assert!(args.dev_only);
        assert_eq!(log_dir(&args), Path::new("private/loop"));
    }

    #[test]
    fn a_run_id_that_could_leave_its_directory_is_a_usage_error() {
        for id in [
            "",
            ".",
            "..",
            ".hidden",
            "-baseline",
            "_a",
            "a/b",
            "../loop",
            "a b",
        ] {
            assert!(
                parsed(&["private", "base", "candidate", "--run", id]).is_none(),
                "{id}"
            );
        }
        // Refused for itself too, as loop/round and loop/run refuse it, not
        // only because the parser takes it for an option.
        assert!(!run_id("-baseline"));
    }

    #[test]
    fn near_copies_must_list_both_private_sets() {
        let dir = eval::Scratch::new("gate-near-copies").unwrap();
        let file = dir.join("near-copies.json");
        std::fs::write(&file, r#"{"sets": {"validation": {"g1_a": {}}}}"#).unwrap();
        assert!(near_copies(&dir).unwrap_err().contains("held-out"));
        std::fs::write(
            &file,
            r#"{"sets": {"validation": {"g1_a": {}}, "held-out": {}}}"#,
        )
        .unwrap();
        assert_eq!(
            near_copies(&dir).unwrap()["validation"],
            BTreeSet::from(["g1_a".to_owned()])
        );
        std::fs::remove_file(&file).unwrap();
        assert!(near_copies(&dir).unwrap().is_empty());
    }

    #[test]
    fn keeps_a_fix_that_lowers_no_score() {
        use Stage::{Mismatch, Panic, Pass};
        let before = results(&[Pass, Mismatch], &[Pass, Panic]);
        let same = results(&[Mismatch, Pass], &[Pass, Panic]);
        let worse = results(&[Mismatch, Mismatch], &[Pass, Panic]);
        let overfit = results(&[Pass, Pass], &[Panic, Panic]);
        let same_overfit = results(&[Mismatch, Pass], &[Panic, Panic]);
        assert_eq!(reject(&before, &same, true, false), None);
        assert!(reject(&before, &same, false, false).is_some());
        assert!(
            reject(&before, &worse, true, false)
                .unwrap()
                .contains("lowered")
        );
        for candidate in [&overfit, &same_overfit] {
            let reason = reject(&before, candidate, true, false).unwrap();
            assert!(reason.contains("validation"), "{reason}");
        }
    }

    #[test]
    fn a_change_is_a_fix_when_all_its_commits_are() {
        let dir = eval::Scratch::new("test").unwrap();
        let git = |args: &[&str]| git_in(&dir, args).unwrap();
        let commit = |subject: &str| {
            let author = ["-c", "user.name=t", "-c", "user.email=t@t"];
            let unsigned = ["-c", "commit.gpgsign=false", "commit", "-q", "--no-verify"];
            let empty = ["--allow-empty", "--allow-empty-message", "-m", subject];
            git(&[&author[..], &unsigned, &empty].concat());
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "main"]);
        let base = commit("feat(transpiler): a rule");
        let fix = commit("fix(transpiler): a review comment");
        let rule = commit("feat(transpiler): another rule");
        let late = commit("fix(transpiler): a late fix");
        let blank = commit("");
        let after_blank = commit("fix(transpiler): after a blank subject");
        let is_fix = |base: &str, candidate: &str| commits_are(&dir, base, candidate, &[FIX]);
        assert_eq!(is_fix(&base, &fix), Ok(true));
        assert_eq!(is_fix(&base, &rule), Ok(false));
        assert_eq!(is_fix(&base, &late), Ok(false));
        // A blank subject at either end of the range is no fix either.
        assert_eq!(is_fix(&rule, &blank), Ok(false));
        assert_eq!(is_fix(&late, &after_blank), Ok(false));
        assert_eq!(is_fix(&base, &base), Ok(false));
        // A refactoring keeps the scores as a fix does, alone or beside one.
        let refactoring = commit("refactor(transpiler): a fact for later rules");
        let steady = [FIX, REFACTOR];
        assert_eq!(
            commits_are(&dir, &after_blank, &refactoring, &[REFACTOR]),
            Ok(true)
        );
        assert_eq!(
            commits_are(&dir, &blank, &refactoring, &[REFACTOR]),
            Ok(false)
        );
        assert_eq!(commits_are(&dir, &blank, &refactoring, &steady), Ok(true));
        assert_eq!(commits_are(&dir, &base, &refactoring, &steady), Ok(false));
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
        assert_eq!(check_paths(&[rule, CATALOGUE, "Cargo.lock"], false), None);
        assert_eq!(
            check_paths(&["crates/transpiler/tests/cli.rs"], false),
            None
        );
        assert!(check_paths(&[], false).is_some());
        assert!(check_paths(&[rule], false).unwrap().contains("CATALOGUE"));
        // A refactoring translates nothing new, so it has no entry to add.
        assert_eq!(check_paths(&[rule], true), None);
        assert!(check_paths(&[rule, "Cargo.toml"], true).is_some());
        for protected in [
            "crates/eval/src/lib.rs",
            "bench/dev/g1_x.c",
            "Cargo.toml",
            "Cargo.lock.orig",
        ] {
            assert!(check_paths(&[rule, CATALOGUE, protected], false).is_some());
        }
        // A prefix match, not a substring: no sibling crate named like it.
        assert!(check_paths(&["crates/transpiler-extra/src/lib.rs"], false).is_some());
    }
}
