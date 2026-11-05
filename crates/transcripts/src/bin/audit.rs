//! Checks the loop agent's transcripts for contact with the private sets: the
//! name of a private program anywhere in what it saw or wrote, and a tool call
//! that names the private directory or the TRACTOR corpus, or changes Claude
//! Code settings. Prints each finding and a summary line.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: audit PRIVATE_DIR PATH...\n\
     PATH is a transcript (.jsonl) or a directory searched for them, such as the\n\
     loop checkout's directory under ~/.claude/projects. Exit code 1 means findings.";
/// Text in a tool call that reaches for the TRACTOR corpus, which holds the
/// private programs too: its repository and the cache `intake` keeps it in.
const CORPUS: [&str; 3] = [
    "DARPA-TRACTOR-Program",
    "PUBLIC-Test-Corpus",
    ".cache/c-to-rust-autoresearch",
];
/// Tools whose input may change a file.
const WRITERS: [&str; 4] = ["Bash", "Edit", "Write", "NotebookEdit"];

struct Patterns {
    /// Names of the private programs and their variants.
    names: Vec<String>,
    /// The private directory, as an absolute path and by its own name.
    private: [String; 2],
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [private, paths @ ..] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if paths.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    match audit(private, paths) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

/// Prints the findings in the transcripts under `paths` and returns how many.
fn audit(private: &Path, paths: &[PathBuf]) -> Result<usize, String> {
    let patterns = patterns(private)?;
    let files = transcripts::find(paths)?;
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
    let private = private
        .canonicalize()
        .map_err(|err| format!("cannot open {}: {err}", private.display()))?;
    let mut dirs = vec![private.join("validation"), private.join("held-out")];
    let variants = private.join("variants");
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
        return Err(format!("no private programs in {}", private.display()));
    }
    let own_name = private.file_name().unwrap_or_default().to_string_lossy();
    Ok(Patterns {
        names,
        private: [private.display().to_string(), own_name.into_owned()],
    })
}

/// Findings in one transcript, with their line numbers.
fn findings(text: &str, patterns: &Patterns) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Request records repeat the conversation that these records hold.
        if !matches!(
            record["type"].as_str(),
            Some("user" | "assistant" | "attachment")
        ) {
            continue;
        }
        let mut add = |finding: String| found.push((number + 1, finding));
        for name in &patterns.names {
            if contains_word(line, name) {
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
            let input = call["input"].to_string();
            // The private directory's path contains its name; one finding a call.
            let mut markers = patterns.private.iter().map(String::as_str).chain(CORPUS);
            if let Some(marker) = markers.find(|marker| input.contains(marker)) {
                add(format!("{tool} names {marker}"));
            }
            if tool.starts_with("Web") && input.to_lowercase().contains("tractor") {
                add(format!("{tool} looks for TRACTOR"));
            }
            if WRITERS.contains(&tool) && input.contains(".claude/settings") {
                add(format!("{tool} touches Claude Code settings"));
            }
        }
    }
    found
}

/// Whether `word` occurs in `text` as a whole identifier, so that a dev
/// program whose name extends a private one does not count.
fn contains_word(text: &str, word: &str) -> bool {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(identifier) && !after.is_some_and(identifier)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns() -> Patterns {
        Patterns {
            names: vec!["g1_secret".to_owned(), "g2_dev__r".to_owned()],
            private: ["/p/holdout".to_owned(), "holdout".to_owned()],
        }
    }

    fn tool_use(name: &str, input: serde_json::Value) -> String {
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

    #[test]
    fn private_names_and_reaching_tool_calls_are_found() {
        let lines = [
            tool_use(
                "Bash",
                serde_json::json!({"command": "cat /p/holdout/validation/g1_secret.c"}),
            ),
            tool_result("int main(void) { /* g2_dev__r */ }"),
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
        ];
        let found = findings(&lines.join("\n"), &patterns());
        let expected = [
            (1, "private program g1_secret"),
            (1, "Bash names /p/holdout"),
            (2, "private program g2_dev__r"),
            (3, "WebSearch looks for TRACTOR"),
            (4, "Edit touches Claude Code settings"),
            (5, "WebFetch names DARPA-TRACTOR-Program"),
            (5, "WebFetch looks for TRACTOR"),
        ];
        let expected: Vec<(usize, String)> = expected
            .iter()
            .map(|&(line, finding)| (line, finding.to_owned()))
            .collect();
        assert_eq!(found, expected);
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
            tool_use(
                "Read",
                serde_json::json!({"file_path": ".claude/settings.local.json"}),
            ),
            serde_json::json!({"type": "api-request-blob", "text": "g1_secret"}).to_string(),
        ];
        assert_eq!(findings(&lines.join("\n"), &patterns()), []);
    }

    #[test]
    fn names_come_from_the_private_sets_and_their_variants() {
        let dir = eval::Scratch::new("test").unwrap();
        for file in [
            "validation/a.c",
            "held-out/b.c",
            "variants/dev/c__r.c",
            "variants/dev/c__r.json",
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let found = super::patterns(&dir).unwrap();
        assert_eq!(found.names, ["a", "b", "c__r"]);
        assert_eq!(
            found.private[0],
            dir.canonicalize().unwrap().display().to_string()
        );
    }
}
