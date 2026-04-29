# AFS

AFS is an experimental Linux-only Agentic File System control plane written in
Rust. It runs a foreground supervisor daemon, installs directory-scoped agents
into selected directories, records local AFS history, routes questions to the
right agent, and supports latest-entry undo.

Status: PRD #1 (AFS v1) is complete. See "PRD #1 Status" below for the
source-grounded coverage map.

## What Works Today

- Foreground supervisor daemon with one Unix socket per AFS home.
- Explicit daemon lifecycle: client commands fail with `daemon is not running`
  when the supervisor is absent.
- One-time `afs login --provider <claude|openai>` authentication that records
  the chosen provider and optional model in `$AFS_HOME/config.json`.
- `afs install <path>` creates a managed directory with `.afs/identity`,
  `.afs/instructions.md`, `.afs/history/` (git-backed), and a seeded
  `.afs/ignore`.
- Idempotent install preserves existing identity, baseline state, and ignore
  policy.
- Nested managed directories split ownership from their parent.
- `afs remove <path>` removes a nested managed directory, archives its Agent
  Home under the parent, and returns ownership to the parent.
- `afs remove <path>` also handles the top-level managed directory: by default
  the Agent Home is archived under the supervisor home; `--discard-history`
  deletes it instead.
- Transitive (nested) removal preserves the original origin and rewrites
  ownership summaries in the parent history.
- Symlink-aware ownership: explicit asks via a symlink resolve to the owning
  managed directory, references that escape the managed directory through
  symlinks are filtered out, and self-referential symlink loops are tolerated.
- Move-aware rediscovery: if the supervisor is stopped and a managed directory
  is moved with its `.afs/` intact, startup rediscovers it and rewrites the
  registry without losing other unresolved rows.
- Per-directory AFS ignore policy stored in `.afs/ignore`, seeded from any
  `.gitignore` at install time. Ignored paths are filtered from broadcast
  references but still recorded in AFS history.
- Directory agents are launched through an external Pi runtime command over
  stdio RPC, configured via `afs login`.
- `afs agents` reports registered agents, runtime health, index state,
  reconciliation state, active work, and queue depth. The index column shows
  `index=warming(scanned=N)` while the local text index warms,
  `index=ready(files=N)` once it is current, and
  `index=incomplete(files=N, failed=M)` if some files could not be extracted.
- Each managed directory has a Rust-owned local index of its content.
  The index warms on install, updates on filesystem events, and is rebuilt
  after a saturated burst of changes. PDF files contribute their extracted
  text to the index. Other binary files, ignored files, symlinks, and
  nested-managed subtrees are excluded from indexing, but binary files
  remain tracked in AFS history and are restored byte-for-byte through
  `afs undo` without UTF-8 assumptions.
- `afs ask` emits `caveat: local index is warming` while the owning
  agent's local index is still warming, and
  `caveat: local index could not extract N file(s)` once the index is
  ready but some files (for example, malformed PDFs) failed extraction.
- `afs ask <prompt>` supports explicit path routing to the deepest owning
  managed directory.
- Broad asks are broadcast to registered agents and include relevant replies,
  file references, participating agents, changed files, and the broadcast
  timeout.
- After broadcast discovery, relevant agents enter a sequential collaboration
  round. Each may delegate to another relevant agent (delegator reply target)
  and use the reply in its refined answer. The final `afs ask` output
  aggregates all participating agents (broadcast + consulted), file
  references, changed files, and history entries from both phases.
- Directory agents can delegate direct tasks to another agent and request the
  reply either to the supervisor or back to the delegator.
- Delegated file changes are recorded as agent history entries and reported in
  the final answer.
- `afs ask` streams `progress: …` lines while it works (broadcast wait,
  per-agent broadcast replies, delegation routing, queueing, task start, and
  per-task file-change milestones) before the existing final-answer block.
- Filesystem monitoring records external changes while the daemon is running.
- Startup reconciliation records changes missed while the daemon was stopped.
- Editor-style atomic save bursts are collapsed into one meaningful external
  change.
- AFS history is stored in an isolated git repository under
  `.afs/history/repo/` so it never touches the surrounding project's git tree.
