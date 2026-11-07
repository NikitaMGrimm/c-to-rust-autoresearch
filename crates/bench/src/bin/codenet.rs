//! Samples programs from IBM's Project CodeNet into the private transfer set,
//! which shows whether rules learned on TRACTOR carry over to programs from
//! another source. No loop sees it or selects on it. Families are taken in a
//! seeded order, and a family's program is the first of its accepted C
//! solutions, in a seeded order, that is a valid reference on CodeNet's sample
//! input and prints CodeNet's expected output. Only counts are printed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: codenet CODENET_DIR PRIVATE_DIR\n\
     CODENET_DIR is an unpacked Project_CodeNet; keep it outside this repository.\n\
     Writes PRIVATE_DIR/transfer and PRIVATE_DIR/codenet.json.";
/// Programs in the set.
const SIZE: usize = 200;
/// Solutions tried per family, so that a problem whose solutions all fail
/// costs a bounded time.
const TRIES: usize = 10;
const NOTICE: &str = "Programs and inputs from IBM's Project CodeNet,
https://github.com/IBM/Project_CodeNet, under the Community Data License
Agreement - Permissive, Version 2.0, https://cdla.dev/permissive-2-0/.
";

struct Solution {
    problem: String,
    submission: String,
    path: PathBuf,
}

