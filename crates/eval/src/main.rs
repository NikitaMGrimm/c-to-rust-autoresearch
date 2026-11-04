use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const USAGE: &str = "usage: eval [BENCH_DIR] [--only TEXT] [--json FILE] [--jobs N] [--quiet]\n\
     BENCH_DIR defaults to local/bench/dev; program names start with their group, e.g. g1_gcd.c";

struct Args {
    bench: PathBuf,
    only: Option<String>,
    json: Option<PathBuf>,
    jobs: usize,
    quiet: bool,
}

fn parse_args() -> Option<Args> {
    let mut args = Args {
        bench: PathBuf::from("local/bench/dev"),
        only: None,
        json: None,
        // Shared machines: two parallel programs by default.
        jobs: 2,
        quiet: false,
    };
    let mut rest = std::env::args().skip(1);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--only" => args.only = Some(rest.next()?),
            "--json" => args.json = Some(rest.next()?.into()),
            "--jobs" => args.jobs = rest.next()?.parse().ok().filter(|&n| n > 0)?,
            "--quiet" => args.quiet = true,
            _ if arg.starts_with('-') => return None,
            _ => args.bench = arg.into(),
        }
    }
    Some(args)
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut programs: Vec<PathBuf> = match std::fs::read_dir(&args.bench) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                args.only
                    .as_ref()
                    .is_none_or(|only| name.contains(only.as_str()))
            })
            .collect(),
        Err(err) => {
            eprintln!("cannot read {}: {err}", args.bench.display());
            return ExitCode::FAILURE;
        }
    };
    programs.sort();
    if programs.is_empty() {
        eprintln!("no .c programs in {}", args.bench.display());
        return ExitCode::FAILURE;
    }
    let transpiler = match eval::build_transpiler() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let scratch = std::env::temp_dir().join(format!("c2r-eval-{}", std::process::id()));
    let next = AtomicUsize::new(0);
    let records = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.min(programs.len()) {
            scope.spawn(|| {
                while let Some(program) = programs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let work = scratch.join(program.file_stem().unwrap_or_default());
                    std::fs::create_dir_all(&work).expect("cannot create scratch directory");
                    let record = eval::evaluate(program, &transpiler, &work);
                    records.lock().unwrap().push(record);
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&scratch);
    let mut records = records.into_inner().unwrap();
    records.sort_by(|a, b| a.name.cmp(&b.name));

    report(&records, args.quiet);
    if let Some(path) = &args.json {
        let passed = records.iter().filter(|r| r.stage == "pass").count();
        let json = serde_json::json!({
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

fn report(records: &[eval::Record], quiet: bool) {
    for record in records {
        println!(
            "{:>14}  {}  ({:.1}s)",
            record.stage, record.name, record.seconds
        );
    }
    if !quiet {
        for record in records
            .iter()
            .filter(|r| r.stage != "pass" && !r.detail.is_empty())
        {
            println!(
                "\n===== {}: {} =====\n{}",
                record.name,
                record.stage,
                record.detail.trim_end()
            );
        }
    }
    let mut groups: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for record in records {
        let entry = groups.entry(record.group.as_str()).or_default();
        entry.0 += usize::from(record.stage == "pass");
        entry.1 += 1;
    }
    println!();
    for (group, (passed, total)) in &groups {
        println!("{group}: {passed}/{total}");
    }
    let passed = records.iter().filter(|r| r.stage == "pass").count();
    println!("SCORE {passed}/{}", records.len());
}
