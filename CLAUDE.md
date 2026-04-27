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

## Project Status Guidance

PRD #1 (AFS v1) is complete: every user story, implementation decision, and
testing decision in `docs/prd/agentic-file-system-v1.md` is implemented in the
source and exercised by `tests/cli.rs`. See the README "PRD #1 Status" section
for the source-grounded coverage map.

Future work belongs to a successor PRD, not to PRD #1. Out-of-scope items
(GUI, cross-platform watcher, global content index, selective undo,
transactional cross-dir writes, ACLs, sandboxing, receipt browser,
cancel/interrupt, mandatory OCR/vision, daemon auto-start, vendored Pi)
should stay out of scope until a new PRD is opened.

Keep README.md aligned with user-visible CLI behavior whenever it changes.
