#!/bin/sh
# Runs .githooks/commit-msg against good and bad sample messages.
#   sh scripts/test-commit-msg.sh
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
hook="$root/.githooks/commit-msg"
tmp=$(mktemp "${TMPDIR:-/tmp}/pitwall-msg.XXXXXX")
trap 'rm -f "$tmp"' EXIT
failures=0

# expect <0|1> <flag or ""> <message>
expect() {
  want=$1 flag=$2 text=$3
  printf '%s\n' "$text" > "$tmp"
  if [ -n "$flag" ]; then sh "$hook" "$flag" "$tmp" 2>/dev/null; else sh "$hook" "$tmp" 2>/dev/null; fi
  got=$?
  [ "$got" -ne 0 ] && got=1
  if [ "$got" -ne "$want" ]; then
    echo "FAIL (want exit $want, got $got): $(printf '%s' "$text" | head -1)"
    failures=$((failures + 1))
  fi
}

nl='
'
# Good
expect 0 "" "feat: add a worktree picker"
expect 0 "" "fix(core): keep the PATH from the login shell"
expect 0 "" "perf(detect): skip unchanged screens"
expect 0 "" "docs(website): describe the agw provider"
expect 0 "" "chore(deps): bump tauri to 2.9"
expect 0 "" "refactor(providers): split the agw transport${nl}${nl}The transport now owns reconnects."
expect 0 "" "feat(cli)!: rename --host to --target${nl}${nl}BREAKING CHANGE: scripts calling --host must use --target."
expect 0 "" "fix!: drop the old socket path${nl}${nl}BREAKING-CHANGE: clients older than 0.2 can't connect."
expect 0 "" "test(hold): cover restart on ~/code/my-app${nl}# Please enter the commit message${nl}# Lines starting with '#' are ignored"
expect 0 "" "${nl}${nl}ci: cache cargo on windows"
expect 0 "" "build(app): pin \`tauri-cli\` in the lockfile"
expect 0 "" "Merge branch 'main' into my-feature"
expect 0 "" "Merge pull request #12 from someone/my-feature"
expect 0 "" "Revert \"feat(ui): add a worktree picker\""
expect 0 "" "revert: feat(ui): add a worktree picker"
expect 0 "" "fixup! fix(core): keep the PATH"
expect 0 "" "squash! feat: add a picker"
expect 0 "--title" "feat(agw)!: talk to my-vm over ssh"
expect 0 "" "feat(ui): $(printf 'a%.0s' $(seq 1 62))"   # 72 characters exactly

# Bad
expect 1 "" ""
expect 1 "" "Add a worktree picker"
expect 1 "" "feature: add a worktree picker"
expect 1 "" "feat(gui): add a worktree picker"
expect 1 "" "feat(): add a worktree picker"
expect 1 "" "feat:add a worktree picker"
expect 1 "" "feat:  add a worktree picker"
expect 1 "" "feat: Add a worktree picker"
expect 1 "" "feat: add a worktree picker."
expect 1 "" "Feat: add a worktree picker"
expect 1 "" "feat(core) : add a worktree picker"
expect 1 "" "feat(ui): $(printf 'a%.0s' $(seq 1 63))"   # 73 characters
expect 1 "" "feat(core): add a picker${nl}no blank line before the body"
expect 1 "" "feat(cli)!: rename --host to --target"
expect 1 "" "feat(cli): rename --host${nl}${nl}BREAKING CHANGE: scripts must change."
expect 1 "" "WIP"
expect 1 "--title" "Update README.md"

if [ "$failures" -eq 0 ]; then echo "commit-msg: all sample messages behave as expected"; fi
[ "$failures" -eq 0 ]
