//! Fake `pi --mode rpc` binary used by `tests/cli.rs`.
//!
//! Speaks Pi's JSONL JSON-RPC protocol (per
//! `@mariozechner/pi-coding-agent/docs/rpc.md`): one JSON object per
//! line, LF-only framing. Replaces the previous shell-script fake at
//! the same call sites; behavior is driven by JSON fixture files
//! placed in `$AFS_AGENT_HOME` by the tests.
//!
//! Per-prompt routing: the fake reads the `<<<AFS:VERB=...>>>` tag
//! that AFS prepends to every prompt message and dispatches to a
//! verb-specific reply builder. This keeps the per-test fixture
//! contract close to what the line+TAB fake offered.

use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const SCHEMA_VERSION: u32 = 1;

fn main() {
    let agent_home = match env::var_os("AFS_AGENT_HOME") {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!("fake_pi: AFS_AGENT_HOME not set");
            std::process::exit(2);
        }
    };
    let _ = fs::create_dir_all(&agent_home);
    write_spawn_observed(&agent_home);
    write_runtime_started(&agent_home);
    if let Ok(banner) = fs::read_to_string(agent_home.join("runtime-stderr")) {
        let _ = io::stderr().write_all(banner.as_bytes());
    }

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut buffer = String::new();
    loop {
        buffer.clear();
        let read = match handle.read_line(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) => {
                eprintln!("fake_pi: read error: {error}");
                break;
            }
        };
        if read == 0 {
            break;
        }
        let trimmed = buffer.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "prompt" => handle_prompt(&agent_home, &value),
            "abort" => emit(&json!({
                "type": "response",
                "command": "abort",
                "id": value.get("id"),
                "success": true,
            })),
            "extension_ui_response" => {}
            _ => {}
        }
    }
}

fn write_spawn_observed(agent_home: &Path) {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut body = String::new();
    body.push_str(&format!("argv={}\n", args.join(" ")));
    for arg in &args {
        body.push_str(&format!("arg={arg}\n"));
    }
    body.push_str(&format!(
        "env_HOME={}\n",
        env::var("HOME").unwrap_or_default()
    ));
    body.push_str(&format!(
        "env_ANTHROPIC_API_KEY={}\n",
        env::var("ANTHROPIC_API_KEY").unwrap_or_default()
    ));
    body.push_str(&format!(
        "env_OPENAI_API_KEY={}\n",
        env::var("OPENAI_API_KEY").unwrap_or_default()
    ));
    body.push_str("done=1\n");
    let _ = fs::write(agent_home.join("spawn-observed"), body);
}

fn write_runtime_started(agent_home: &Path) {
    let body = format!(
        "identity={}\nmanaged_dir={}\nrpc={}\n",
        env::var("AFS_AGENT_ID").unwrap_or_default(),
        env::var("AFS_MANAGED_DIR").unwrap_or_default(),
        env::var("AFS_AGENT_RPC").unwrap_or_default(),
    );
    let _ = fs::write(agent_home.join("runtime-started"), body);
}

fn emit(value: &Value) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", value);
    let _ = stdout.flush();
}

fn handle_prompt(agent_home: &Path, value: &Value) {
    let id = value.get("id").and_then(Value::as_str).unwrap_or("");
    let message = value.get("message").and_then(Value::as_str).unwrap_or("");
    let verb = parse_verb(message);

    // Log the prompt first so tests can observe the in-flight turn
    // (e.g., the FIFO queue test waits for prompt-received to land
    // before spawning the second ask).
    log_prompt(agent_home, verb, message);
    apply_delay(agent_home, verb);

    emit(&json!({
        "type": "response",
        "command": "prompt",
        "id": id,
        "success": true,
    }));
    emit(&json!({"type": "agent_start"}));

    apply_side_effects(agent_home, verb);
    let reply = build_reply(agent_home, verb, message);

    emit(&json!({
        "type": "tool_execution_end",
        "toolCallId": format!("call-{id}"),
        "toolName": "afs_reply",
        "result": {
            "content": [{"type": "text", "text": "afs_reply recorded"}],
            "details": reply,
        },
        "isError": false,
    }));
    emit(&json!({"type": "agent_end", "messages": []}));
}

