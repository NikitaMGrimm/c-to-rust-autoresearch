//! The loop agent's settings from loop/setup and the hook they install, next
//! to the audit that checks the same calls afterwards.

use std::io::Write;
use std::path::{Path, PathBuf};
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

fn git(dir: &Path, args: &[&str]) -> String {
    let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];
    let unchecked = [
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    let output = Command::new("git")
        .args(identity)
        .args(unchecked)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str) -> String {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", file]);
    git(
        dir,
        &["commit", "-q", "-m", &format!("feat(transpiler): {file}")],
    );
    git(dir, &["rev-parse", "HEAD"])
}

/// A scratch copy of this repository's loop scripts on a main branch, which
/// CI does not check out, with a home directory, a private directory and a
/// fake cargo that stands in for the audit, the usage tool and the gate.
struct Fixture {
    _scratch: eval::Scratch,
    dir: PathBuf,
    repo: PathBuf,
    home: PathBuf,
    loop_dir: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let scratch = eval::Scratch::new("test").unwrap();
        let dir = scratch.canonicalize().unwrap();
        let (repo, home) = (dir.join("repo"), dir.join("home"));
        for path in [
            repo.join("loop/skills/inspect"),
            home.join("notes"),
            home.join(".claude/skills/mine"),
            dir.join("private"),
            dir.join("bin"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        for script in ["setup", "guard", "round"] {
            let source = format!("{}/../../loop/{script}", env!("CARGO_MANIFEST_DIR"));
            std::fs::copy(source, repo.join("loop").join(script)).unwrap();
        }
        std::fs::write(repo.join("loop/skills/inspect/SKILL.md"), "name: inspect\n").unwrap();
        std::fs::write(home.join(".claude/skills/mine/SKILL.md"), "name: mine\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "/local/\n").unwrap();
        let cargo = dir.join("bin/cargo");
        std::fs::write(
            &cargo,
            r#"#!/usr/bin/env bash
echo "$*" >>"$CALLS"
case "$*" in
  *"--bin audit"*) exit "$AUDIT" ;;
  *"--bin usage"*) echo '{}' ;;
  *"--bin gate"*)
    echo "dev: 1 -> 2 of 3"
    if ((GATE == 0)); then echo keep; else echo "reject: the development score did not rise"; fi
    exit "$GATE" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&cargo, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "loop"]);
        let loop_dir = dir.join("loop");
        Fixture {
            _scratch: scratch,
            dir,
            repo,
            home,
            loop_dir,
        }
    }

    /// Runs a loop script with the private directory, the loop checkout and
    /// `notes` as a denied directory, and returns its exit code.
    fn run(&self, script: &str, audit: u8, gate: u8) -> Option<i32> {
        let path = format!("{}:{}", self.dir.join("bin").display(), env!("PATH"));
        Command::new(self.repo.join("loop").join(script))
            .args([
                self.dir.join("private"),
                self.loop_dir.clone(),
                self.home.join("notes"),
            ])
            .env("HOME", &self.home)
            .env("PATH", path)
            .env("CALLS", self.dir.join("calls"))
            .env("AUDIT", audit.to_string())
            .env("GATE", gate.to_string())
            .env_remove("XDG_CACHE_HOME")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .code()
    }

    fn settings(&self) -> serde_json::Value {
        let path = self.loop_dir.join(".claude/settings.local.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Exit code of the hook the loop checkout's settings install, run as
    /// Claude Code runs it.
    fn hook(&self, tool: &str, input: serde_json::Value) -> Option<i32> {
        let settings = self.settings();
        let hook = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let mut child = Command::new("sh")
            .args(["-c", hook])
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let call = json!({"tool_name": tool, "tool_input": input});
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(call.to_string().as_bytes()).unwrap();
        drop(stdin);
        child.wait().unwrap().code()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default()
    }
}

#[test]
fn setup_keeps_the_agent_out_of_denied_places_and_off_the_web() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run("setup", 0, 0), Some(0));
    let (repo, home) = (&fixture.repo, &fixture.home);
    let settings = fixture.settings();
    let strings = |value: &serde_json::Value| -> Vec<String> {
        let items = value.as_array().unwrap().iter();
        items
            .map(|item| item.as_str().unwrap().to_owned())
            .collect()
    };
    let deny = strings(&settings["permissions"]["deny"]);
    let home_dir = home.display();
    for rule in [
        format!("Read(/{}/**)", repo.display()),
        format!("Edit(/{}/**)", repo.display()),
        format!("Read(/{}/**)", transcripts(home, repo)),
        format!("Read(/{}/**)", fixture.dir.join("private").display()),
        format!("Read(/{home_dir}/.cache/c-to-rust-autoresearch/**)"),
        format!("Read(/{home_dir}/notes/**)"),
        format!("Edit(/{home_dir}/.claude/**)"),
        "WebFetch".to_owned(),
        "WebSearch".to_owned(),
        "mcp__t3-code".to_owned(),
    ] {
        assert!(deny.contains(&rule), "{rule} not in {deny:?}");
    }
    // Only the repository's instructions and skills.
    let excludes = strings(&settings["claudeMdExcludes"]);
    for pattern in [
        format!("{home_dir}/.claude/**"),
        format!("{}/CLAUDE.md", fixture.dir.display()),
        "/CLAUDE.md".to_owned(),
    ] {
        assert!(excludes.contains(&pattern), "{pattern} not in {excludes:?}");
    }
    assert_eq!(settings["skillOverrides"], json!({"mine": "off"}));
    let skill = fixture.loop_dir.join(".claude/skills/inspect/SKILL.md");
    assert!(skill.is_file());
    assert_eq!(git(&fixture.loop_dir, &["status", "--porcelain"]), "");

    let hook = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let own = transcripts(home, &fixture.loop_dir);
    assert!(
        hook.starts_with(&format!("'{}/loop/guard' '{own}' ", repo.display())),
        "{hook}"
    );
    let bash = |command: &str| fixture.hook("Bash", json!({"command": command}));
    assert_eq!(bash("cat ~/notes/a.md"), Some(2));
    assert_eq!(bash("cargo test"), Some(0));
    assert_eq!(fixture.hook("Skill", json!({"skill": "inspect"})), Some(0));
    assert_eq!(fixture.hook("Skill", json!({"skill": "mine"})), Some(2));
    assert_eq!(
        fixture.hook("Skill", json!({"skill": "../skills/inspect"})),
        Some(2)
    );
    let delegate = fixture.hook("mcp__t3-code__delegate_task", json!({"task": "x"}));
    assert_eq!(delegate, Some(2));
    let marked = git(&fixture.loop_dir, &["config", "c2r.loop"]);
    assert_eq!(marked, "true");
}

