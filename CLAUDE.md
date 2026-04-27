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

PRD #1 is the broad AFS v1 contract. Many child issues are now closed (see the
README "PRD #1 Status" section for the live breakdown), but closing every
child issue is not the same as completing PRD #1. Do not close #1 or claim
the PRD is complete just because the linked child issues are closed.

Before judging PRD #1 completion, compare the live issue body and README status
against the actual source and tests. Known gaps as of the current code include:

- Concurrent per-agent FIFO task queue (#19).
- Detailed agent lifecycle status beyond coarse CLI markers (#21).
- Final PRD coverage audit before closing #1 (#22).

When documenting or extending AFS, distinguish between "available today" and
"planned by PRD #1". Keep README.md aligned with user-visible CLI behavior.
