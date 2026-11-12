//! Checks the loop agent's transcripts for contact with the private sets: the
//! name of a private program anywhere in what it saw or wrote, and a tool call
//! that names the private directory or a corpus that holds private programs,
//! TRACTOR or CodeNet, or touches Claude Code's files. Prints each finding and
//! a summary line.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

const USAGE: &str = "usage: audit PRIVATE_DIR [--readable DIR]... PATH...\n\
     PATH is a transcript (.jsonl) or a directory searched for them, such as the\n\
     loop checkout's directory under ~/.claude/projects; reading files back from\n\
     these directories is no finding, nor from a DIR among Claude Code's files\n\
     that the agent may read, such as its skills. Exit code 1 means findings.";
/// Text in a tool call that reaches for a corpus that holds private programs
/// too: TRACTOR's repository, CodeNet's archive and the cache that `intake`
/// and `codenet` read them from.
/// A name runs on into a dot, so CodeNet's archive and repository are named in
/// full.
const CORPUS: [&str; 6] = [
    "DARPA-TRACTOR-Program",
    "PUBLIC-Test-Corpus",
    "Project_CodeNet",
    "Project_CodeNet.tar.gz",
    "Project_CodeNet.git",
    ".cache/c-to-rust-autoresearch",
];

struct Patterns {
    /// Names of the private programs and their variants.
    names: Vec<String>,
    /// Paths and names a tool call may not contain: the private directory as
    /// given and resolved, each by its own name too, and the corpus.
    places: Vec<String>,
    /// The audited transcripts' directories, which Claude Code saves long
    /// tool output to and the agent may read back, and any other directory
    /// it may read.
    own: Vec<String>,
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (mut paths, mut readable) = (Vec::new(), Vec::new());
    let private = args.next();
    while let Some(arg) = args.next() {
        if arg.as_os_str() == "--readable" {
            let Some(dir) = args.next() else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            readable.push(dir);
        } else {
            paths.push(arg);
        }
    }
    let (Some(private), false) = (private, paths.is_empty()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match audit(&private, &paths, &readable) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

/// Prints the findings in the transcripts under `paths` and returns how many.
/// Reading files in a `readable` directory is no finding.
fn audit(private: &Path, paths: &[PathBuf], readable: &[PathBuf]) -> Result<usize, String> {
    let mut patterns = patterns(private)?;
    // A call may name the directory as given or, where that is a link, as
    // resolved.
    for dir in readable {
        let given = std::path::absolute(dir).map_err(|err| err.to_string())?;
        let resolved = dir.canonicalize().unwrap_or_else(|_| given.clone());
        for dir in [given, resolved] {
            patterns.own.push(dir.display().to_string());
        }
    }
    for path in paths {
        let path = path
            .canonicalize()
            .map_err(|err| format!("cannot open {}: {err}", path.display()))?;
        let dir = if path.is_dir() {
            &path
        } else {
            path.parent().unwrap_or(&path)
        };
        patterns.own.push(dir.display().to_string());
    }
    let files = transcripts::find(paths)?;
    if files.is_empty() {
        // A wrong path would otherwise pass as a clean audit.
        return Err("no transcripts to audit".to_owned());
    }
    let (mut total, mut flagged) = (0, 0);
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|err| format!("cannot read {}: {err}", file.display()))?;
        let found = findings(&text, &patterns);
        for (line, finding) in &found {
            println!("{}:{line}: {finding}", file.display());
        }
        flagged += usize::from(!found.is_empty());
        total += found.len();
    }
    if total == 0 {
        println!("AUDIT clean: {} transcripts", files.len());
    } else {
        println!(
            "AUDIT {total} findings in {flagged} of {} transcripts",
            files.len()
        );
    }
    Ok(total)
}

