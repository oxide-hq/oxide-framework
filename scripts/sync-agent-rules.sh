#!/usr/bin/env bash
# Mirrors the Claude Code skill into Cursor's rules format.
# The body is shared; only the frontmatter differs.
#   scripts/sync-agent-rules.sh          regenerate .cursor/rules/oxide-eloquent-rust.mdc
#   scripts/sync-agent-rules.sh --check  fail if it is out of date
set -euo pipefail
cd "$(dirname "$0")/.."

skill=.claude/skills/oxide-eloquent-rust/SKILL.md
rule=.cursor/rules/oxide-eloquent-rust.mdc

description=$(sed -n 's/^description: //p' "$skill" | head -n1)
generated=$(
    printf -- '---\ndescription: %s\nglobs: ["**/*.rs", "**/Cargo.toml"]\nalwaysApply: false\n---\n' "$description"
    printf '<!-- Generated from %s by scripts/sync-agent-rules.sh; edit the skill, not this file. -->\n' "$skill"
    # Drop the skill's own frontmatter (first two `---` lines and everything between).
    awk 'n < 2 { if ($0 == "---") n++; next } { print }' "$skill"
)

if [[ ${1:-} == --check ]]; then
    if ! diff -u "$rule" <(printf '%s\n' "$generated"); then
        echo "$rule is out of date; run scripts/sync-agent-rules.sh" >&2
        exit 1
    fi
    exit 0
fi

mkdir -p "$(dirname "$rule")"
printf '%s\n' "$generated" > "$rule"
