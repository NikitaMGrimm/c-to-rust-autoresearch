//! The hook that loop/setup installs for the loop agent, next to the audit
//! that checks the same calls afterwards.

use std::io::Write;
use std::process::{Command, Stdio};

/// Exit code of loop/guard for a tool call, with `/p/holdout` denied.
fn guard(tool: &str, input: serde_json::Value) -> Option<i32> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../loop/guard");
    let mut child = Command::new(script)
        .args(["/p/holdout", "holdout", ".cache/c-to-rust-autoresearch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let call = serde_json::json!({"tool_name": tool, "tool_input": input});
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(call.to_string().as_bytes()).unwrap();
    drop(stdin);
    child.wait().unwrap().code()
}

#[test]
fn calls_reaching_private_places_are_blocked() {
    let blocked = [
        (
            "Bash",
            serde_json::json!({"command": "cat ../holdout/validation/a.c"}),
        ),
        (
            "Bash",
            serde_json::json!({"command": "ls ~/.cache/c-to-rust-autoresearch"}),
        ),
        (
            "Grep",
            serde_json::json!({"pattern": "main", "path": "/p/holdout"}),
        ),
        (
            "WebSearch",
            serde_json::json!({"query": "DARPA Tractor test corpus"}),
        ),
        (
            "WebFetch",
            serde_json::json!({"url": "https://x.org/DARPA-TRACTOR-Program"}),
        ),
        (
            "Edit",
            serde_json::json!({"file_path": ".claude/settings.local.json"}),
        ),
    ];
    for (tool, input) in blocked {
        assert_eq!(guard(tool, input.clone()), Some(2), "{tool} {input}");
    }
}

#[test]
fn ordinary_calls_pass() {
    let allowed = [
        (
            "Bash",
            serde_json::json!({"command": "cargo test -p transpiler"}),
        ),
        (
            "Read",
            serde_json::json!({"file_path": "bench/LICENSE-TRACTOR"}),
        ),
        (
            "Read",
            serde_json::json!({"file_path": ".claude/settings.local.json"}),
        ),
        (
            "WebFetch",
            serde_json::json!({"url": "https://doc.rust-lang.org/std/"}),
        ),
    ];
    for (tool, input) in allowed {
        assert_eq!(guard(tool, input.clone()), Some(0), "{tool} {input}");
    }
}