struct Program {
    name: String,
    solution: String,
    source: Vec<u8>,
    stdin: String,
    signals: Vec<&'static str>,
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [codenet, private] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match sample(codenet, private, SIZE) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn sample(codenet: &Path, private: &Path, size: usize) -> Result<(), String> {
    let seed = bench::mix(bench::seed(private)? ^ bench::fnv("codenet"));
    let families = families(codenet)?;
    let mut order: Vec<&String> = families.keys().collect();
    order.sort_by_key(|family| (bench::mix(seed ^ bench::fnv(family)), *family));
    let datasets = datasets(codenet)?;

    let scratch = eval::Scratch::new("codenet")?;
    let mut programs = Vec::new();
    let mut excluded: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut tried = 0;
    for family in order {
        if programs.len() == size {
            break;
        }
        tried += 1;
        let mut solutions: Vec<&Solution> = families[family].iter().collect();
        solutions.sort_by_key(|solution| {
            let key = format!("{}/{}", solution.problem, solution.submission);
            (bench::mix(seed ^ bench::fnv(&key)), key)
        });
        let mut found = None;
        for solution in solutions.into_iter().take(TRIES) {
            let work = scratch.join(&solution.submission);
            match check(codenet, family, solution, &work)? {
                Ok(program) => {
                    found = Some((program, &solution.problem));
                    break;
                }
                Err(reason) => *excluded.entry(reason).or_default() += 1,
            }
        }
        match found {
            Some(found) => programs.push(found),
            None => *excluded.entry("family_without_program").or_default() += 1,
        }
    }
    if programs.is_empty() {
        return Err("no CodeNet program passed; check the excluded reasons".to_owned());
    }

    let dir = private.join("transfer");
    clear(&dir)?;
    write(&dir.join("NOTICE"), NOTICE)?;
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_dataset: BTreeMap<&str, usize> = BTreeMap::new();
    for (program, problem) in &programs {
        write_program(program, &dir)?;
        *groups.entry(program.name[..2].to_owned()).or_default() += 1;
        let dataset = datasets.get(*problem).map_or("?", String::as_str);
        *by_dataset.entry(dataset).or_default() += 1;
    }
    let report = serde_json::json!({
        "source": "IBM Project CodeNet",
        "size": size,
        "tries": TRIES,
        "families": families.len(),
        "families_tried": tried,
        "programs": programs.iter().map(|(program, problem)| serde_json::json!({
            "name": program.name,
            "source": program.solution,
            "dataset": datasets.get(*problem),
            "signals": program.signals,
        })).collect::<Vec<_>>(),
        "excluded": excluded,
    });
    write(&private.join("codenet.json"), &format!("{report:#}\n"))?;

    println!(
        "transfer: {} programs from {tried} of {} families",
        programs.len(),
        families.len()
    );
    println!("excluded: {excluded:?}");
    println!("groups: {groups:?}");
    println!("datasets: {by_dataset:?}");
    Ok(())
}

/// The accepted C solutions of every problem with a verified sample input and
/// output, by family: identical problems form one family, named after its
/// first problem.
fn families(codenet: &Path) -> Result<BTreeMap<String, Vec<Solution>>, String> {
    let io = codenet.join("derived/input_output");
    let unverified: BTreeSet<String> = read(&io.join("unverified_accepted_solutions.txt"))?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let clusters = read(&codenet.join("derived/duplicates/identical_problem_clusters"))?;
    let family = family_names(&clusters);
    let mut families: BTreeMap<String, Vec<Solution>> = BTreeMap::new();
    for dir in eval::entries(&codenet.join("data"))? {
        let problem = file_name(&dir);
        let sample = io.join("data").join(&problem);
        if !dir.join("C").is_dir()
            || unverified.contains(&problem)
            || !sample.join("input.txt").is_file()
            || !sample.join("output.txt").is_file()
        {
            continue;
        }
        let metadata = read(&codenet.join("metadata").join(format!("{problem}.csv")))?;
        let mut rows = metadata.lines().map(fields);
        let header = rows.next().unwrap_or_default();
        let column = |name: &str| {
            header
                .iter()
                .position(|field| field == name)
                .ok_or_else(|| format!("{problem}.csv has no column {name}"))
        };
        let (id, language, extension, status) = (
            column("submission_id")?,
            column("language")?,
            column("filename_ext")?,
            column("status")?,
        );
        for row in rows {
            let field = |i: usize| row.get(i).map_or("", String::as_str);
            if field(language) != "C" || field(status) != "Accepted" {
                continue;
            }
            let path = dir
                .join("C")
                .join(format!("{}.{}", field(id), field(extension)));
            if path.is_file() {
                let name = family.get(&problem).unwrap_or(&problem);
                families.entry(name.clone()).or_default().push(Solution {
                    problem: problem.clone(),
                    submission: field(id).to_owned(),
                    path,
                });
            }
        }
    }
    Ok(families)
}

/// For each problem in a cluster of identical problems, the cluster's
/// smallest problem ID.
fn family_names(clusters: &str) -> BTreeMap<String, String> {
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    fn root(parent: &BTreeMap<String, String>, mut id: String) -> String {
        while let Some(next) = parent.get(&id) {
            id = next.clone();
        }
        id
    }
    for line in clusters.lines() {
        let ids: Vec<String> = line.split(',').map(|id| id.trim().to_owned()).collect();
        for pair in ids.windows(2) {
            let (a, b) = (
                root(&parent, pair[0].clone()),
                root(&parent, pair[1].clone()),
            );
            // The smaller ID is the root, so the root is the cluster's smallest.
            match a.cmp(&b) {
                std::cmp::Ordering::Less => parent.insert(b, a),
                std::cmp::Ordering::Greater => parent.insert(a, b),
                std::cmp::Ordering::Equal => None,
            };
        }
    }
    let ids: Vec<String> = parent.keys().cloned().collect();
    ids.into_iter()
        .map(|id| (id.clone(), root(&parent, id)))
        .collect()
}

/// Each problem's dataset, AIZU or AtCoder, for the report.
fn datasets(codenet: &Path) -> Result<BTreeMap<String, String>, String> {
    let list = read(&codenet.join("metadata/problem_list.csv"))?;
    let mut rows = list.lines().map(fields);
    let header = rows.next().unwrap_or_default();
    let column = |name: &str| header.iter().position(|field| field == name);
    let (Some(id), Some(dataset)) = (column("id"), column("dataset")) else {
        return Err("problem_list.csv has no id or dataset column".to_owned());
    };
    Ok(rows
        .filter_map(|row| Some((row.get(id)?.clone(), row.get(dataset)?.clone())))
        .collect())
}

/// The fields of a CSV line; a quoted field may hold commas and doubled quotes.
fn fields(line: &str) -> Vec<String> {
    let (mut fields, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields
}

/// The solution as a program of the transfer set, or why not.
fn check(
    codenet: &Path,
    family: &str,
    solution: &Solution,
    work: &Path,
) -> Result<Result<Program, &'static str>, String> {
    std::fs::create_dir_all(work)
        .map_err(|err| format!("cannot create {}: {err}", work.display()))?;
    let sample = codenet
        .join("derived/input_output/data")
        .join(&solution.problem);
    let source = std::fs::read(&solution.path)
        .map_err(|err| format!("cannot read {}: {err}", solution.path.display()))?;
    let Ok(text) = std::str::from_utf8(&source) else {
        return Ok(Err("source_not_utf8"));
    };
    // The spec holds standard input as text.
    let Ok(stdin) = String::from_utf8(read_bytes(&sample.join("input.txt"))?) else {
        return Ok(Err("input_not_utf8"));
    };
    let expected = read_bytes(&sample.join("output.txt"))?;
    let input = eval::Input {
        name: "sample".to_owned(),
        args: Vec::new(),
        stdin: stdin.clone().into_bytes(),
    };
    if bench::parses_numbers_unchecked(text) && !bench::numbers_fit_int(&input) {
        return Ok(Err("numbers_beyond_int"));
    }
    let c_file = work.join("main.c");
    std::fs::write(&c_file, &source)
        .map_err(|err| format!("cannot write {}: {err}", c_file.display()))?;
    let Ok(references) = eval::References::build(&c_file, &[], work) else {
        return Ok(Err("c_build"));
    };
    let run = match references.check(&input) {
        Ok(run) => run,
        Err(reason) => return Ok(Err(reason)),
    };
    // Online judges accept trailing spaces and blank lines.
    if judged(&run.stdout) != judged(&expected) {
        return Ok(Err("c_differs_from_expected"));
    }
    let (group, signals) = bench::classify(&c_file)?;
    Ok(Ok(Program {
        name: format!("g{group}_cn_{family}"),
        solution: format!("{}/{}", solution.problem, solution.submission),
        source,
        stdin,
        signals,
    }))
}

/// Output as an online judge compares it: without carriage returns, trailing
/// spaces and trailing blank lines.
fn judged(output: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(output).replace("\r\n", "\n");
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn write_program(program: &Program, dir: &Path) -> Result<(), String> {
    let base = dir.join(&program.name);
    let c_file = base.with_extension("c");
    std::fs::write(&c_file, &program.source)
        .map_err(|err| format!("cannot write {}: {err}", c_file.display()))?;
    let json = serde_json::json!({
        "source": format!("Project CodeNet {}", program.solution),
        "signals": program.signals,
        "inputs": [{"name": "sample", "args": [], "stdin": program.stdin}],
    });
    write(&base.with_extension("json"), &format!("{json:#}\n"))
}

/// Removes earlier programs and their inputs, so a rerun leaves no stale ones.
fn clear(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    for path in eval::entries(dir)? {
        let name = file_name(&path);
        if name.starts_with('g') && (name.ends_with(".c") || name.ends_with(".json")) {
            std::fs::remove_file(&path)
                .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("cannot read {}: {err}", path.display()))
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_keep_quoted_commas_and_quotes() {
        assert_eq!(
            fields(r#"p1,"A, ""B""",AtCoder"#),
            ["p1", r#"A, "B""#, "AtCoder"]
        );
        assert_eq!(fields("a,,b"), ["a", "", "b"]);
    }

    #[test]
    fn identical_problems_are_named_after_the_smallest() {
        let names = family_names("p3,p2\np5,p3\np7\n");
        assert_eq!(names["p3"], "p2");
        assert_eq!(names["p5"], "p2");
        assert!(!names.contains_key("p7"));
    }

    #[test]
    fn output_is_compared_as_a_judge_does() {
        assert_eq!(judged(b"1 2 \r\n3\n\n"), judged(b"1 2\n3"));
        assert_ne!(judged(b"1 2\n3"), judged(b"1  2\n3"));
    }

    /// A CodeNet tree with problems p1 to p4: p2 is identical to p1, p3's
    /// sample is unverified and p4 has no accepted C solution.
    fn codenet(dir: &Path) {
        let solution = |problem: &str, id: &str, status: &str, source: &str| {
            let c = dir.join("data").join(problem).join("C");
            std::fs::create_dir_all(&c).unwrap();
            std::fs::write(c.join(format!("{id}.c")), source).unwrap();
            format!("{id},{problem},u1,0,C,C (GCC),c,{status},0,0,1,1/1\n")
        };
        let header = "submission_id,problem_id,user_id,date,language,original_language,filename_ext,status,cpu_time,memory,code_size,accuracy\n";
        let doubled = "#include <stdio.h>\nint main(void) { int n; scanf(\"%d\", &n); printf(\"%d \\n\", 2 * n); return 0; }\n";
        let wrong = "#include <stdio.h>\nint main(void) { printf(\"0\\n\"); return 0; }\n";
        let metadata = dir.join("metadata");
        std::fs::create_dir_all(&metadata).unwrap();
        let rows = [
            ("p1", vec![solution("p1", "s1", "Accepted", wrong)]),
            ("p2", vec![solution("p2", "s2", "Accepted", doubled)]),
            ("p3", vec![solution("p3", "s3", "Accepted", doubled)]),
            ("p4", vec![solution("p4", "s4", "Wrong Answer", doubled)]),
        ];
        for (problem, rows) in rows {
            std::fs::write(
                metadata.join(format!("{problem}.csv")),
                format!("{header}{}", rows.concat()),
            )
            .unwrap();
            let sample = dir.join("derived/input_output/data").join(problem);
            std::fs::create_dir_all(&sample).unwrap();
            std::fs::write(sample.join("input.txt"), "21\n").unwrap();
            std::fs::write(sample.join("output.txt"), "42\n").unwrap();
        }
        std::fs::write(
            metadata.join("problem_list.csv"),
            "id,name,dataset\np1,\"One, two\",AtCoder\np2,Two,AtCoder\np3,Three,AIZU\np4,Four,AIZU\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("derived/input_output/unverified_accepted_solutions.txt"),
            "p3\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("derived/duplicates")).unwrap();
        std::fs::write(
            dir.join("derived/duplicates/identical_problem_clusters"),
            "p1,p2\n",
        )
        .unwrap();
    }

    #[test]
    fn a_family_gets_the_first_solution_that_passes() {
        let scratch = eval::Scratch::new("codenet-test").unwrap();
        let (tree, private) = (scratch.join("codenet"), scratch.join("private"));
        codenet(&tree);
        std::fs::create_dir_all(private.join("transfer")).unwrap();
        std::fs::write(private.join("seed"), "1f\n").unwrap();
        std::fs::write(private.join("transfer/g1_cn_old.c"), "stale").unwrap();
        sample(&tree, &private, SIZE).unwrap();

        // p1 and p2 are one family, in which only p2's solution is right.
        // p3's sample is unverified and p4 has no accepted solution.
        let mut names: Vec<String> = eval::entries(&private.join("transfer"))
            .unwrap()
            .iter()
            .map(|path| file_name(path))
            .collect();
        names.sort();
        assert_eq!(names, ["NOTICE", "g1_cn_p1.c", "g1_cn_p1.json"]);
        let spec = eval::spec(&private.join("transfer/g1_cn_p1.c")).unwrap();
        assert_eq!(spec.inputs.len(), 1);
        assert_eq!(spec.inputs[0].stdin, b"21\n");
        let report: serde_json::Value =
            serde_json::from_str(&read(&private.join("codenet.json")).unwrap()).unwrap();
        assert_eq!(report["programs"][0]["source"], "p2/s2");
        assert_eq!(report["programs"][0]["dataset"], "AtCoder");
        assert_eq!(report["families"], 1);
        let first = read(&private.join("codenet.json")).unwrap();
        // The same seed writes the same set.
        sample(&tree, &private, SIZE).unwrap();
        assert_eq!(read(&private.join("codenet.json")).unwrap(), first);
    }
}
