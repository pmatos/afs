## Problem Statement

PRD #1 shipped a working Agentic File System on top of a hand-rolled
tab-delimited stdio protocol that AFS itself defined and that real Pi
never implemented. The fake Pi used by `tests/cli.rs` satisfied the
protocol because the protocol was specified by what the fake reads.
Against the canonical `@mariozechner/pi-coding-agent` runtime advertised
in the README, every `afs ask`, broadcast, collaboration, delegation,
and queued task is silently inert: Pi's `--mode rpc` parser receives
legacy AFS verb records instead of JSONL JSON-RPC, feeds them to
`JSON.parse`, and returns
`Failed to parse command: Unexpected token …`. The supervisor, the
registry, the index, the watcher, history, undo, and reconciliation
all continue to work; only the agent-runtime channel is broken. Issue
#40 documents the symptom and root cause.

## Solution

Rewrite the AFS↔Pi runtime adapter to speak Pi's documented JSONL
JSON-RPC, the protocol described in
`@mariozechner/pi-coding-agent/docs/rpc.md`. The supervisor seam
established by PRD #1 stays exactly where it was: Rust owns the
system, Pi provides directory-local agent reasoning over a
subprocess RPC channel, and the boundary remains replaceable. What
changes is the wire format on that channel and the strategy for
extracting structured output from Pi.

Pi's RPC `prompt` command exposes no `response_format` /
`json_schema` parameter. Pi DOES support schema-enforced output
through extension-defined tools: `defineTool` with TypeBox
`parameters` and `terminate: true` makes the agent end on a tool
call whose arguments are validated against a JSON schema before
they ever leave Pi. AFS will vendor a small `afs_reply.ts`
extension into every Agent Home and pass `-e <path>` to
`pi --mode rpc` so every directory agent has the same structured
output contract. The LLM is instructed to call `afs_reply` as the
final action of every turn; AFS reads the structured arguments
from the `tool_execution_end` event and never has to parse
free-form text.

The AFS-level conversation primitives (ASK, BROADCAST, COLLABORATE,
TASK, DELEGATE, DELEGATED_REPLY) are not part of Pi's vocabulary.
Pi has exactly one prompting verb: `prompt`. The supervisor keeps
all routing, fan-out, collaboration, and delegation logic; per
agent, AFS only sends prompts and consumes events. Each AFS verb
becomes a templated prompt that names what AFS needs the agent to
do, plus a fixed addendum reminding the agent to end with
`afs_reply`.