- Files ignored by the surrounding project's `.gitignore` are still tracked in
  AFS history and restored on undo.
- `afs history <path>` shows newest-first history entries, including child
  history merged into the parent after a nested removal.
- `afs undo <path> <entry> [--yes]` undoes the latest undoable entry only.
- Undoing external or reconciliation changes requires `--yes` in scripted use or
  interactive confirmation on a TTY.

## Quick Start

### Prerequisite: the Pi runtime

AFS is a control plane around an external "Pi" agent runtime that talks to
the underlying provider (Claude or OpenAI). Pi is **not vendored in this
repository** (intentionally — see PRD #1). Every command that talks to a
provider, including `afs login`, shells out to it.

The canonical Pi build is
[`@mariozechner/pi-coding-agent`](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent)
from the `badlogic/pi-mono` monorepo. Its CLI accepts the
`--mode rpc --provider <claude|openai> [--model <model>]` invocation AFS
uses to start each directory agent.

Install it globally with npm (Node.js required):

```sh
npm install -g @mariozechner/pi-coding-agent
```

This places a `pi` executable in your npm global `bin/` directory (run
`npm config get prefix` to find it; the binary is at `<prefix>/bin/pi`).
Confirm with `which pi`.

If `pi` is on `$PATH`, AFS finds it automatically — no extra configuration
needed. If it is not on `$PATH`, or you want to pin a specific build, point
AFS at it explicitly:

```sh
export AFS_PI_RUNTIME=/absolute/path/to/pi
```

If neither condition is met, `afs login` and `afs daemon` will fail with:

```text
AFS agent runtime not found: pi (set AFS_PI_RUNTIME)
```

Pi has its own provider authentication. You can either set
`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` in the environment before
`afs login`, or let `afs login` drive Pi's interactive OAuth flow.

See [Agent Runtime Protocol](#agent-runtime-protocol) for the stdio RPC
contract Pi must implement if you want to plug in a different runtime.

### Build, log in, and run

Build AFS:

```sh
cargo build
```

Authenticate once with the chosen provider (interactive):

```sh
cargo run -- login --provider claude
# or: cargo run -- login --provider openai
```

Login forwards to the configured Pi runtime to perform the OAuth flow and
writes `$AFS_HOME/config.json` on success.

Start the supervisor in one terminal:

```sh
cargo run -- daemon
```

Install a selected directory from another terminal:

```sh
cargo run -- install /path/to/project
```

Ask about an explicit managed path:

```sh
cargo run -- ask "summarize /path/to/project/README.md"
```

Ask a broad discovery question:

```sh
cargo run -- ask "where are my recent health records?"
```

Inspect status and history:

```sh
cargo run -- agents
cargo run -- history /path/to/project
```

Undo the newest undoable history entry:

```sh
cargo run -- undo /path/to/project history-123 --yes
```

Use a separate supervisor home for experiments:

```sh
export AFS_HOME=/tmp/afs-demo-home
cargo run -- daemon
```

## CLI Reference

### `afs login --provider <claude|openai>`

Authenticates with the chosen provider through the external Pi runtime. The
runtime drives the interactive flow; on success AFS writes
`$AFS_HOME/config.json` recording the provider, optional model, and auth
method. Login must be run on a TTY.

### `afs daemon`

Runs the supervisor in the foreground. The daemon creates the Supervisor Home
and a Unix socket named `supervisor.sock`.

Environment:

- `AFS_HOME`: overrides the Supervisor Home. Defaults to `$HOME/.afs`.
- `AFS_PI_RUNTIME`: external Pi runtime command. Defaults to `pi`.
- `AFS_BROADCAST_REPLY_TIMEOUT_MS`: broadcast ask timeout in milliseconds.

### `afs install <path>`

Installs an agent into a selected directory. Install is supervisor-owned and
requires the daemon to be running and a completed `afs login`.

Created under the managed directory:

- `.afs/identity`
- `.afs/instructions.md`
- `.afs/ignore` (seeded from any sibling `.gitignore`)
- `.afs/history/repo/` (isolated git repository for AFS history)

If `<path>` is inside another managed directory, install records an ownership
split in the parent and the child becomes the owner of its subtree.

### `afs remove <path> [--discard-history]`

Removes a managed directory. Both top-level and nested removal are supported.

Nested removal:

- Stops the child runtime.
- By default moves the child `.afs/` Agent Home under the parent
  `.afs/archives/` and merges the child history into the parent view.
- With `--discard-history`, deletes the child Agent Home instead and skips the
  history merge.
- Records an ownership transition in the parent history.
- Future edits under the former child path are recorded by the parent.

Top-level removal:

- Stops the runtime.
- By default archives the Agent Home under the supervisor home so its history
  remains inspectable.
- With `--discard-history`, deletes the Agent Home outright.
- Refuses to remove a directory that contains the supervisor home.

### `afs agents`

Shows one line per registered agent:

```text
<managed-dir> agent=<id> runtime=pi-rpc-stdio health=<running|stopped> index=<index-state> reconciliation=<reconciliation-state> active=<true|false> queue=<n>
```

The index field is one of `warming`, `warming(scanned=N)`,
`warming(scanned=N/total=M)`, `ready(files=N)`, or
`incomplete(files=N, failed=M)` when M file(s) (for example, malformed PDFs)
could not be extracted. The reconciliation field is one of `idle`, `running`,
`complete(changed_files=N)`, or `error`. The active field reports whether the
agent is currently handling a turn, and the queue field reports waiting turns.

### `afs ask <prompt>`

If the prompt contains an explicit existing path, AFS routes the request to the
deepest owning agent. If the explicit path is unmanaged, AFS refuses the request
and suggests an install path.

If the prompt does not contain an explicit path, AFS broadcasts to all
registered agents and waits for `possible` or `strong` replies until the
configured timeout. Replies whose file references fall outside the managed
directory (including via symlinks) or match the directory's ignore policy are
filtered out.

Direct and delegated answers include:

- file references
- an index-warming caveat (only while the owning agent's local index is
  still warming)
- an extraction-failure caveat (once the scan is no longer warming and at
  least one file failed to extract; the caveat reports how many)
- a startup-reconciliation caveat while the owning agent is replaying missed
  changes after daemon downtime
- participating agents
- changed files
- history entries for delegated file changes

While the request is in flight, `afs ask` streams `progress: …` lines so
multi-agent conversations do not feel stalled. The supervisor emits these
lines (each terminated by `\n`) before the final-answer block:

- `progress: route=direct agent=<id>` after explicit-path resolution.
- `progress: route=broadcast agents=<n> timeout_ms=<m>` and
  `progress: broadcast waiting agents=<n>` while a broadcast is in flight.
- `progress: broadcast reply agent=<id> relevance=<possible|strong|none>` per
  parseable reply (timed-out agents emit no line).
- `progress: route=delegated from=<id>` when the owning agent decides to
  delegate after a direct ask.
- `progress: queued task agent=<id> queue=<n>` when a direct or delegated task
  waits behind another turn for the same target, and
  `progress: started task agent=<id> queue=<n>` when the queue drains.
- `progress: delegating from=<id> to=<id> reply=<delegator|supervisor>` before
  each delegated task runs.
- `progress: task complete agent=<id> changed_files=<n>` after each delegated
  task returns.

The streamed body is otherwise unchanged; consumers that buffer the entire
`afs ask` response see the same final answer plus the additional progress
lines at the start.

### `afs history <path>`

Shows history for the owning managed directory. Output is newest first:

```text
entry=<id> timestamp=<unix-seconds> type=<kind> summary=<summary> files=<n> undoable=<yes|no>
```

History entry types currently include:

- `external`: filesystem changes observed by the watcher
- `reconciliation`: changes found after daemon downtime
- `agent`: changes made during delegated tasks
- `undo`: an undo operation
- `ownership`: nested install/remove ownership transitions

After a nested removal, the merged child entries are surfaced under the parent
history with their original origin preserved.

### `afs undo <path> <history-entry> [--yes]`

Restores the managed content snapshot from before the newest undoable history
entry. Non-latest entries are rejected. External and reconciliation entries need
`--yes` in non-interactive use.

## Agent Runtime Protocol

AFS speaks Pi's documented JSONL JSON-RPC over child stdio (see
`@mariozechner/pi-coding-agent/docs/rpc.md`). The supervisor starts the runtime as:

```text
$AFS_PI_RUNTIME --mode rpc --provider <claude|openai> -e <agent-home>/extensions/afs_reply.ts [--model <model>]
```

The runtime is started with the managed directory as its working directory and
receives these environment variables:

- `AFS_AGENT_ID`
- `AFS_AGENT_HOME`
- `AFS_MANAGED_DIR`
- `AFS_AGENT_RPC=stdio`
- `<api-key-env>` when the configured auth method is `api_key`

### Wire format

One JSON object per line, LF (`\n`) as the only record delimiter. Clients
must NOT split on `\r`, U+2028, or U+2029 — those characters can appear
inside JSON strings. The supervisor sends commands on the agent's stdin
and consumes responses + events on stdout.

### Structured output (`afs_reply` extension)

Pi's RPC `prompt` command does not accept a `response_format` /
`json_schema` parameter. AFS instead vendors `assets/pi-extensions/afs_reply.ts`
into every Agent Home and passes `-e <path>` to Pi. The extension registers
an `afs_reply` tool whose TypeBox parameters pin the structured-output
contract every directory agent must satisfy:

```text
{
  "schema_version": 1,
  "relevance":      "none" | "possible" | "strong",
  "reason":         <string>,
  "answer":         <string>,
  "file_references":  [<string>, ...],
  "changed_files":    [<string>, ...],
  "history_entries":  [<string>, ...],
  "delegates":        [{
    "target":       <agent-identity-or-absolute-path>,
    "reply_target": "delegator" | "supervisor",
    "prompt":       <string>
  }, ...]
}
```

The tool uses `terminate: true`, so the agent ends its turn on the
`afs_reply` call. AFS reads the structured args from the `tool_execution_end`
event on Pi's stdout. A stale on-disk extension is rejected on the very
first turn because the Rust deserializer asserts `schema_version == 1`.

### Conversation primitives

Pi has exactly one prompting verb: `prompt`. The supervisor performs all
fan-out (broadcast), collaboration, and delegation; per agent it just
sends prompts and consumes events. AFS prepends a structured envelope tag
to every prompt's first line so the test fake (and a future routing layer)
can identify the AFS-level intent without changing Pi's contract:

- `<<<AFS:VERB=ask>>>` — direct ask against an explicit managed path.
- `<<<AFS:VERB=broadcast>>>` — relevance discovery across all agents.
- `<<<AFS:VERB=collaborate>>>` — a follow-up round when 2+ agents reply
  `possible`/`strong`; carries the peer manifest in the message body.
- `<<<AFS:VERB=task>>>` — a delegated task issued from another agent.
- `<<<AFS:VERB=delegated_reply>>>` — the consultee's reply delivered back
  to the consulter so the agent can refine its answer in a follow-up turn.

The per-turn timeout reuses `AFS_BROADCAST_REPLY_TIMEOUT_MS`.

### Wire-format canary

A single `#[ignore]`-marked test (`real_pi_smoke_ask_returns_afs_reply` in
`tests/cli.rs`) exercises the full wire path against a real `pi --mode rpc`.
Run it with:

```sh
AFS_REAL_PI_SMOKE=1 cargo test -- --ignored real_pi_smoke
```

It is excluded from the default verification gate so contributors are not
required to install Pi to run `cargo test`.

## PRD #1 Status

PRD #1 is complete. Every v1 user story, implementation decision, and testing
decision in `docs/prd/agentic-file-system-v1.md` is implemented in the source
and exercised by the behavior tests in `tests/cli.rs`. The repository
verification gate (`cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`) is green.

Closed child-issue coverage:

- #2: supervisor daemon bootstrap and CLI socket handshake.
- #3: managed directory install, Agent Home creation, external Pi runtime, and
  live agent status.
- #4: live filesystem tracking, local history, and startup reconciliation for a
  managed directory.
- #5: direct ask routing for explicit managed paths.
- #6: safe latest-entry undo.
- #7: broadcast discovery across registered agents.
- #8: direct delegation, reply routing, queue visibility, and delegated change
  reports.
- #9: nested ownership split, nested removal merge record, and child Agent Home
  archive.
- #10: git-backed AFS history backend isolated from the surrounding project.
- #11: child history merged into the parent view after a nested removal.
- #12: symlink-aware ownership boundaries for explicit asks and broadcast
  references.
- #13: move-aware rediscovery of a managed directory after the daemon is
  restarted.
- #14: per-directory AFS ignore policy seeded from `.gitignore`, with explicit
  ask routing still honored on ignored paths.
- #18: streamed `afs ask` progress for broadcast wait, replies, delegation,
  queueing, task start, and per-task file-change milestones.
- #19: concurrent `afs ask` clients with per-agent FIFO task queueing and
  queue depth visible through `afs agents`.
- #20: top-level managed-directory remove lifecycle with optional history
  archive or discard.
- #21: detailed per-agent lifecycle status (reconciliation running/complete,
  active turn, queue depth) surfaced through `afs agents` and ask caveats.
