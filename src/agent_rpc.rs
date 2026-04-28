//! AFS↔Pi JSONL JSON-RPC adapter.
//!
//! Speaks the wire format documented in
//! `@mariozechner/pi-coding-agent/docs/rpc.md`: one JSON object per
//! line, LF as the only record delimiter (per Pi's framing rule, do
//! not split on U+2028 / U+2029). Pairs with the
//! `assets/pi-extensions/afs_reply.ts` extension so every directory
//! agent ends every turn with a structured `afs_reply` tool call.
//!
//! Wired into `src/lib.rs`'s supervisor module by issue #45 (PRD #2
//! step 4). `Turn::run` is the canonical blocking turn driver;
//! `dispatch` plus `JsonlReader`/`JsonlWriter` let callers that need
//! non-blocking polling (broadcast / collaboration) drive the same
//! state machine line-by-line.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Schema version pinned in `assets/pi-extensions/afs_reply.ts`.
/// The Rust deserializer rejects any other value so a stale on-disk
/// extension surfaces as a typed error on the very first turn.
pub(crate) const AFS_REPLY_SCHEMA_VERSION: u32 = 1;

/// Envelope tag prefix used on every prompt to identify the AFS
/// conversational verb. Inserted on the first line of the prompt
/// `message` by the supervisor; stripped from no-one's view (Pi
/// treats it as natural-language context). Centralized here so the
/// vocabulary lives in one place.
pub(crate) mod envelope {
    pub(crate) const ASK: &str = "<<<AFS:VERB=ask>>>";
    pub(crate) const BROADCAST: &str = "<<<AFS:VERB=broadcast>>>";
    pub(crate) const COLLABORATE: &str = "<<<AFS:VERB=collaborate>>>";
    pub(crate) const TASK: &str = "<<<AFS:VERB=task>>>";
    pub(crate) const DELEGATED_REPLY: &str = "<<<AFS:VERB=delegated_reply>>>";
}

// ---------------------------------------------------------------------------
// Outbound commands.
// ---------------------------------------------------------------------------

/// Subset of Pi's RPC commands AFS sends. Borrows so callers don't
/// have to clone strings into the wire format.
#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RpcCommand<'a> {
    Prompt {
        id: &'a str,
        message: &'a str,
    },
    Abort {
        id: &'a str,
    },
    ExtensionUiResponse {
        id: &'a str,
        cancelled: bool,
    },
}

// ---------------------------------------------------------------------------
// Structured reply (deserialized from the `afs_reply` tool call).
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Relevance {
    None,
    Possible,
    Strong,
}

impl Relevance {
    pub(crate) fn as_wire_str(&self) -> &'static str {
        match self {
            Relevance::None => "none",
            Relevance::Possible => "possible",
            Relevance::Strong => "strong",
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyTarget {
    Delegator,
    Supervisor,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateRequest {
    pub(crate) target: String,
    pub(crate) reply_target: ReplyTarget,
    pub(crate) prompt: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct AfsReply {
    pub(crate) schema_version: u32,
    pub(crate) relevance: Relevance,
    pub(crate) reason: String,
    pub(crate) answer: String,
    #[serde(default)]
    pub(crate) file_references: Vec<String>,
    #[serde(default)]
    pub(crate) changed_files: Vec<String>,
    #[serde(default)]
    pub(crate) history_entries: Vec<String>,
    #[serde(default)]
    pub(crate) delegates: Vec<DelegateRequest>,
}

// ---------------------------------------------------------------------------
// Inbound event dispatch.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum RpcEvent {
    /// `{"type":"response","command":"prompt",...}` ack.
    PromptResponse {
        id: Option<String>,
        success: bool,
        error: Option<String>,
    },
    /// `{"type":"agent_end",...}` — turn finished.
    AgentEnd,
    /// `{"type":"tool_execution_end","toolName":<name>,"result":{"details":<value>}}`.
    ToolExecutionEnd {
        tool_name: String,
        details: Option<serde_json::Value>,
    },
    /// `{"type":"extension_ui_request","id":...,"method":...}`.
    ExtensionUiRequest { id: String, method: String },
    /// Every other event Pi may emit (`agent_start`, `turn_*`,
    /// `message_*`, `tool_execution_start/_update`, `queue_update`,
    /// `compaction_*`, `auto_retry_*`, `extension_error`, plus future
    /// types). Carries the raw `Value` so callers that want to peek
    /// can do so; `Turn::run` drops them.
    Ignored(#[allow(dead_code)] serde_json::Value),
}

/// Decode one JSONL line into an `RpcEvent`. Reads
/// `serde_json::Value` first and dispatches by the `"type"` field —
/// avoids the `#[serde(other)]`-with-`Value` shape that does not
/// compile with adjacently-tagged enums.
pub(crate) fn dispatch(line: &str) -> io::Result<RpcEvent> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Pi RPC line is not JSON: {error} (line: {line:?})"),
        )
    })?;
    let ty = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match ty.as_str() {
        "response" => {
            let command = value
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if command == "prompt" {
                let id = value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let success = value
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let error = value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Ok(RpcEvent::PromptResponse { id, success, error })
            } else {
                Ok(RpcEvent::Ignored(value))
            }
        }
        "agent_end" => Ok(RpcEvent::AgentEnd),
        "tool_execution_end" => {
            let tool_name = value
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let details = value.pointer("/result/details").cloned();
            Ok(RpcEvent::ToolExecutionEnd { tool_name, details })
        }
        "extension_ui_request" => {
            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let method = value
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RpcEvent::ExtensionUiRequest { id, method })
        }
        _ => Ok(RpcEvent::Ignored(value)),
    }
}

