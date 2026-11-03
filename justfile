# Oxide task runner — run `just` to list recipes.

default:
    @just --list

# Type-check the whole workspace
check:
    cargo check --workspace --all-targets

# Run the full test suite
test:
    cargo test --workspace

# Clippy with warnings treated as errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings

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
