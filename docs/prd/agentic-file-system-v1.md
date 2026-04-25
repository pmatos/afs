## Problem Statement

The user wants a file system experience where selected directories are supervised by long-running agents that can monitor files, answer questions, collaborate with other directory-scoped agents, and safely make changes that can be undone. Today there is no implementation of that system: no supervisor daemon, no installed directory agents, no routing model, no durable local history, no monitoring loop, and no user-facing CLI for installing agents or asking questions across the managed file system.

## Solution

Build the first version of an Agentic File System (AFS) for Linux as a Rust application. A foreground Rust supervisor daemon coordinates long-running directory agents while keeping the AFS control plane, ownership rules, history, file watching, and CLI in Rust. Each installed directory gets an `.afs/` Agent Home that stores agent identity, instructions, a dedicated local history backend, ignore policy, and other durable state. Directory agents use Pi as an external runtime over subprocess RPC rather than as vendored in-process code. They manage non-overlapping managed subtrees, communicate by direct message or broadcast, maintain a local directory index, observe filesystem changes via inotify-style events, and report file modifications through explicit change reports. Users interact through a CLI to start the daemon, install/remove agents, ask questions, inspect agent status, inspect history, and undo the latest change.

## User Stories

1. As a user, I want to start a supervisor daemon from the CLI, so that the agent system has one clear control plane.
2. As a user, I want `afs ask` to fail clearly when the daemon is not running, so that daemon lifecycle stays explicit.
3. As a user, I want to install an agent into a selected directory, so that only chosen parts of my filesystem become managed.
4. As a user, I want agent installation to be idempotent, so that re-running install does not create duplicate ownership or state.
5. As a user, I want nested managed directories to become ownership boundaries, so that a file is never managed by two agents.
6. As a user, I want symlink-following to respect existing ownership, so that symlinked files are still managed by at most one agent.
7. As a user, I want each managed directory to have its own durable Agent Home, so that local agent state travels with the directory.
8. As a user, I want each directory agent to have stable identity and local instructions, so that agents can be routed reliably and behave according to the directory's purpose.
9. As a user, I want a dedicated AFS history backend separate from project git, so that agent undo does not pollute my source control history.
10. As a user, I want the initial managed state to be captured at install time, so that changes are reversible from a known baseline.
11. As a user, I want all managed file types, including binaries like PDFs, tracked in history, so that undo works across real personal and project data.
12. As a user, I want directory agents to notice changes through filesystem events, so that indexing and history stay fresh while the agent is running.
13. As a user, I want editor temp-file bursts normalized into meaningful external changes, so that history reflects real outcomes instead of noisy save mechanics.
14. As a user, I want restart reconciliation after downtime, so that missed edits are folded back into history before the agent resumes normal work.
15. As a user, I want agents to answer questions even while indexing is still warming, so that the system is useful immediately after install.
16. As a user, I want answers during indexing or reconciliation to include caveats when coverage is incomplete, so that the system is honest about confidence.
17. As a user, I want the supervisor to route explicit path-based questions directly to the owning agent, so that targeted requests are fast.
18. As a user, I want broad discovery questions to use broadcast, so that the supervisor can find which managed areas are relevant.
19. As a user, I want agents to reply to broadcasts only when they have possible or strong relevance, so that conversations stay efficient.
20. As a user, I want relevant agents to collaborate after broadcast discovery, so that one domain agent can consult another before answering.
21. As a user, I want direct agent-to-agent delegation, so that a directory expert can ask another directory expert for supporting context.
22. As a user, I want reply routing to support "send the reply back to me" or "send it to the supervisor," so that delegated work can compose cleanly.
23. As a user, I want every agent reply to say whether it modified files, so that I always know when an answer also changed the filesystem.
24. As a user, I want the supervisor's final answer to include changed files and participating agents, so that multi-agent work stays transparent.
25. As a user, I want `afs agents` to show live health, indexing state, reconciliation state, and queue depth, so that I can operate the system without guessing.
26. As a user, I want `afs history <path>` to show concise, decision-ready history entries, so that I can decide what to inspect or undo.
27. As a user, I want undo to work on the latest history entry only in v1, so that reversal is reliable before selective undo exists.
28. As a user, I want extra confirmation before undoing my own external edits, so that the CLI does not casually reverse human work.
29. As a user, I want directory agents to serialize active tasks, so that local history and file mutations remain understandable.
30. As a user, I want queued work to be visible rather than hidden, so that long-running or blocked agents can be understood operationally.
31. As a user, I want `afs ask` to stream progress updates, so that multi-agent conversations do not feel stalled.
32. As a user, I want unmanaged paths to be reported as unmanaged rather than silently inspected, so that the boundary between managed and unmanaged data stays real.
33. As a user, I want install and removal to remain supervisor-owned actions, so that lifecycle, registry state, and ownership splits stay consistent.
34. As a user, I want removing a nested agent to merge its history into the parent and archive its Agent Home, so that ownership can collapse without losing useful auditability.
35. As a user, I want moving a managed directory to preserve agent identity when its Agent Home moves too, so that renames do not silently create a different agent.
36. As a user, I want ignore rules to be configurable per managed directory and seeded from `.gitignore` when useful, so that repos and personal folders both behave sensibly.
37. As a user, I want ignored files to stay out of normal indexing and answers, so that noisy areas do not dominate search and summaries.
38. As a user, I want explicit path-based work on ignored files to still be possible and still recorded in history, so that ignore means "not by default," not "impossible."
39. As a user, I want the system to be Linux-only in v1, so that filesystem event behavior is reliable instead of half-portable.
40. As a user, I want a future simple GUI to remain optional and layered on top of the CLI/daemon model, so that the core system stays composable.
41. As a user, I want `afs remove` on a top-level managed directory to archive its Agent Home under the Supervisor Home with a discoverable path, and I want a `--discard-history` flag that explicitly opts out of archiving for both top-level and nested removals, so that removal is auditable by default but can be forced to discard when history is not needed.