fn parse_verb(message: &str) -> &'static str {
    let first_line = message.lines().next().unwrap_or("");
    if first_line.starts_with("<<<AFS:VERB=ask>>>") {
        "ask"
    } else if first_line.starts_with("<<<AFS:VERB=broadcast>>>") {
        "broadcast"
    } else if first_line.starts_with("<<<AFS:VERB=collaborate>>>") {
        "collaborate"
    } else if first_line.starts_with("<<<AFS:VERB=task>>>") {
        "task"
    } else if first_line.starts_with("<<<AFS:VERB=delegated_reply>>>") {
        "delegated_reply"
    } else {
        "unknown"
    }
}

fn apply_delay(agent_home: &Path, verb: &str) {
    let file = match verb {
        "ask" => "ask-delay-seconds",
        "broadcast" => "broadcast-delay-seconds",
        "task" => "task-delay-seconds",
        _ => return,
    };
    if let Ok(body) = fs::read_to_string(agent_home.join(file))
        && let Ok(seconds) = body.trim().parse::<f64>()
    {
        let millis = (seconds * 1000.0).round() as u64;
        thread::sleep(Duration::from_millis(millis));
    }
}

fn apply_side_effects(agent_home: &Path, verb: &str) {
    if verb != "task" {
        return;
    }
    let Some(file_name) = read_trim(agent_home, "task-write-file") else {
        return;
    };
    let content = read_trim(agent_home, "task-write-content")
        .unwrap_or_else(|| "delegated task content".to_string());
    let managed_dir = match env::var_os("AFS_MANAGED_DIR") {
        Some(value) => PathBuf::from(value),
        None => return,
    };
    let _ = fs::write(managed_dir.join(file_name), format!("{content}\n"));
}

fn log_prompt(agent_home: &Path, verb: &str, message: &str) {
    append_line(agent_home, "prompt-received", &format!("verb={verb}"));
    append_line(agent_home, "prompt-received", message);

    // Match the legacy `key=value` log format the v1 fake produced.
    // Tests grep for `requester=<id>`, `path=<path>`, etc.
    match verb {
        "ask" => {
            let path = extract_field(message, "Path: ").unwrap_or_default();
            let prompt = extract_field(message, "Question: ").unwrap_or_default();
            append_line(agent_home, "ask-received", &format!("path={path}"));
            append_line(agent_home, "ask-received", &format!("prompt={prompt}"));
        }
        "broadcast" => {
            let prompt = extract_field(message, "Question: ").unwrap_or_default();
            append_line(
                agent_home,
                "broadcast-received",
                &format!("prompt={prompt}"),
            );
        }
        "collaborate" => {
            let prompt = extract_field(message, "Question: ").unwrap_or_default();
            append_line(
                agent_home,
                "collaborate-received",
                &format!("prompt={prompt}"),
            );
            // Mirror the v1 `collaborate-peers` file: one peer per
            // line in `<identity>\t<managed_dir>` form, parsed from
            // the bullet-list portion of the prompt envelope.
            let mut peer_lines = Vec::new();
            let mut in_peers = false;
            for line in message.lines() {
                if line.starts_with("Peers:") {
                    in_peers = true;
                    continue;
                }
                if in_peers {
                    if let Some(rest) = line.strip_prefix("- ")
                        && let Some((identity, rest)) = rest.split_once(" (")
                    {
                        let managed_dir = rest.trim_end_matches(')').trim();
                        peer_lines.push(format!("{identity}\t{managed_dir}"));
                        continue;
                    }
                    if line.is_empty() {
                        continue;
                    }
                    break;
                }
            }
            if !peer_lines.is_empty() {
                let _ = fs::write(
                    agent_home.join("collaborate-peers"),
                    peer_lines.join("\n") + "\n",
                );
            }
        }
        "task" => {
            let requester = extract_field(message, "Requester: ").unwrap_or_default();
            let reply_target = extract_field(message, "Reply target: ").unwrap_or_default();
            let task_prompt = extract_field(message, "Task: ").unwrap_or_default();
            append_line(
                agent_home,
                "task-received",
                &format!("requester={requester}"),
            );
            append_line(
                agent_home,
                "task-received",
                &format!("reply_target={reply_target}"),
            );
            append_line(
                agent_home,
                "task-received",
                &format!("prompt={task_prompt}"),
            );
        }
        "delegated_reply" => {
            let agent =
                extract_delegated_field(message, "Delegated reply from ").unwrap_or_default();
            let answer = extract_field(message, "Answer: ").unwrap_or_default();
            let changed = extract_field(message, "Changed files: ").unwrap_or_default();
            let history = extract_field(message, "History entries: ").unwrap_or_default();
            append_line(
                agent_home,
                "delegated-reply-received",
                &format!("agent={agent}"),
            );
            append_line(
                agent_home,
                "delegated-reply-received",
                &format!("answer={answer}"),
            );
            append_line(
                agent_home,
                "delegated-reply-received",
                &format!("changed_files={changed}"),
            );
            append_line(
                agent_home,
                "delegated-reply-received",
                &format!("history_entries={history}"),
            );
        }
        _ => {}
    }
}

