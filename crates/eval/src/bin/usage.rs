//! Sums the model usage that Claude Code records in its session transcripts,
//! per session and model, optionally within a time window. T3 Code runs Claude
//! through Claude Code, so this also covers T3 chats.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: usage [--since TIME] [--until TIME] PATH...\n\
     PATH is a transcript (.jsonl) or a directory searched for them, for example\n\
     ~/.claude/projects/-home-USER-src-c-to-rust-autoresearch. TIME is UTC, like\n\
     2026-11-04T20:30:00Z.";
const TOKENS: [&str; 4] = [
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "output_tokens",
];

#[derive(Default)]
struct Session {
    title: String,
    first: String,
    last: String,
    /// Tokens by model, then by kind.
    models: BTreeMap<String, BTreeMap<&'static str, u64>>,
    /// Requests by reasoning effort.
    efforts: BTreeMap<String, u64>,
}

fn main() -> ExitCode {
    let Some((since, until, paths)) = parse_args() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let (mut files, mut visited) = (Vec::new(), BTreeSet::new());
    for path in &paths {
        if let Err(err) = transcripts(path, &mut files, &mut visited) {
            eprintln!("cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    }
    // A transcript reached twice, through a repeated path or a link, counts once.
    let mut seen = BTreeSet::new();
    files.retain(|file| seen.insert(file.canonicalize().unwrap_or_else(|_| file.clone())));
    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
    for file in &files {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("cannot read {}: {err}", file.display());
                return ExitCode::FAILURE;
            }
        };
        add(&text, since.as_deref(), until.as_deref(), &mut sessions);
    }
    sessions.retain(|_, session| !session.first.is_empty());
    println!("{:#}", report(&sessions));
    ExitCode::SUCCESS
}

type Args = (Option<String>, Option<String>, Vec<PathBuf>);

fn parse_args() -> Option<Args> {
    let (mut since, mut until, mut paths) = (None, None, Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--since" => since = Some(bound(&args.next()?)?),
            "--until" => until = Some(bound(&args.next()?)?),
            _ if arg.starts_with('-') => return None,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    (!paths.is_empty()).then_some((since, until, paths))
}

/// A time given on the command line. Transcripts keep milliseconds, so finer
/// fractions cannot be compared and are rejected rather than cut off.
fn bound(time: &str) -> Option<String> {
    let fraction = time.split_once('.').map_or("", |(_, rest)| rest);
    (fraction.trim_end_matches('Z').len() <= 3)
        .then(|| normalise(time))
        .flatten()
}

/// Usage of one message, the largest seen in any of its records.
#[derive(Default)]
struct Message {
    session: String,
    model: String,
    tokens: BTreeMap<&'static str, u64>,
    /// Absent when no record reported a thinking count.
    thinking: Option<u64>,
}

/// Adds one transcript's records. Claude Code writes an assistant message once
/// per content block, so a message counts once by id, with the largest usage
/// any of its records reports.
fn add(
    text: &str,
    since: Option<&str>,
    until: Option<&str>,
    sessions: &mut BTreeMap<String, Session>,
) {
    let mut messages: BTreeMap<String, Message> = BTreeMap::new();
    for record in text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    {
        let Some(id) = record["sessionId"].as_str() else {
            continue;
        };
        let session = sessions.entry(id.to_owned()).or_default();
        if let Some(title) = record["aiTitle"].as_str() {
            session.title = title.to_owned();
        }
        let Some(time) = record["timestamp"].as_str().and_then(normalise) else {
            continue;
        };
        if since.is_some_and(|since| time.as_str() < since)
            || until.is_some_and(|until| time.as_str() >= until)
        {
            continue;
        }
        match record["type"].as_str() {
            Some("api-request") => {
                let effort = record["params"]["output_config"]["effort"]
                    .as_str()
                    .unwrap_or("default");
                *session.efforts.entry(effort.to_owned()).or_default() += 1;
            }
            Some("assistant") => {
                let message = &record["message"];
                let (Some(model), Some(message_id)) =
                    (message["model"].as_str(), message["id"].as_str())
                else {
                    continue;
                };
                // Claude Code's placeholder for messages it made up itself.
                if model == "<synthetic>" {
                    continue;
                }
                let entry = messages
                    .entry(message_id.to_owned())
                    .or_insert_with(|| Message {
                        session: id.to_owned(),
                        model: model.to_owned(),
                        ..Message::default()
                    });
                let usage = &message["usage"];
                for kind in TOKENS {
                    let count = entry.tokens.entry(kind).or_default();
                    *count = (*count).max(usage[kind].as_u64().unwrap_or(0));
                }
                if let Some(thinking) = usage["output_tokens_details"]["thinking_tokens"].as_u64() {
                    entry.thinking = Some(entry.thinking.unwrap_or(0).max(thinking));
                }
            }
            _ => continue,
        }
        if session.first.is_empty() || time < session.first {
            session.first.clone_from(&time);
        }
        if time > session.last {
            session.last = time;
        }
    }
    for message in messages.into_values() {
        let session = sessions.entry(message.session).or_default();
        let tokens = session.models.entry(message.model).or_default();
        *tokens.entry("messages").or_default() += 1;
        for (kind, count) in message.tokens {
            *tokens.entry(kind).or_default() += count;
        }
        if let Some(thinking) = message.thinking {
            *tokens.entry("thinking_tokens").or_default() += thinking;
            *tokens.entry("messages_with_thinking_count").or_default() += 1;
        }
    }
}

fn report(sessions: &BTreeMap<String, Session>) -> serde_json::Value {
    let mut total: BTreeMap<&str, BTreeMap<&str, u64>> = BTreeMap::new();
    for session in sessions.values() {
        for (model, tokens) in &session.models {
            let sum = total.entry(model).or_default();
            for (kind, count) in tokens {
                *sum.entry(kind).or_default() += count;
            }
        }
    }
    serde_json::json!({
        "models": total,
        "sessions": sessions.iter().map(|(id, session)| serde_json::json!({
            "id": id,
            "title": session.title,
            "first": session.first,
            "last": session.last,
            "models": session.models,
            "requests_by_effort": session.efforts,
        })).collect::<Vec<_>>(),
    })
}

/// Turns a UTC timestamp into `YYYY-MM-DDTHH:MM:SS.mmm`, which sorts as text.
fn normalise(time: &str) -> Option<String> {
    let time = time.strip_suffix('Z')?;
    let (seconds, fraction) = time.split_once('.').unwrap_or((time, ""));
    let bytes = seconds.as_bytes();
    let shape = bytes.len() == 19
        && bytes.iter().enumerate().all(|(i, byte)| match i {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        });
    let field = |range: std::ops::Range<usize>| seconds.get(range)?.parse::<u32>().ok();
    let valid = shape
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
        && field(5..7).is_some_and(|month| (1..=12).contains(&month))
        && field(8..10).is_some_and(|day| (1..=31).contains(&day))
        && field(11..13).is_some_and(|hour| hour <= 23)
        && field(14..16).is_some_and(|minute| minute <= 59)
        && field(17..19).is_some_and(|second| second <= 60);
    valid.then(|| format!("{seconds}.{:0<3.3}", fraction))
}

/// Collects transcripts under `path`, entering each directory once so that a
/// link back to an ancestor cannot repeat or loop.
fn transcripts(
    path: &Path,
    files: &mut Vec<PathBuf>,
    visited: &mut BTreeSet<PathBuf>,
) -> std::io::Result<()> {
    if path.is_file() {
        files.push(path.to_owned());
        return Ok(());
    }
    if !visited.insert(path.canonicalize()?) {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            transcripts(&entry, files, visited)?;
        } else if entry.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = r#"{"type":"ai-title","aiTitle":"Loop","sessionId":"s1"}
{"type":"api-request","sessionId":"s1","timestamp":"2026-11-04T10:00:00.000Z","params":{"output_config":{"effort":"high"}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T10:00:01.000Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":10,"cache_read_input_tokens":100,"output_tokens":5,"output_tokens_details":{"thinking_tokens":3}}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T10:00:01.500Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":10,"cache_read_input_tokens":100,"output_tokens":5,"output_tokens_details":{"thinking_tokens":3}}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T11:00:00Z","message":{"id":"m2","model":"claude-opus-5-5","usage":{"input_tokens":1,"output_tokens":7}}}
"#;

    #[test]
    fn repeated_message_blocks_count_once() {
        let mut sessions = BTreeMap::new();
        add(TRANSCRIPT, None, None, &mut sessions);
        let tokens = &sessions["s1"].models["claude-opus-5-5"];
        assert_eq!(tokens["messages"], 2);
        assert_eq!(tokens["output_tokens"], 12);
        assert_eq!(tokens["thinking_tokens"], 3);
        assert_eq!(sessions["s1"].efforts["high"], 1);
        assert_eq!(sessions["s1"].title, "Loop");
    }

    #[test]
    fn window_keeps_records_from_since_until_before_until() {
        let mut sessions = BTreeMap::new();
        let since = normalise("2026-11-04T10:30:00Z");
        add(TRANSCRIPT, since.as_deref(), None, &mut sessions);
        let tokens = &sessions["s1"].models["claude-opus-5-5"];
        assert_eq!((tokens["messages"], tokens["output_tokens"]), (1, 7));
        assert_eq!(sessions["s1"].first, "2026-11-04T11:00:00.000");
    }

    #[test]
    fn largest_usage_of_a_message_counts() {
        let partial = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T10:00:01.000Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"output_tokens":2}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T10:00:02.000Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"output_tokens":9,"output_tokens_details":{"thinking_tokens":4}}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-04T10:00:03.000Z","message":{"id":"m2","model":"claude-opus-5-5","usage":{"output_tokens":1}}}
"#;
        let mut sessions = BTreeMap::new();
        add(partial, None, None, &mut sessions);
        let tokens = &sessions["s1"].models["claude-opus-5-5"];
        assert_eq!(tokens["messages"], 2);
        assert_eq!(tokens["output_tokens"], 10);
        assert_eq!(tokens["thinking_tokens"], 4);
        assert_eq!(tokens["messages_with_thinking_count"], 1);
    }

    #[test]
    fn impossible_times_and_fine_fractions_are_rejected() {
        assert_eq!(normalise("2026-99-99T99:99:99Z"), None);
        assert_eq!(normalise("2026-11-04X10:00:00Z"), None);
        assert_eq!(bound("2026-11-04T10:00:02.1235Z"), None);
        assert!(bound("2026-11-04T10:00:02.123Z").is_some());
    }

    #[test]
    fn linked_directories_and_repeated_paths_are_read_once() {
        let dir = std::env::temp_dir().join(format!("usage-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::write(dir.join("project/s1.jsonl"), TRANSCRIPT).unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("project/loop")).unwrap();
        let (mut files, mut visited) = (Vec::new(), BTreeSet::new());
        transcripts(&dir, &mut files, &mut visited).unwrap();
        transcripts(&dir.join("project"), &mut files, &mut visited).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
    }

    #[test]
    fn timestamps_with_and_without_fractions_compare_in_time_order() {
        let whole = normalise("2026-11-04T10:00:02Z").unwrap();
        let fraction = normalise("2026-11-04T10:00:02.5Z").unwrap();
        assert!(whole < fraction, "{whole} {fraction}");
        assert_eq!(normalise("2026-11-04T10:00:02+01:00"), None);
    }
}