## Implementation Decisions

- Build the AFS core as a Rust application, including the supervisor daemon, CLI, history engine, ownership resolution, file watching, and orchestration logic.
- Build a supervisor control-plane module around a single foreground daemon, a Unix-domain supervisor socket, a user-level supervisor home, and single-instance ownership per user home.
- Build a CLI application with the initial commands `daemon`, `install`, `remove`, `ask`, `agents`, `history`, and `undo`.
- Integrate Pi as an external runtime over subprocess RPC instead of embedding its SDK directly into the AFS core.
- Build a Rust directory-agent host/runtime adapter that manages Pi child processes, streams RPC events, and hides Pi-specific details behind an AFS-owned interface.
- Build an agent registry module that maps managed directories to stable agent identities, runtime endpoints, and ownership metadata.
- Build a managed-subtree ownership resolver as a deep module responsible for nested managed-directory exclusion, symlink ownership boundaries, path-to-agent resolution, and rename-aware rediscovery.
- Build an agent-home module responsible for durable local state, including identity, instructions, ignore policy, history backend, and other local metadata.
- Build a history module as a deep module that manages history baselines, agent changes, external changes, merge/archive behavior on removal, latest-entry undo, undoability checks, and change conflict handling.
- Implement the history backend as a dedicated AFS-local backend inside each agent home, initially backed by git semantics but isolated from any user or project repository.
- Build a filesystem monitoring module around Linux inotify-style events with event normalization into settled changes and explicit startup reconciliation after downtime.
- Build a directory-index module as a deep module that owns local searchable metadata, content digests, lightweight summaries, partial-index caveats, and cheap type-specific extraction such as PDF text extraction.
- Keep content knowledge local to directory agents; the supervisor stores coordination metadata only and does not maintain a global content index.
- Build an agent-conversation/orchestration module as a deep module that handles direct routing, broadcast routing, reply timeouts, relevance replies, progress updates, final answer synthesis, and conversation participant tracking.
- Model broadcast as a fan-out to all registered directory agents with silence for `none` relevance in normal mode and optional diagnostic behavior later.
- Allow direct agent-to-agent delegation by default, including mutation-capable tasks, but require every reply to carry a change report describing whether files were modified and which history entries/files were affected.
- Serialize task requests per directory agent with a FIFO queue and no cancel/interrupt support in v1.
- Allow directory agents to decide whether to persist useful information while handling a task request; do not require explicit user permission for every write because history/undo is the safety mechanism.
- Run agent commands with the managed directory as the default working directory and treat outside-subtree access as out of scope unless it goes through an included, owned symlink target.
- Use an AFS ignore policy per managed directory, stored in agent home and optionally seeded from `.gitignore`, while keeping ignored entries inside the managed subtree for explicit path-based operations and history.
- Keep conversation receipts as internal supervisor state in v1 rather than designing a user-facing receipt browser immediately.
- Preserve Linux-only scope for v1 because reliable, event-driven monitoring is a core behavior rather than a pluggable optional feature.
- Do not vendor Pi into the AFS repository for v1; pin it as an upstream dependency and keep the runtime boundary replaceable.
- Design the following deep modules for isolated testing and low interface churn: managed-subtree ownership resolver, history/undo engine, filesystem-event normalizer plus reconciliation logic, directory index, and agent-conversation orchestrator.