fn patterns(private: &Path) -> Result<Patterns, String> {
    let open = |err| format!("cannot open {}: {err}", private.display());
    let given = std::path::absolute(private).map_err(open)?;
    let resolved = private.canonicalize().map_err(open)?;
    let mut dirs = vec![resolved.join("validation"), resolved.join("held-out")];
    if resolved.join("transfer").is_dir() {
        dirs.push(resolved.join("transfer"));
    }
    let variants = resolved.join("variants");
    if variants.exists() {
        dirs.extend(eval::entries(&variants)?.into_iter().filter(|p| p.is_dir()));
    }
    let mut names = Vec::new();
    for dir in &dirs {
        for program in eval::c_programs(dir)? {
            let name = program.file_stem().unwrap_or_default().to_string_lossy();
            names.push(name.into_owned());
        }
    }
    if names.is_empty() {
        return Err(format!("no private programs in {}", resolved.display()));
    }
    // Whole paths first, so that a call is reported by the longest it names.
    let mut places: Vec<String> = [&given, &resolved]
        .map(|path| path.display().to_string())
        .into_iter()
        .chain([&given, &resolved].map(|path| {
            let name = path.file_name().unwrap_or_default();
            name.to_string_lossy().into_owned()
        }))
        .chain(CORPUS.map(str::to_owned))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    places.retain(|place| !place.is_empty() && seen.insert(place.clone()));
    Ok(Patterns {
        names,
        places,
        own: Vec::new(),
    })
}

/// Findings in one transcript, with their line numbers.
fn findings(text: &str, patterns: &Patterns) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let mut add = |finding: String| found.push((number + 1, finding));
        // A record the audit cannot read could hide anything.
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            add("unreadable record".to_owned());
            continue;
        };
        // Request records repeat the conversation that these records hold.
        if !matches!(
            record["type"].as_str(),
            Some("user" | "assistant" | "attachment")
        ) {
            continue;
        }
        let texts = strings(&record);
        for name in &patterns.names {
            if texts.iter().any(|text| contains(text, name, identifier)) {
                add(format!("private program {name}"));
            }
        }
        let content = record["message"]["content"].as_array();
        for call in content
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "tool_use")
        {
            let tool = call["name"].as_str().unwrap_or_default();
            let input = strings(&call["input"]);
            // Every place a call names, since a run sets aside a session whose
            // calls name only places its sandbox hid. A name inside a path
            // already found adds nothing.
            let named = |place: &&String| input.iter().any(|text| contains(text, place, path));
            let mut found: Vec<&String> = Vec::new();
            for place in patterns.places.iter().filter(named) {
                if !found.iter().any(|longer| longer.contains(place.as_str())) {
                    add(format!("{tool} names {place}"));
                    found.push(place);
                }
            }
            for (word, corpus) in [("tractor", "TRACTOR"), ("codenet", "CodeNet")] {
                if tool.starts_with("Web")
                    && input.iter().any(|text| text.to_lowercase().contains(word))
                {
                    add(format!("{tool} looks for {corpus}"));
                }
            }
            let touches = input.iter().any(|text| {
                let mut rest = text.to_string();
                // A path with .. could climb out of the agent's own directory.
                if tool == "Read" && !text.contains("..") {
                    for dir in &patterns.own {
                        rest = without(&rest, dir);
                    }
                }
                contains(&rest, ".claude", path)
            });
            if touches {
                add(format!("{tool} touches Claude Code's files"));
            }
        }
    }
    found
}

/// `text` without the directory `dir` where it names it, and not a sibling
/// whose name extends it.
fn without(text: &str, dir: &str) -> String {
    let (mut rest, mut tail) = (String::new(), text);
    while let Some(at) = tail.find(dir).filter(|_| !dir.is_empty()) {
        let after = &tail[at + dir.len()..];
        rest.push_str(&tail[..at]);
        if after.chars().next().is_some_and(path) {
            rest.push_str(dir);
        }
        tail = after;
    }
    rest + tail
}

/// The strings in `value`, decoded, so that an escape such as `\n` cannot
/// hide a name's start.
fn strings(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text],
        Value::Array(items) => items.iter().flat_map(strings).collect(),
        Value::Object(fields) => fields.values().flat_map(strings).collect(),
        _ => Vec::new(),
    }
}