fn extract_delegated_field(message: &str, prefix: &str) -> Option<String> {
    for line in message.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest.trim_end_matches(':').trim().to_string());
        }
    }
    None
}

fn append_line(agent_home: &Path, file: &str, line: &str) {
    let path = agent_home.join(file);
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(line);
    if !existing.ends_with('\n') {
        existing.push('\n');
    }
    let _ = fs::write(path, existing);
}

fn read_trim(agent_home: &Path, file: &str) -> Option<String> {
    fs::read_to_string(agent_home.join(file))
        .ok()
        .map(|s| s.trim().to_string())
}

fn read_fixture(agent_home: &Path, file: &str) -> Option<Value> {
    let body = fs::read_to_string(agent_home.join(file)).ok()?;
    serde_json::from_str::<Value>(&body).ok()
}

fn agent_identity() -> String {
    env::var("AFS_AGENT_ID")
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn build_reply(agent_home: &Path, verb: &str, message: &str) -> Value {
    match verb {
        "ask" => build_ask_reply(agent_home, message),
        "broadcast" => build_broadcast_reply(agent_home),
        "collaborate" => build_collaborate_reply(agent_home, message),
        "task" => build_task_reply(agent_home, message),
        "delegated_reply" => build_delegated_reply(message),
        _ => default_none_reply(),
    }
}

fn default_none_reply() -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "relevance": "none",
        "reason": "fake default",
        "answer": "",
        "file_references": [],
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    })
}