## Testing Decisions

- Good tests should exercise external behavior and observable contracts, not internal implementation details. Tests should verify ownership decisions, routing outcomes, history behavior, undo behavior, CLI-visible results, and agent state transitions from the perspective of callers and users.
- Most tests should target Rust-owned modules directly and use fakes or harnesses around the Pi runtime boundary wherever possible.
- The managed-subtree ownership resolver should be tested heavily because it encodes the "a file is never managed by two agents" invariant, including nested managed directories, symlink ownership, and moved directories.
- The history module should be tested heavily because it is the primary safety mechanism. Tests should cover history baseline creation, agent changes, external changes, latest-only undo, confirmation rules for external changes, ownership merge behavior, archived agent homes, and reconciliation batches.
- The filesystem monitoring/event-normalization module should be tested with editor-style atomic-save sequences, rename bursts, temporary files, and offline/restart reconciliation scenarios.
- The directory-index module should be tested for warm indexing, partial-index caveats, cheap extraction behavior, ignore-policy interaction, and on-demand content reads.
- The agent-conversation orchestration module should be tested for direct routing, broadcast timeouts, relevance replies, participant tracking, progress updates, late replies, and final answer summaries with file references and change reports.
- The task-queue behavior for a single directory agent should be tested at the behavioral level: one active task at a time, FIFO ordering, and visibility through agent status.
- The CLI surface should be tested end to end for the first user loop: foreground daemon startup, install, ask, agents, history, undo, and daemon-not-running failure behavior.
- There is effectively no prior art in the current codebase because the repository is still at documentation/bootstrap stage. The test suite should therefore establish a new precedent of behavior-first tests around deep modules and a smaller set of integration tests around CLI and daemon flows.

## Out of Scope

- A full GUI beyond a later minimal prompt window.
- Cross-platform watcher support for macOS or Windows.
- A global content index in the supervisor.
- Selective undo of non-latest history entries.
- Transactional all-or-nothing cross-directory writes.
- A formal permissions or privacy ACL system between agents.
- OS-level sandboxing for directory-agent shell commands.
- A user-facing conversation-receipt browser.
- Cancel/interrupt semantics for queued directory-agent work.
- Rich media understanding such as mandatory OCR, vision, or audio transcription in the core indexing path.
- Background auto-start of the supervisor daemon from `afs ask`.
- Vendoring or forking Pi into the AFS repository for v1.

## Further Notes

- The domain model and terminology for this PRD are already captured in the repository context documentation and should remain the source of truth for naming as implementation begins.
- The dedicated AFS history backend decision is important enough that it already has an ADR and should be treated as a foundational architectural constraint.
- The runtime seam is intentional: Rust owns the system, Pi provides directory-local agent reasoning over RPC, and that boundary should remain replaceable in case AFS later evaluates Hermes or another runtime.
- The recommended implementation order is vertical: supervisor daemon and socket first, install/history baseline second, directory-agent runtime and monitoring third, ask/broadcast/progress fourth, and history/undo CLI polish after that.
