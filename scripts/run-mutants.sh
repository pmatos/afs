#!/usr/bin/env bash
# Run cargo-mutants locally with afs's recommended flag set.
#
# Examples:
#   scripts/run-mutants.sh                    # full mutation run
#   scripts/run-mutants.sh --shard 0/8        # one shard (interruptible)
#   scripts/run-mutants.sh --list             # list mutants without running
#   scripts/run-mutants.sh -- --test cli      # pass-through test selection
#
# The `cargo mutate` alias defined in .cargo/config.toml supplies the standard
# flags (--baseline=skip --in-place --timeout 180). The default test invocation
# is `--all-targets --all-features`, mirroring the project verification gate.
set -euo pipefail

if ! command -v cargo-mutants >/dev/null 2>&1; then
    echo "cargo-mutants not installed." >&2
    echo "Install with: cargo install cargo-mutants" >&2
    exit 1
fi

# If the user already supplied a `--` separator, honor their test args.
# Otherwise append the project default.
for arg in "$@"; do
    if [ "$arg" = "--" ]; then
        exec cargo mutate "$@"
    fi
done

exec cargo mutate "$@" -- --all-targets --all-features
