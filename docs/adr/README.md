# Architecture Decision Records

## Legacy numbering (0001-0003)

`0001-*.md` through `0003-*.md` use sequential numbers assigned by hand.
This scheme is collision-prone under concurrent PRs — two authors can
independently pick the same next integer for different decisions (this
already happened in sibling repos: s11 shipped two different
`0011-*.md` files, and symphonika, where this check originated, has a
dozen-plus duplicate numbers). These three files are frozen: don't
renumber or reuse a number from this range — they're referenced by
number (`ADR-0001`, `ADR-0002`) in `CLAUDE.md` and
`docs/prd/agentic-file-system-v2-pi-rpc-rewrite.md`.

## Current naming

New ADRs use `docs/adr/YYYY-MM-DD-slug.md`, dated the day the ADR is
authored. Reference one in prose or comments as `ADR-YYYY-MM-DD` (add
the slug too if more than one ADR shares a date). Two authors can't
independently pick the same real-world date-and-slug pair the way they
could pick the same next integer, so there's no numbering-collision
class left to check for in CI.
