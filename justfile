# Oxide task runner — run `just` to list recipes.

default:
    @just --list

# Type-check the whole workspace
check:
    cargo check --workspace --all-targets --all-features

# Run the full test suite (the PostgreSQL suite skips unless OXIDE_TEST_POSTGRES_URL is set)
test:
    cargo test --workspace --all-features

# Run the tests against a throwaway PostgreSQL container (podman or docker)
test-pg engine="podman":
    #!/usr/bin/env bash
    set -euo pipefail
    {{engine}} run --rm -d --name oxide-test-pg -p 55432:5432 -e POSTGRES_PASSWORD=oxide postgres:17 >/dev/null
    trap '{{engine}} stop oxide-test-pg >/dev/null' EXIT
    until {{engine}} exec oxide-test-pg pg_isready -q; do sleep 1; done
    OXIDE_TEST_POSTGRES_URL=postgres://postgres:oxide@localhost:55432/postgres cargo test --workspace --all-features

# Clippy with warnings treated as errors
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format all crates
fmt:
    cargo fmt --all

# Verify formatting without writing
fmt-check:
    cargo fmt --all -- --check

# Expand macros for a crate/item (requires cargo-expand), e.g. `just expand oxide-http`
expand crate="oxide-http" *args="":
    cargo expand -p {{crate}} {{args}}

# License & security audit (requires cargo-deny)
deny:
    cargo deny check

# Regenerate the Cursor rule from the Claude Code skill
sync-rules:
    scripts/sync-agent-rules.sh

# Everything CI runs
ci: fmt-check lint test
    scripts/sync-agent-rules.sh --check