Tests get one wire-format-conforming fake (a Rust `[[bin]]` that
speaks Pi's JSONL schema) plus an opt-in real-Pi smoke test gated
by an env var. The opt-in smoke is the canary: any future Pi
release that drifts from the schema AFS targets will surface in
that test, not in user-visible silent failure.

## User Stories

1. As a user, I want `afs ask` against the canonical Pi runtime to
   actually receive replies, so that the system is functional in
   production and not just against an in-tree fake.
2. As a user, I want every directory agent to ship with the same
   structured-output contract, so that relevance, file references,
   and change reports are reliable across providers and across LLM
   prompt drift.
3. As a user, I want broadcast file references to keep honoring my
   per-directory ignore policy, so that ignored areas stay quiet
   even after the wire format changes.
4. As a user, I want one agent to be able to delegate to multiple
   peers in one turn, so that complex cross-directory questions
   keep working as they did under PRD #1.
5. As a user, I want the streamed `afs ask` progress lines defined
   by issue #18 to be unchanged, so that the user-visible UX of a
   broadcast/collaboration round does not regress.
6. As a user, I want supervisor shutdown to give Pi a chance to
   finish in-flight LLM calls cleanly, so that session files are
   not corrupted by hard kills.
7. As a user, I want extensions Pi has loaded outside AFS's control
   (e.g., user-installed) to never deadlock a directory agent on
   an interactive UI prompt, so that the system is robust against
   third-party extension installs.
8. As a user, I want the test suite to fail loudly when Pi changes
   wire format, so that the silent failure of PRD #1 cannot
   recur. An opt-in real-Pi smoke test is sufficient — I do not
   want every contributor to install the Pi runtime to run
   `cargo test`.
9. As a user, I want AFS docs to describe the actual protocol
   AFS speaks to Pi, so that anyone replacing the runtime (Hermes
   or another agent runtime) has an accurate contract to
   implement.

## Implementation Decisions

- Speak Pi's documented JSONL JSON-RPC over child stdio. Strict
  LF framing — never `BufReader::lines()` (which splits on
  Unicode-class separators) or any `\r`/CR-aware splitter. One
  JSON object per line.
- Vendor the structured-output contract as an AFS-owned Pi
  extension, not as a prompt-engineered envelope. The schema
  lives in TypeBox in `assets/pi-extensions/afs_reply.ts` and
  the matching Rust deserializer lives in `src/agent_rpc.rs`.
- Pin the schema with `schema_version: Type.Literal(1)`. AFS
  asserts the version before deserializing args. A stale on-disk
  extension surfaces as a typed adapter error on the very first
  turn instead of silent miscommunication.
- Map every AFS conversational verb to one Pi `prompt` call
  carrying a structured envelope tag (`<<<AFS:VERB=…>>>` on the
  first line of the message). The tag is a routing aid for the
  test fake; production Pi treats it as natural-language context
  inside the prompt.
- Build an `agent_rpc` module as a small deep module with a
  single user-facing entry point (`Turn::run`). Inside the
  module live framing, id correlation, event dispatch, the
  extension-UI auto-cancel handler, and the `AfsReply`
  deserializer. Keep the surface area on `lib.rs` minimal so
  future Pi schema changes touch one file.
- Rely on the per-agent FIFO queue established in PRD #1
  (`AgentTaskQueue`) for prompt serialization. AFS never sets
  `streamingBehavior` because AFS never overlaps prompts on one
  agent. The invariant is documented on `Turn::run` and on
  `RegisteredAgent::runtime`.
- Auto-cancel every dialog `extension_ui_request` (`select`,
  `confirm`, `input`, `editor`) with `cancelled: true`. Ignore
  fire-and-forget UI methods. Drain a buffered UI request seen
  at the top of the next `Turn::run`.
- On supervisor shutdown, send `{"type":"abort"}` then drain
  stdout for `AFS_AGENT_SHUTDOWN_DRAIN_MS` (default 500ms)
  before kill. Read the matching `command:"abort"` response
  plus any final events.
- Replace the embedded shell-script `fake_pi_runtime` with a
  Rust `[[bin]]` test helper. The fake reads JSONL on stdin,
  emits JSONL on stdout, and consults JSON-shaped fixture files
  in `$AFS_AGENT_HOME` for per-test reply behavior.
- Keep the supervisor↔CLI Unix-socket text protocol unchanged.
  The CLI seam is unrelated to the Pi seam and works.

## Testing Decisions

- Replace `fake_pi_runtime` with a JSONL-conforming fake. The
  fake routes by the explicit envelope tag on the prompt
  (`<<<AFS:VERB=…>>>`), not by natural-language sniffing.
  Fixture filenames stay stable; only the encoding inside
  changes from the legacy tab-delimited format to JSON.
- Add unit tests in `agent_rpc::tests` for: LF-only framing,
  CR-strip, extension-UI auto-cancel for each dialog method,
  multi-`afs_reply` first-wins semantics, missing-`afs_reply`
  error path, schema-version mismatch error, and end-to-end
  turn execution with multiple `delegates[]`.
- Add a `filter_broadcast_references` unit test that proves the
  per-directory ignore policy still applies to the structured
  reply.
- Add a single `#[ignore]`-marked real-Pi smoke test
  (`real_pi_smoke_ask_returns_afs_reply`) that spawns
  `pi --mode rpc` if available and `AFS_REAL_PI_SMOKE=1`, runs
  one `afs ask`, and asserts the reply parses. This is the wire
  canary; it does not run in default `cargo test`.
- The verification gate (`cargo fmt --all -- --check`,
  `cargo clippy -D warnings`, `cargo test --all-targets
  --all-features`) must be green at every commit. The Step 4
  wire-format flip and the Step 6 fake replacement land in one
  PR because they are mutually dependent.
- Update spawn-arg test assertions in `tests/cli.rs:4141`,
  `:4149`, `:4201`, `:4264` to widen for the new
  `arg=-e <agent_home>/extensions/afs_reply.ts` flag without
  brittle ordering checks.

## Out of Scope

- Embedding Pi as a library (`AgentSession` from
  `@mariozechner/pi-coding-agent`). The PRD #1 seam stays
  subprocess RPC.
- Replacing the supervisor↔CLI Unix-socket text protocol.
- Adding new AFS conversational verbs.
- Per-LLM-token streaming through `afs ask`. Issue #18's
  progress contract is preserved (supervisor-emitted
  milestones); per-token streaming was never implemented and
  stays out of scope.
- Cross-platform Pi runtimes. PRD #1's Linux-only scope holds.
- A background drain thread for asynchronous `extension_ui_request`s
  that fire outside a turn boundary. The in-turn drain in
  `Turn::run` is sufficient for v2; revisit if observed.
- Resuming a Pi session across an agent process restart. PRD #1
  does not promise session restoration and v2 does not add it.
- A user-facing schema-version migration path for the
  `afs_reply` tool. The plan-of-record for v2 is
  `schema_version: 1`; future bumps are a separate PRD concern.

## Further Notes

- The wire-format decision is recorded in
  `docs/adr/0002-pi-jsonl-rpc-adapter.md`. Future runtime
  evaluations (Hermes, etc.) should reference both ADR-0001
  (history backend) and ADR-0002 (wire format) as the
  cross-cutting constraints.
- The `assets/pi-extensions/afs_reply.ts` file is part of the
  AFS source tree and is built into the Rust binary via
  `include_str!`. Updates to the schema must touch the Rust
  deserializer and the TypeBox definition together; the
  `schema_version` literal is the safety latch.
- The structured envelope tag (`<<<AFS:VERB=…>>>`) is a routing
  hint for the test fake. It is included in the production
  prompt because it costs ~20 tokens and gives the LLM useful
  context about what kind of turn it is in. Centralized in
  `agent_rpc::envelope::*` so the vocabulary lives in one place.
- The recommended PR order is: (1) PRD doc + sub-issues,
  (2) Pi extension + spawn flag + spawn-arg test widening,
  (3) `agent_rpc` module landed but unused, (4+6) wire-format
  flip paired with fake replacement, (5) graceful shutdown,
  (7) real-Pi smoke, (9) docs + ADR. Step 8 ("update spawn-arg
  test assertions") folds into step (2) for landing.
