#!/usr/bin/env bash
# Committed agent-session gate: enforce the formatting half of the CLAUDE.md
# verification gate on every Rust-file edit. Formatting is fast (sub-second) and
# deterministic, so it is cheap to check on each edit; the heavier clippy + test
# gate stays in CI (.github/workflows/rust.yml) and the pre-commit hook
# (.githooks/pre-commit), which are the right place for slow checks.
#
# Wired up by .claude/settings.json as a PostToolUse hook on Edit|Write.
set -euo pipefail

input="$(cat)"

# Only act on Rust-file edits. If a file_path is present and is not *.rs, skip;
# if we cannot find one, fall through and run the check anyway (cheap).
if printf '%s' "$input" | grep -q '"file_path"'; then
    if ! printf '%s' "$input" | grep -qE '"file_path"[[:space:]]*:[[:space:]]*"[^"]*\.rs"'; then
        exit 0
    fi
fi

command -v cargo >/dev/null 2>&1 || exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

if ! out="$(cargo fmt --all -- --check 2>&1)"; then
    {
        echo "rustfmt gate failed — run 'cargo fmt --all' before continuing:"
        echo "$out"
    } >&2
    exit 2
fi