fn extract_field(message: &str, prefix: &str) -> Option<String> {
    for line in message.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn build_ask_reply(agent_home: &Path, message: &str) -> Value {
    let path = extract_field(message, "Path: ").unwrap_or_default();
    let question = extract_field(message, "Question: ").unwrap_or_default();
    if let Some(target) = read_trim(agent_home, "delegate-target") {
        let reply_target = read_trim(agent_home, "delegate-reply-target")
            .unwrap_or_else(|| "supervisor".to_string());
        let prompt = read_trim(agent_home, "delegate-prompt").unwrap_or_else(|| question.clone());
        let mut delegates = vec![json!({
            "target": target,
            "reply_target": reply_target,
            "prompt": prompt,
        })];
        if let Some(prompt2) = read_trim(agent_home, "delegate-second-prompt") {
            delegates.push(json!({
                "target": target,
                "reply_target": reply_target,
                "prompt": prompt2,
            }));
        }
        return json!({
            "schema_version": SCHEMA_VERSION,
            "relevance": "strong",
            "reason": "fake delegating",
            "answer": "",
            "file_references": [],
            "changed_files": [],
            "history_entries": [],
            "delegates": delegates,
        });
    }
    json!({
        "schema_version": SCHEMA_VERSION,
        "relevance": "strong",
        "reason": "fake answer",
        "answer": format!("agent {} answered about {}", agent_identity(), path),
        "file_references": [],
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    })
}

fn build_broadcast_reply(agent_home: &Path) -> Value {
    if let Some(value) = read_fixture(agent_home, "broadcast-response.json") {
        return ensure_schema(value);
    }
    // Backwards compatibility with the prior line+TAB fixture format
    // used by ~50 tests inherited from the v1 fake. Format:
    //   <relevance>\t<reason>\t<answer>\t<file_refs separated by ';'>\n
    if let Some(body) = read_trim(agent_home, "broadcast-response") {
        let mut fields = body.splitn(4, '\t');
        let relevance = fields.next().unwrap_or("none").to_string();
        let reason = fields.next().unwrap_or("").to_string();
        let answer = fields.next().unwrap_or("").to_string();
        let file_references: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        return json!({
            "schema_version": SCHEMA_VERSION,
            "relevance": relevance,
            "reason": reason,
            "answer": answer,
            "file_references": file_references,
            "changed_files": [],
            "history_entries": [],
            "delegates": [],
        });
    }
    default_none_reply()
}

fn build_collaborate_reply(agent_home: &Path, _message: &str) -> Value {
    let mut delegates = Vec::new();
    if let Some(target) = read_trim(agent_home, "collaborate-delegate-target") {
        let prompt = read_trim(agent_home, "collaborate-delegate-prompt")
            .unwrap_or_else(|| "fake collab prompt".to_string());
        delegates.push(json!({
            "target": target,
            "reply_target": "delegator",
            "prompt": prompt,
        }));
    }

    let mut reply = if let Some(value) =
        read_fixture(agent_home, "collaborate-response-template.json")
            .or_else(|| read_fixture(agent_home, "collaborate-response.json"))
    {
        ensure_schema(value)
    } else if let Some(reply) = compat_collaborate_reply(agent_home) {
        reply
    } else {
        json!({
            "schema_version": SCHEMA_VERSION,
            "relevance": "strong",
            "reason": "fake collab",
            "answer": format!("{} collaborated", agent_identity()),
            "file_references": [],
            "changed_files": [],
            "history_entries": [],
            "delegates": [],
        })
    };

    if !delegates.is_empty() {
        reply["delegates"] = Value::Array(delegates);
    }
    reply
}

/// Parses the old-format `collaborate-response-template` /
/// `collaborate-response` fixtures (line+TAB
/// `COLLABORATE_REPLY\t<answer>\t<changed>\t<history>` content),
/// preserving the `__PEER_ANSWER__` substitution where present.
fn compat_collaborate_reply(agent_home: &Path) -> Option<Value> {
    let body = read_trim(agent_home, "collaborate-response-template")
        .or_else(|| read_trim(agent_home, "collaborate-response"))?;
    let payload = body
        .strip_prefix("COLLABORATE_REPLY\t")
        .unwrap_or(body.as_str());
    let mut fields = payload.splitn(3, '\t');
    let answer_template = fields.next().unwrap_or("").to_string();
    let answer = if answer_template.contains("__PEER_ANSWER__") {
        let peer = read_trim(agent_home, "collaborate-delegated-peer-answer").unwrap_or_default();
        answer_template.replace("__PEER_ANSWER__", &peer)
    } else {
        answer_template
    };
    let changed_files = parse_csv_list(fields.next().unwrap_or(""));
    let history_entries = parse_csv_list(fields.next().unwrap_or(""));
    Some(json!({
        "schema_version": SCHEMA_VERSION,
        "relevance": "strong",
        "reason": "fake collab compat",
        "answer": answer,
        "file_references": [],
        "changed_files": changed_files,
        "history_entries": history_entries,
        "delegates": [],
    }))
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "none" {
        return Vec::new();
    }
    raw.split([';', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn build_task_reply(agent_home: &Path, message: &str) -> Value {
    if let Some(value) = read_fixture(agent_home, "task-response.json") {
        return ensure_schema(value);
    }
    // Backwards compatibility: the previous fake treated
    // `task-response` as a raw answer string and emitted
    // `TASK_REPLY\t<answer>\tnone\tnone`. Preserve that exact answer
    // text so existing fixtures keep working.
    if let Some(answer) = read_trim(agent_home, "task-response") {
        return json!({
            "schema_version": SCHEMA_VERSION,
            "relevance": "strong",
            "reason": "fake task compat",
            "answer": answer,
            "file_references": [],
            "changed_files": [],
            "history_entries": [],
            "delegates": [],
        });
    }
    let task_prompt = extract_field(message, "Task: ").unwrap_or_default();
    json!({
        "schema_version": SCHEMA_VERSION,
        "relevance": "strong",
        "reason": "fake task",
        "answer": format!("delegated answer for {} from {}", task_prompt, agent_identity()),
        "file_references": [],
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    })
}

fn build_delegated_reply(message: &str) -> Value {
    let answer = extract_field(message, "Answer: ").unwrap_or_default();
    let agent_home = match env::var_os("AFS_AGENT_HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::new(),
    };

    // Also record into the v1 `collaborate-delegated-reply` log used
    // by tests that previously asserted on the shell fake's
    // collaborate→delegate flow.
    let delegated_agent =
        extract_delegated_field(message, "Delegated reply from ").unwrap_or_default();
    append_line(
        &agent_home,
        "collaborate-delegated-reply",
        &format!("agent={delegated_agent}"),
    );
    append_line(
        &agent_home,
        "collaborate-delegated-reply",
        &format!("answer={answer}"),
    );

    // Honor the v1 `collaborate-response-template` substitution: if
    // the agent has a template with `__PEER_ANSWER__`, render the
    // refined answer using the consultee's reply text.
    if let Some(template) = read_trim(&agent_home, "collaborate-response-template") {
        let payload = template
            .strip_prefix("COLLABORATE_REPLY\t")
            .unwrap_or(template.as_str());
        let mut fields = payload.splitn(3, '\t');
        let answer_template = fields.next().unwrap_or("").to_string();
        let rendered = answer_template.replace("__PEER_ANSWER__", &answer);
        let changed_files = parse_csv_list(fields.next().unwrap_or(""));
        let history_entries = parse_csv_list(fields.next().unwrap_or(""));
        return json!({
            "schema_version": SCHEMA_VERSION,
            "relevance": "strong",
            "reason": "fake refined (template)",
            "answer": rendered,
            "file_references": [],
            "changed_files": changed_files,
            "history_entries": history_entries,
            "delegates": [],
        });
    }

    json!({
        "schema_version": SCHEMA_VERSION,
        "relevance": "strong",
        "reason": "fake refined",
        "answer": format!("delegator {} used {}", agent_identity(), answer),
        "file_references": [],
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    })
}

fn ensure_schema(mut value: Value) -> Value {
    if value
        .get("schema_version")
        .and_then(Value::as_u64)
        .is_none()
    {
        value["schema_version"] = json!(SCHEMA_VERSION);
    }
    for key in [
        "file_references",
        "changed_files",
        "history_entries",
        "delegates",
    ] {
        if value.get(key).is_none() {
            value[key] = json!([]);
        }
    }
    if value.get("relevance").is_none() {
        value["relevance"] = json!("strong");
    }
    if value.get("reason").is_none() {
        value["reason"] = json!("fake reply");
    }
    if value.get("answer").is_none() {
        value["answer"] = json!("");
    }
    value
}