// ---------------------------------------------------------------------------
// JSONL transport: byte-at-a-time LF reader, line-oriented writer.
// ---------------------------------------------------------------------------

/// Byte-at-a-time LF line reader. Matches Pi's framing rule
/// (`docs/rpc.md`): split records on `\n` only, optionally strip a
/// trailing `\r`. Never `BufReader::lines()`, which splits on
/// Unicode-class separators that are valid inside JSON strings.
pub(crate) struct JsonlReader<R: Read> {
    reader: R,
    buffer: Vec<u8>,
}

impl<R: Read> JsonlReader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
        }
    }

    /// Blocking read of one JSONL line. Returns `None` if the stream
    /// reached EOF before any byte was consumed (clean shutdown);
    /// returns an `Err` of kind `UnexpectedEof` if EOF arrives in
    /// the middle of a line.
    pub(crate) fn read_line(&mut self) -> io::Result<Option<String>> {
        self.buffer.clear();
        let mut byte = [0u8; 1];
        loop {
            match self.reader.read(&mut byte) {
                Ok(0) => {
                    if self.buffer.is_empty() {
                        return Ok(None);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Pi RPC stream ended mid-line",
                    ));
                }
                Ok(_) => {
                    if byte[0] == b'\n' {
                        return Ok(Some(decode_line(&mut self.buffer)));
                    }
                    self.buffer.push(byte[0]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}

fn decode_line(buffer: &mut Vec<u8>) -> String {
    if buffer.last() == Some(&b'\r') {
        buffer.pop();
    }
    String::from_utf8_lossy(buffer).into_owned()
}

/// JSONL writer: serializes one command, appends `\n`, flushes. No
/// other separators.
pub(crate) struct JsonlWriter<W: Write> {
    writer: W,
}

impl<W: Write> JsonlWriter<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self { writer }
    }

    pub(crate) fn send(&mut self, command: &RpcCommand<'_>) -> io::Result<()> {
        let mut line = serde_json::to_vec(command)
            .map_err(|error| io::Error::other(format!("failed to encode RPC command: {error}")))?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()
    }
}

// ---------------------------------------------------------------------------
// Turn driver.
// ---------------------------------------------------------------------------

/// Outcome of a single prompt-to-`agent_end` cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnOutcome {
    pub(crate) reply: AfsReply,
    pub(crate) warnings: Vec<String>,
}

