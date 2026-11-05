//! The loop agent's settings from loop/setup and the hook they install, next
//! to the audit that checks the same calls afterwards.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::json;

/// Exit code of loop/guard for a hook input, with `/p/holdout` denied and
/// `/h/.claude/projects/-loop` as the agent's own transcripts.
fn guard_input(input: serde_json::Value) -> Option<i32> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../loop/guard");
    let mut child = Command::new(script)
        .args([
            "/h/.claude/projects/-loop",
            "/p/holdout",
            "holdout",
            ".cache/c-to-rust-autoresearch",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.to_string().as_bytes()).unwrap();
    drop(stdin);
    child.wait().unwrap().code()
}

fn guard(tool: &str, input: serde_json::Value) -> Option<i32> {
    guard_input(json!({"tool_name": tool, "tool_input": input}))
}

#[test]
fn calls_reaching_private_places_are_blocked() {
    let blocked = [
        ("Bash", json!({"command": "cat ../holdout/validation/a.c"})),
        (
            "Bash",
            json!({"command": "cd\n.cache/c-to-rust-autoresearch"}),
        ),
        (
            "Bash",
            json!({"command": "ls ~/.cache/c-to-rust-autoresearch"}),
        ),
        ("Grep", json!({"pattern": "main", "path": "/p/holdout"})),
        (
            "WebFetch",
            json!({"url": "https://x.org/DARPA-TRACTOR-Program"}),
        ),
        ("Edit", json!({"file_path": ".claude/settings.local.json"})),
        ("Read", json!({"file_path": ".claude/settings.local.json"})),
        (
            "Read",
            json!({"file_path": "/h/.claude/projects/-main/s.jsonl"}),
        ),
        (
            "Bash",
            json!({"command": "cat /h/.claude/projects/-loop/s.jsonl"}),
        ),
        (
            "Bash",
            json!({"command": "cat ~/.claude/skills/../settings.json"}),
        ),
        (
            "Read",
            json!({"file_path": "/h/.claude/projects/-loop/../-main/s.jsonl"}),
        ),
    ];
    for (tool, input) in blocked {
        assert_eq!(guard(tool, input.clone()), Some(2), "{tool} {input}");
    }
    assert_eq!(guard_input(json!({"tool_input": {}})), Some(2));
    assert_eq!(guard_input(json!("not a call")), Some(2));
}

#[test]
fn ordinary_calls_pass() {
    let allowed = [
        ("Bash", json!({"command": "cargo test -p transpiler"})),
        ("Bash", json!({"command": "ls /p/holdout-two"})),
        ("Read", json!({"file_path": "bench/LICENSE-TRACTOR"})),
        (
            "Read",
            json!({"file_path": "/h/.claude/projects/-loop/s/tool-results/a.txt"}),
        ),
        ("Read", json!({"file_path": "/h/.claude/skills/x/SKILL.md"})),
        (
            "Bash",
            json!({"command": "echo https://code.claude.com/docs"}),
        ),
    ];
    for (tool, input) in allowed {
        assert_eq!(guard(tool, input.clone()), Some(0), "{tool} {input}");
    }
}

/// Claude Code's transcript directory for a project at `path`.
fn transcripts(home: &Path, path: &Path) -> String {
    let name: String = path
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{}/.claude/projects/{name}", home.display())
}

#[test]
fn setup_keeps_the_agent_out_of_the_main_checkout_and_off_the_web() {
    let dir = eval::Scratch::new("test").unwrap();
    let dir = dir.canonicalize().unwrap();
    let (repo, home) = (dir.join("repo"), dir.join("home"));
    // CI checks out no main branch, so setup runs from a copy with one.
    std::fs::create_dir_all(repo.join("loop")).unwrap();
    std::fs::create_dir_all(dir.join("private")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    for script in ["setup", "guard"] {
        let source = format!("{}/../../loop/{script}", env!("CARGO_MANIFEST_DIR"));
        std::fs::copy(source, repo.join("loop").join(script)).unwrap();
    }
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "loop"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "--no-verify",
        "-m",
        "loop",
    ]);
    let status = Command::new(repo.join("loop/setup"))
        .args([dir.join("private"), dir.join("loop")])
        .env("HOME", &home)
        .env_remove("XDG_CACHE_HOME")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());

    let settings = std::fs::read_to_string(dir.join("loop/.claude/settings.local.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
    let deny: Vec<&str> = settings["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule.as_str().unwrap())
        .collect();
    let home_dir = home.display();
    for rule in [
        format!("Read(/{}/**)", repo.display()),
        format!("Edit(/{}/**)", repo.display()),
        format!("Read(/{}/**)", transcripts(&home, &repo)),
        format!("Read(/{}/**)", dir.join("private").display()),
        format!("Read(/{home_dir}/.cache/c-to-rust-autoresearch/**)"),
        format!("Edit(/{home_dir}/.claude/**)"),
        "WebFetch".to_owned(),
        "WebSearch".to_owned(),
    ] {
        assert!(deny.contains(&rule.as_str()), "{rule} not in {deny:?}");
    }
    let hook = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let own = transcripts(&home, &dir.join("loop"));
    assert!(
        hook.starts_with(&format!("'{}/loop/guard' '{own}' ", repo.display())),
        "{hook}"
    );
    let marked = Command::new("git")
        .args(["config", "c2r.loop"])
        .current_dir(dir.join("loop"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&marked.stdout).trim(), "true");
}