- #22: final PRD coverage audit comparing PRD #1 against the live source and
  tests; closed with the audit summary on issue #1.

Implemented or usable:

- Supervisor daemon, socket ownership, stale-socket recovery, and explicit
  daemon-not-running failure.
- One-time `afs login` flow that records the provider and optional model in
  `$AFS_HOME/config.json` and gates `afs install` until authentication is
  complete.
- Install, idempotent install, per-directory Agent Home, stable identity,
  local instructions, and seeded `.afs/ignore`.
- Git-backed AFS history files stored under Agent Home, isolated from project
  git.
- Baseline/snapshot/content snapshots, live external changes, startup
  reconciliation, newest-first history, and latest-only undo.
- Nested ownership split, deepest-owner path routing, nested removal, merge
  record, child Agent Home archive, transitive nested removal, and top-level
  removal with optional `--discard-history`.
- Broadcast discovery with relevance replies, ignore-policy filtering, and
  timeout.
- Broadcast collaboration: when two or more agents reply with relevance, the
  supervisor runs a sequential collaboration round in which each relevant
  agent may delegate to another relevant agent and use the reply in its
  refined answer. The final `afs ask` output reports every participant and
  aggregates change reports from both phases.
- Direct agent-to-agent delegation with supervisor/delegator reply targets.
- Change reports in final ask output for delegated mutations.
- FIFO handling for multiple delegated tasks sent to one target during one ask.
- Per-agent local content index that warms on install, updates from filesystem
  events, distinguishes warming and ready coverage in `afs agents`, and gates
  the `afs ask` warming caveat on real index state. PDF files contribute
  extracted text to the index; extraction failures are surfaced as
  `incomplete(..., failed=N)` in `afs agents` and as an honest caveat in
  `afs ask`. Binary files are tracked in AFS history and restored
  byte-for-byte through `afs undo`.
- Streamed `afs ask` progress lines (broadcast wait, broadcast replies,
  delegation routing, queueing, task start, per-task file-change milestones)
  emitted before the final-answer block while the request is in flight.
- Concurrent direct `afs ask` clients can target the same directory agent; the
  agent runs one turn at a time in FIFO order while later turns report queued
  and started progress, and `afs agents` exposes active work plus waiting
  queue depth.
- Detailed lifecycle status for startup reconciliation: `afs agents` reports
  `running` while missed changes replay, `complete(changed_files=N)` after
  replay, and direct/delegated asks include a caveat while replay is running.

Out of scope for v1 (deliberate, see PRD): a full GUI, cross-platform watcher
support, a global supervisor content index, selective undo of non-latest
entries, cross-directory transactional writes, a formal permissions/ACL
system, OS-level sandboxing for agent shell commands, a user-facing
conversation-receipt browser, cancel/interrupt for queued tasks, mandatory
OCR/vision/audio in the indexing path, background auto-start of the
supervisor, and vendoring Pi into this repository.

## Development

Before committing changes, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Behavior-focused tests live in `tests/cli.rs` and exercise the public CLI plus
supervisor socket path.