/// Characters that continue a program name, so that a dev program whose name
/// extends a private one does not count.
fn identifier(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Characters that continue a file name, so that a sibling directory whose
/// name extends the private one does not count.
fn path(c: char) -> bool {
    identifier(c) || c == '.' || c == '-'
}

/// Whether `word` occurs in `text` with no `inside` character on either side.
fn contains(text: &str, word: &str, inside: fn(char) -> bool) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(inside) && !after.is_some_and(inside)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns() -> Patterns {
        Patterns {
            names: vec!["g1_secret".to_owned(), "g2_dev__r".to_owned()],
            places: ["/p/holdout", "holdout"]
                .into_iter()
                .chain(CORPUS)
                .map(str::to_owned)
                .collect(),
            own: vec!["/h/.claude/projects/-loop".to_owned()],
        }
    }

    fn tool_use(name: &str, input: Value) -> String {
        serde_json::json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": name, "input": input}
        ]}})
        .to_string()
    }

    fn tool_result(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "content": text}
        ]}})
        .to_string()
    }

    fn found(lines: &[String]) -> Vec<(usize, String)> {
        findings(&lines.join("\n"), &patterns())
    }

    fn expected(findings: &[(usize, &str)]) -> Vec<(usize, String)> {
        findings
            .iter()
            .map(|&(line, finding)| (line, finding.to_owned()))
            .collect()
    }

    #[test]
    fn private_names_and_reaching_tool_calls_are_found() {
        let lines = [
            tool_use(
                "Bash",
                serde_json::json!({"command": "cat /p/holdout/validation/g1_secret.c"}),
            ),
            tool_result("ls\ng2_dev__r.c"),
            tool_use(
                "WebSearch",
                serde_json::json!({"query": "Tractor test corpus"}),
            ),
            tool_use(
                "Edit",
                serde_json::json!({"file_path": ".claude/settings.local.json"}),
            ),
            tool_use(
                "WebFetch",
                serde_json::json!({"url": "https://github.com/DARPA-TRACTOR-Program/x"}),
            ),
            tool_use(
                "Read",
                serde_json::json!({"file_path": "/h/.claude/settings.json"}),
            ),
            tool_use(
                "Bash",
                serde_json::json!({"command": "cat /h/.claude/projects/-loop/s/tool-results/a.txt"}),
            ),
            tool_use(
                "Read",
                serde_json::json!({"file_path": "/h/.claude/projects/-loop/../settings.json"}),
            ),
            tool_use(
                "Read",
                serde_json::json!({"file_path": "/h/.claude/skills/x/SKILL.md"}),
            ),
            "{\"type\": \"user\", \"message\": ".to_owned(),
            tool_use(
                "Bash",
                serde_json::json!({"command": "cd /p/holdout && fetch https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus"}),
            ),
            tool_use(
                "WebSearch",
                serde_json::json!({"query": "IBM CodeNet p02345"}),
            ),
            tool_use(
                "Bash",
                serde_json::json!({"command": "ls Project_CodeNet/data"}),
            ),
            tool_use(
                "Bash",
                serde_json::json!({"command": "curl -O https://x.org/1.0.0/Project_CodeNet.tar.gz"}),
            ),
        ];
        let want = expected(&[
            (1, "private program g1_secret"),
            (1, "Bash names /p/holdout"),
            (2, "private program g2_dev__r"),
            (3, "WebSearch looks for TRACTOR"),
            (4, "Edit touches Claude Code's files"),
            (5, "WebFetch names DARPA-TRACTOR-Program"),
            (5, "WebFetch looks for TRACTOR"),
            (6, "Read touches Claude Code's files"),
            (7, "Bash touches Claude Code's files"),
            (8, "Read touches Claude Code's files"),
            (9, "Read touches Claude Code's files"),
            (10, "unreadable record"),
            (11, "Bash names /p/holdout"),
            (11, "Bash names DARPA-TRACTOR-Program"),
            (11, "Bash names PUBLIC-Test-Corpus"),
            (12, "WebSearch looks for CodeNet"),
            (13, "Bash names Project_CodeNet"),
            (14, "Bash names Project_CodeNet.tar.gz"),
        ]);
        assert_eq!(found(&lines), want);
    }

    #[test]
    fn dev_programs_docs_and_repeated_requests_are_not_findings() {
        let lines = [
            // A dev program whose name extends a private one.
            tool_use(
                "Read",
                serde_json::json!({"file_path": "bench/dev/g1_secret_two.c"}),
            ),
            // bench/README.md names the corpus; reading it is fine.
            tool_result("https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus"),
            // A sibling of the private directory.
            tool_use("Bash", serde_json::json!({"command": "ls /p/holdout-two"})),
            // Long tool output that Claude Code saved for the agent.
            tool_use(
                "Read",
                serde_json::json!({"file_path": "/h/.claude/projects/-loop/s/tool-results/a.txt"}),
            ),
            tool_use(
                "Bash",
                serde_json::json!({"command": "echo https://code.claude.com/docs"}),
            ),
            serde_json::json!({"type": "api-request-blob", "text": "g1_secret"}).to_string(),
        ];
        assert_eq!(found(&lines), []);
    }

    #[test]
    fn names_come_from_the_private_sets_and_their_variants() {
        let dir = eval::Scratch::new("test").unwrap();
        for file in [
            "validation/a.c",
            "held-out/b.c",
            "transfer/d.c",
            "variants/dev/c__r.c",
            "variants/dev/c__r.json",
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let link = dir.join("link");
        std::os::unix::fs::symlink(&*dir, &link).unwrap();
        let found = super::patterns(&link).unwrap();
        assert_eq!(found.names, ["a", "b", "d", "c__r"]);
        let resolved = dir.canonicalize().unwrap();
        let resolved_name = resolved.file_name().unwrap().to_string_lossy();
        let places = [
            link.display().to_string(),
            resolved.display().to_string(),
            "link".to_owned(),
            resolved_name.into_owned(),
        ];
        assert_eq!(found.places[..4], places);
    }

    #[test]
    fn an_audit_without_transcripts_fails() {
        let dir = eval::Scratch::new("test").unwrap();
        std::fs::create_dir_all(dir.join("private/validation")).unwrap();
        std::fs::create_dir(dir.join("private/held-out")).unwrap();
        std::fs::write(dir.join("private/validation/a.c"), "").unwrap();
        std::fs::create_dir(dir.join("empty")).unwrap();
        let audited = audit(&dir.join("private"), &[dir.join("empty")], &[]);
        assert_eq!(audited, Err("no transcripts to audit".to_owned()));
    }

    #[test]
    fn a_readable_directory_is_read_without_a_finding_and_not_searched() {
        let dir = eval::Scratch::new("test").unwrap();
        std::fs::create_dir_all(dir.join("private/validation")).unwrap();
        std::fs::create_dir(dir.join("private/held-out")).unwrap();
        std::fs::write(dir.join("private/validation/a.c"), "").unwrap();
        // The skills are behind a link and hold a file that is no transcript.
        let (given, real) = (dir.join("home/.claude"), dir.join("real/.claude"));
        std::fs::create_dir_all(real.join("skills/x")).unwrap();
        std::fs::create_dir(dir.join("home")).unwrap();
        std::os::unix::fs::symlink(&real, &given).unwrap();
        std::fs::write(real.join("skills/x/data.jsonl"), "not a transcript\n").unwrap();
        std::fs::create_dir(dir.join("transcripts")).unwrap();
        let read = |path: PathBuf| tool_use("Read", serde_json::json!({"file_path": path}));
        // By the name the directory was given with, and by the one it has.
        let calls = [
            read(given.join("skills/x/SKILL.md")),
            read(real.canonicalize().unwrap().join("skills/x/SKILL.md")),
        ];
        std::fs::write(dir.join("transcripts/s.jsonl"), calls.join("\n")).unwrap();
        let skills = [given.join("skills")];
        let audit = |readable: &[PathBuf]| {
            audit(&dir.join("private"), &[dir.join("transcripts")], readable)
        };
        assert_eq!(audit(&skills), Ok(0));
        assert_eq!(audit(&[]), Ok(2));
        // Not what lies beside it.
        let beside = [
            read(given.join("skills/../settings.json")),
            read(given.join("skills-old/x/SKILL.md")),
        ];
        std::fs::write(dir.join("transcripts/s.jsonl"), beside.join("\n")).unwrap();
        assert_eq!(audit(&skills), Ok(2));
    }
}