#[test]
fn a_round_keeps_or_drops_the_commit_and_starts_afresh() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run("setup", 0, 0), Some(0));
    let kept = commit(&fixture.loop_dir, "kept");
    assert_eq!(fixture.run("round", 0, 0), Some(0));
    assert_eq!(git(&fixture.repo, &["rev-parse", "main"]), kept);
    let rounds = std::fs::read_to_string(fixture.loop_dir.join("ROUNDS.md")).unwrap();
    assert!(rounds.contains("feat(transpiler): kept") && rounds.contains("keep"));
    assert_eq!(git(&fixture.loop_dir, &["rev-parse", "HEAD"]), kept);
    assert_eq!(git(&fixture.loop_dir, &["status", "--porcelain"]), "");

    commit(&fixture.loop_dir, "dropped");
    assert_eq!(fixture.run("round", 0, 1), Some(0));
    assert_eq!(git(&fixture.repo, &["rev-parse", "main"]), kept);
    assert_eq!(git(&fixture.loop_dir, &["rev-parse", "HEAD"]), kept);
    let rounds = std::fs::read_to_string(fixture.loop_dir.join("ROUNDS.md")).unwrap();
    assert!(rounds.contains("feat(transpiler): dropped") && rounds.contains("reject"));
    // Both candidates stay in the main checkout for the record.
    let candidates = git(&fixture.repo, &["for-each-ref", "refs/loop/"]);
    assert_eq!(candidates.lines().count(), 2, "{candidates}");
    // The second round's usage counts from the start of its session.
    assert!(fixture.calls().contains("--bin usage -- --since "));
}

#[test]
fn a_round_judges_nothing_without_a_commit_or_after_contact() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run("setup", 0, 0), Some(0));
    assert_eq!(fixture.run("round", 0, 0), Some(1));
    let work = commit(&fixture.loop_dir, "work");
    assert_eq!(fixture.run("round", 1, 0), Some(1));
    assert!(
        !fixture.calls().contains("--bin gate"),
        "{}",
        fixture.calls()
    );
    // The checkout is left as it was, for the owner to look at.
    assert_eq!(git(&fixture.loop_dir, &["rev-parse", "HEAD"]), work);
    let main = git(&fixture.repo, &["rev-parse", "main"]);
    assert_ne!(main, work);
    // Only a checkout that loop/setup made is replaced.
    std::fs::write(fixture.loop_dir.join(".claude/settings.local.json"), "{}").unwrap();
    assert_eq!(fixture.run("round", 0, 0), Some(2));
    assert!(fixture.loop_dir.join(".git").is_dir());
}
