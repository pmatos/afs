# AFS

AFS is an experimental Linux-only Agentic File System control plane written in
Rust. It runs a foreground supervisor daemon, installs directory-scoped agents
into selected directories, records local AFS history, routes questions to the
right agent, and supports latest-entry undo.

Status: usable core, not the full PRD #1 yet. See "PRD #1 Status" below before
treating this as a complete v1.

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
  reconciliation state, and queue depth. The index column shows
  `index=warming(scanned=N)` while the local text index warms and
  `index=ready(files=N)` once it is current.
- Each managed directory has a Rust-owned local index of its text content.
  The index warms on install, updates on filesystem events, and is rebuilt
  after a saturated burst of changes. Binary files, ignored files, symlinks,
  and nested-managed subtrees are excluded from indexing.
- `afs ask` only emits the `caveat: local index is warming` line while the
  owning agent's local index is still warming.
- `afs ask <prompt>` supports explicit path routing to the deepest owning
  managed directory.
- Broad asks are broadcast to registered agents and include relevant replies,
  file references, participating agents, changed files, and the broadcast
  timeout.
- Directory agents can delegate direct tasks to another agent and request the
  reply either to the supervisor or back to the delegator.
- Delegated file changes are recorded as agent history entries and reported in
  the final answer.
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

By default AFS starts a `pi` executable as the external agent runtime. To use a
specific runtime command:

```sh
export AFS_PI_RUNTIME=/path/to/pi
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
<managed-dir> agent=<id> runtime=pi-rpc-stdio health=<running|stopped> index=<index-state> reconciliation=idle queue=<n>
```

The index field is one of `warming`, `warming(scanned=N)`,
`warming(scanned=N/total=M)`, or `ready(files=N)`. The reconciliation field
is currently a coarse status marker, not a full progress model.

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
- participating agents
- changed files
- history entries for delegated file changes

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

AFS starts the external runtime as:

```text
$AFS_PI_RUNTIME --mode rpc --provider <claude|openai> [--model <model>]
```

The runtime is started with the managed directory as its working directory and
receives these environment variables:

- `AFS_AGENT_ID`
- `AFS_AGENT_HOME`
- `AFS_MANAGED_DIR`
- `AFS_AGENT_RPC=stdio`
- `<api-key-env>` when the configured auth method is `api_key`

The supervisor sends line-oriented requests on stdin.

Direct ask:

```text
ASK
<requested-path>
<prompt>
```

Broadcast ask:

```text
BROADCAST
<prompt>
```

Broadcast replies use one line:

```text
possible|strong<TAB><reason><TAB><answer><TAB><semicolon-separated-file-references>
```

Delegated task:

```text
TASK
<requester-agent-id>
<delegator|supervisor>
<prompt>
```

Task reply:

```text
TASK_REPLY<TAB><answer><TAB><changed-files-or-none><TAB><history-entries-or-none>
```

A direct ask may request delegation by returning:

```text
DELEGATE<TAB><target-agent-id-or-absolute-path><TAB><delegator|supervisor><TAB><prompt>
```

## PRD #1 Status

PRD #1 is not complete. The current implementation covers a substantial CLI
and history core, but several PRD-level requirements are still missing or only
partially represented.

A large group of child issues is closed and the current code is intended to
satisfy their stated acceptance criteria. Those issues were tracer bullets for
major AFS capabilities, but they do not enumerate every requirement in PRD #1.
Do not treat "all listed child issues are closed" as equivalent to "PRD #1 is
complete."

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
- #20: top-level managed-directory remove lifecycle with optional history
  archive or discard.

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
- Direct agent-to-agent delegation with supervisor/delegator reply targets.
- Change reports in final ask output for delegated mutations.
- FIFO handling for multiple delegated tasks sent to one target during one ask.
- Per-agent local text index that warms on install, updates from filesystem
  events, distinguishes warming and ready coverage in `afs agents`, and gates
  the `afs ask` warming caveat on real index state.

Missing or partial:

- PDF/text extraction and binary coverage in indexing: #16.
- Broadcast collaboration after discovery: #17.
- True streamed progress from `afs ask`: #18.
- Concurrent per-agent FIFO task queue: #19.
- Detailed agent lifecycle status for indexing, reconciliation, and queued work:
  #21.
- Final PRD coverage audit before closing #1: #22.

Because of these gaps, issue #1 should remain open until the missing PRD
requirements are either implemented or deliberately scoped out of v1.

## Development

Before committing changes, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Behavior-focused tests live in `tests/cli.rs` and exercise the public CLI plus
supervisor socket path.
