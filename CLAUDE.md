# Agent Instructions

These instructions apply to the whole repository.

Before committing any change, make sure the Rust verification gate passes:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Do not commit with failing formatting, linting, or tests unless the user explicitly asks for a commit that records a known-broken state.

Prefer behavior-focused tests through public interfaces, especially CLI-visible behavior for AFS user workflows.

When documenting or sharing `cargo run` invocations for the user-facing CLI,
use `cargo run --bin afs -- <command>`. The workspace also builds the
`fake_pi` helper binary, so plain `cargo run -- <command>` is ambiguous.

## Project Status Guidance

PRD #1 (AFS v1) is complete: every user story, implementation decision, and
testing decision in `docs/prd/agentic-file-system-v1.md` is implemented in the
source and exercised by `tests/cli.rs`. See the README "PRD #1 Status" section
for the source-grounded coverage map.

PRD #2 (`docs/prd/agentic-file-system-v2-pi-rpc-rewrite.md`, issue #40)
supersedes PRD #1's runtime-adapter wire format only. The supervisor seam
(subprocess RPC) is unchanged; what changed is the bytes on that channel —
AFS now speaks Pi's documented JSONL JSON-RPC and pins structured output
through a vendored `afs_reply` extension (TypeBox schema, `terminate: true`).
ADR-0002 records the wire-format decision. Sub-issues #43-#48 track the
implementation steps. The opt-in real-Pi smoke test in `tests/cli.rs` is
the canary that prevents another silent-drift failure of the shape that
produced issue #40; run it with
`AFS_REAL_PI_SMOKE=1 cargo test -- --ignored real_pi_smoke`.

Future work belongs to a successor PRD, not to PRD #1 or PRD #2. Out-of-scope
items (GUI, cross-platform watcher, global content index, selective undo,
transactional cross-dir writes, ACLs, sandboxing, receipt browser,
cancel/interrupt, mandatory OCR/vision, daemon auto-start, vendored Pi)
should stay out of scope until a new PRD is opened.

Keep README.md aligned with user-visible CLI behavior whenever it changes.
