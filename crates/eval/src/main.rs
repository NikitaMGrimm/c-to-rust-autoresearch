use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: eval [BENCH_DIR] [--only TEXT] [--json FILE] [--jobs N] [--quiet]\n\
     \x20           [--translations DIR]\n\
     BENCH_DIR defaults to bench/dev; program names start with their group, e.g. g1_gcd.c.\n\
     --translations evaluates existing translations DIR/NAME.rs instead of the transpiler's.";

struct Args {
    bench: PathBuf,
    only: Option<String>,
    json: Option<PathBuf>,
    translations: Option<PathBuf>,
    jobs: usize,
    quiet: bool,
}

fn parse_args() -> Option<Args> {
    let mut args = Args {
        bench: eval::dev_dir(),
        only: None,
        json: None,
        translations: None,
        // Shared machines: two parallel programs by default.
        jobs: 2,
        quiet: false,
    };
    let mut rest = std::env::args().skip(1);
    let mut bench_given = false;
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().filter(|value| !value.starts_with('-'));
        match arg.as_str() {
            "--only" => args.only = Some(value()?),
            "--json" => args.json = Some(value()?.into()),
            "--translations" => args.translations = Some(value()?.into()),
            "--jobs" => args.jobs = value()?.parse().ok().filter(|&n| n > 0)?,
            "--quiet" => args.quiet = true,
            _ if arg.starts_with('-') || bench_given => return None,
            _ => {
                args.bench = arg.into();
                bench_given = true;
            }
        }
    }
    Some(args)
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let programs: Vec<PathBuf> = match eval::c_programs(&args.bench) {
        Ok(paths) => paths
            .into_iter()
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                args.only
                    .as_ref()
                    .is_none_or(|only| name.contains(only.as_str()))
            })
            .collect(),
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    if programs.is_empty() {
        eprintln!("no .c programs in {}", args.bench.display());
        return ExitCode::FAILURE;
    }
    // Holds the transpiler's build until the evaluation ends.
    let scratch = match eval::Scratch::new("eval") {
        Ok(scratch) => scratch,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let translator = match &args.translations {
        Some(dir) => eval::Translator::Existing(dir.clone()),
        None => match transpiler(&scratch) {
            Ok(path) => eval::Translator::Transpiler(path),
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
    };
    let records = match eval::evaluate_all(&programs, &translator, args.jobs) {
        Ok(records) => records,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    report(&records, args.quiet);
    if let Some(path) = &args.json {
        let passed = records
            .iter()
            .filter(|r| r.stage == eval::Stage::Pass)
            .count();
        let json = serde_json::json!({
            "versions": eval::versions(),
            "translations": args.translations,
            "passed": passed,
            "total": records.len(),
            "results": records.iter().map(eval::Record::to_json).collect::<Vec<_>>(),
        });
        if let Err(err) = std::fs::write(path, format!("{json:#}\n")) {
            eprintln!("cannot write {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// Builds this checkout's transpiler in the sandbox, in `scratch`.
fn transpiler(scratch: &Path) -> Result<PathBuf, String> {
    let workspace = scratch.join("workspace");
    std::fs::create_dir(&workspace).map_err(|err| err.to_string())?;
    eval::build_transpiler(Path::new(eval::REPO), &workspace)
}

fn report(records: &[eval::Record], quiet: bool) {
    for record in records {
        let fuzz = match &record.fuzz {
            Some((stage, _)) => format!("  fuzz: {stage}"),
            None => String::new(),
        };
        println!(
            "{:>14}  {}  ({:.1}s){fuzz}",
            record.stage, record.name, record.seconds
        );
    }
    if !quiet {
        for record in records
            .iter()
            .filter(|r| r.stage != eval::Stage::Pass && !r.detail.is_empty())
        {
            println!(
                "\n===== {}: {} =====\n{}",
                record.name,
                record.stage,
                record.detail.trim_end()
            );
        }
        for record in records {
            if let Some((stage, detail)) = &record.fuzz
                && *stage != eval::Stage::Pass
            {
                println!(
                    "\n===== {}: fuzz {stage} =====\n{}",
                    record.name,
                    detail.trim_end()
                );
            }
        }
    }
    let mut groups: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for record in records {
        let entry = groups.entry(record.group.as_str()).or_default();
        entry.0 += usize::from(record.stage == eval::Stage::Pass);
        entry.1 += 1;
    }
    println!();
    for (group, (passed, total)) in &groups {
        println!("{group}: {passed}/{total}");
    }
    let passed = records
        .iter()
        .filter(|r| r.stage == eval::Stage::Pass)
        .count();
    println!("SCORE {passed}/{}", records.len());
    let (held, fuzzed) = eval::fuzz_counts(records);
    println!("FUZZ {held}/{fuzzed}");
}