/// Drives one prompt-to-`agent_end` cycle.
///
/// # Invariants
///
/// AFS's per-agent FIFO queue (`AgentTaskQueue`) prevents
/// overlapping prompts on a single agent, so `streamingBehavior` is
/// never set and any "agent is streaming" rejection from Pi is
/// treated as a bug. Pi events do not carry the request `id`, but
/// because turns are serialized per agent, every event between
/// `prompt` accept and `agent_end` belongs to the in-flight `id` by
/// construction.
///
/// Late `extension_ui_request` events that arrive between one
/// turn's `agent_end` and the next turn's `prompt` accept are
/// auto-cancelled by the next call to `run`, since the loop
/// processes UI requests at any point in the stream.
pub(crate) fn run<R: Read, W: Write>(
    reader: &mut JsonlReader<R>,
    writer: &mut JsonlWriter<W>,
    id: &str,
    message: &str,
) -> io::Result<TurnOutcome> {
    writer.send(&RpcCommand::Prompt { id, message })?;
    let mut captured: Option<AfsReply> = None;
    let mut warnings: Vec<String> = Vec::new();

    loop {
        let line = reader.read_line()?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Pi closed stdout before agent_end",
            )
        })?;
        match dispatch(&line)? {
            RpcEvent::PromptResponse {
                success: false,
                error,
                id: response_id,
            } => {
                let reason = error.unwrap_or_else(|| "prompt rejected".to_string());
                let prefix = match response_id {
                    Some(rid) if rid == id => format!("Pi rejected prompt: {reason}"),
                    Some(rid) => format!("Pi rejected prompt {rid}: {reason}"),
                    None => format!("Pi rejected prompt (no id): {reason}"),
                };
                return Err(io::Error::other(prefix));
            }
            RpcEvent::PromptResponse { success: true, .. } => {}
            RpcEvent::ToolExecutionEnd { tool_name, details } if tool_name == "afs_reply" => {
                let details = details.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "afs_reply tool_execution_end is missing result.details",
                    )
                })?;
                let reply: AfsReply = serde_json::from_value(details).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("afs_reply details did not match schema: {error}"),
                    )
                })?;
                if reply.schema_version != AFS_REPLY_SCHEMA_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "afs_reply schema_version {} not supported (expected {})",
                            reply.schema_version, AFS_REPLY_SCHEMA_VERSION,
                        ),
                    ));
                }
                if captured.is_some() {
                    warnings.push(
                        "afs_reply emitted more than once in one turn; using the first reply"
                            .to_string(),
                    );
                } else {
                    captured = Some(reply);
                }
            }
            RpcEvent::ToolExecutionEnd { .. } => {
                // Some other tool finished — Pi will keep streaming
                // until the agent ends.
            }
            RpcEvent::ExtensionUiRequest { id: ui_id, method } => {
                if matches!(method.as_str(), "select" | "confirm" | "input" | "editor") {
                    writer.send(&RpcCommand::ExtensionUiResponse {
                        id: &ui_id,
                        cancelled: true,
                    })?;
                }
                // Fire-and-forget UI methods (notify, setStatus,
                // setWidget, setTitle, set_editor_text) require no
                // response.
            }
            RpcEvent::AgentEnd => {
                let reply = captured.ok_or_else(|| {
                    io::Error::other("agent finished without afs_reply tool call")
                })?;
                return Ok(TurnOutcome { reply, warnings });
            }
            RpcEvent::Ignored(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_events(
        events: &[&str],
        id: &str,
        message: &str,
    ) -> (io::Result<TurnOutcome>, String) {
        let mut input = Vec::new();
        for event in events {
            input.extend_from_slice(event.as_bytes());
            input.push(b'\n');
        }
        let mut output = Vec::new();
        let mut reader = JsonlReader::new(input.as_slice());
        let mut writer = JsonlWriter::new(&mut output);
        let outcome = run(&mut reader, &mut writer, id, message);
        (outcome, String::from_utf8(output).expect("output is utf-8"))
    }

    #[test]
    fn end_to_end_turn_returns_afs_reply() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"agent_start"}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"strong","reason":"ok","answer":"hello","file_references":["a.txt"],"changed_files":[],"history_entries":[],"delegates":[]}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, output) = run_with_events(&events, "t-1", "ping");
        let outcome = outcome.expect("turn succeeds");
        assert_eq!(outcome.reply.relevance, Relevance::Strong);
        assert_eq!(outcome.reply.answer, "hello");
        assert_eq!(outcome.reply.file_references, vec!["a.txt".to_string()]);
        assert!(outcome.warnings.is_empty());
        assert!(
            output.contains(r#""type":"prompt""#) && output.contains(r#""id":"t-1""#),
            "writer should send prompt with id; got: {output}"
        );
    }

    #[test]
    fn missing_afs_reply_is_typed_error() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, _) = run_with_events(&events, "t-1", "ping");
        let error = outcome.expect_err("turn should fail when no afs_reply");
        assert!(
            error
                .to_string()
                .contains("agent finished without afs_reply"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn prompt_rejection_propagates() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":false,"error":"Failed to parse command: Unexpected token"}"#,
        ];
        let (outcome, _) = run_with_events(&events, "t-1", "ping");
        let error = outcome.expect_err("turn should fail when prompt is rejected");
        assert!(
            error.to_string().contains("Pi rejected prompt"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn schema_version_mismatch_is_typed_error() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":2,"relevance":"strong","reason":"ok","answer":"hello"}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, _) = run_with_events(&events, "t-1", "ping");
        let error = outcome.expect_err("turn should reject schema_version != 1");
        assert!(
            error.to_string().contains("schema_version 2"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn double_afs_reply_keeps_first_and_warns() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"strong","reason":"first","answer":"first"}}}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"none","reason":"second","answer":"second"}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, _) = run_with_events(&events, "t-1", "ping");
        let outcome = outcome.expect("turn succeeds");
        assert_eq!(outcome.reply.answer, "first");
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("more than once"));
    }

    #[test]
    fn dialog_extension_ui_requests_are_auto_cancelled() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"extension_ui_request","id":"ui-1","method":"confirm","title":"Allow?"}"#,
            r#"{"type":"extension_ui_request","id":"ui-2","method":"select","title":"Pick","options":["a","b"]}"#,
            r#"{"type":"extension_ui_request","id":"ui-3","method":"input","title":"Type"}"#,
            r#"{"type":"extension_ui_request","id":"ui-4","method":"editor","title":"Edit"}"#,
            r#"{"type":"extension_ui_request","id":"ui-fire","method":"notify","message":"hi"}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"strong","reason":"ok","answer":"done"}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, output) = run_with_events(&events, "t-1", "ping");
        outcome.expect("turn succeeds");
        for id in ["ui-1", "ui-2", "ui-3", "ui-4"] {
            assert!(
                output.contains(&format!(r#""id":"{id}""#))
                    && output.contains(r#""cancelled":true"#),
                "expected auto-cancel for {id}; got: {output}"
            );
        }
        assert!(
            !output.contains(r#""id":"ui-fire""#),
            "fire-and-forget ui request should not get a response; got: {output}"
        );
    }

    #[test]
    fn delegates_array_round_trips() {
        let events = [
            r#"{"type":"response","command":"prompt","id":"t-1","success":true}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"strong","reason":"ok","answer":"see peers","delegates":[{"target":"agent-A","reply_target":"delegator","prompt":"q1"},{"target":"agent-B","reply_target":"supervisor","prompt":"q2"}]}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, _) = run_with_events(&events, "t-1", "ping");
        let outcome = outcome.expect("turn succeeds");
        assert_eq!(outcome.reply.delegates.len(), 2);
        assert_eq!(outcome.reply.delegates[0].target, "agent-A");
        assert_eq!(
            outcome.reply.delegates[0].reply_target,
            ReplyTarget::Delegator
        );
        assert_eq!(
            outcome.reply.delegates[1].reply_target,
            ReplyTarget::Supervisor
        );
    }

    #[test]
    fn jsonl_reader_strips_cr_and_keeps_unicode_separators() {
        // Pi's framing rule: split only on \n. U+2028 / U+2029 must
        // stay in the line because they are valid inside JSON strings.
        let input = "{\"a\":\"x\u{2028}y\"}\r\n{\"b\":\"z\"}\n";
        let mut reader = JsonlReader::new(input.as_bytes());
        let first = reader.read_line().expect("read 1").expect("not eof");
        let second = reader.read_line().expect("read 2").expect("not eof");
        assert_eq!(first, "{\"a\":\"x\u{2028}y\"}");
        assert_eq!(second, "{\"b\":\"z\"}");
        assert!(reader.read_line().expect("read 3").is_none());
    }

    #[test]
    fn ignored_event_types_do_not_break_dispatch() {
        let event = dispatch(r#"{"type":"queue_update","steering":[],"followUp":[]}"#)
            .expect("dispatch ok");
        match event {
            RpcEvent::Ignored(value) => {
                assert_eq!(
                    value.get("type").and_then(|v| v.as_str()),
                    Some("queue_update")
                );
            }
            other => panic!("expected Ignored, got {other:?}"),
        }
    }

    #[test]
    fn late_ui_request_buffered_before_next_turn_is_auto_cancelled() {
        // Pi may emit an extension_ui_request after the previous
        // turn's agent_end (Step 5: graceful shutdown documents this
        // path). The next Turn::run consumes the buffered request
        // before its own prompt response arrives and auto-cancels
        // it, keeping the agent unblocked.
        let events = [
            r#"{"type":"extension_ui_request","id":"late-1","method":"confirm","title":"Resume?"}"#,
            r#"{"type":"response","command":"prompt","id":"t-2","success":true}"#,
            r#"{"type":"tool_execution_end","toolName":"afs_reply","result":{"details":{"schema_version":1,"relevance":"strong","reason":"ok","answer":"resumed"}}}"#,
            r#"{"type":"agent_end","messages":[]}"#,
        ];
        let (outcome, output) = run_with_events(&events, "t-2", "ping");
        outcome.expect("turn succeeds with buffered UI cancel");
        assert!(
            output.contains(r#""id":"late-1""#) && output.contains(r#""cancelled":true"#),
            "buffered UI request should be auto-cancelled; got: {output}"
        );
    }

    #[test]
    fn malformed_line_returns_typed_error() {
        let result = dispatch("not-json");
        assert!(
            result.is_err(),
            "non-JSON line should fail dispatch; got {result:?}"
        );
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
