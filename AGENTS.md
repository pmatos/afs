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
