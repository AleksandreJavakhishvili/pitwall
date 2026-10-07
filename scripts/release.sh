#!/usr/bin/env bash
# Prepares a release commit; tagging and pushing stay manual.
#
#   scripts/release.sh 0.1.0
#
# 1. Sets the version in package.json, src-tauri/tauri.conf.json and every
#    workspace crate's Cargo.toml (and Cargo.lock), if it differs.
# 2. Prepends the new version's section to CHANGELOG.md with git-cliff
#    (`--prepend`, so older sections and hand-written text are kept), and moves
#    the hand-written text under "## Unreleased" (the Highlights) into it.
# 3. Commits "chore(release): v<version>" and prints the tag and push commands.
#
# Edit the Highlights under "## Unreleased" in CHANGELOG.md before running it;
# that's the only hand-written part of the changelog. Needs git, node and npx
# (git-cliff is fetched by npx if it isn't installed).
set -euo pipefail

version=${1:-}
version=${version#v}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: scripts/release.sh <version>    e.g. scripts/release.sh 0.1.0" >&2
  exit 2
fi
tag=v$version

cd "$(git rev-parse --show-toplevel)"
if [ -n "$(git status --porcelain)" ]; then
  echo "release: the working tree has changes; commit or stash them first" >&2
  exit 1
fi
if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  echo "release: tag $tag already exists" >&2
  exit 1
fi
if grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "release: CHANGELOG.md already has a $version section" >&2
  exit 1
fi

if command -v git-cliff >/dev/null; then cliff=(git-cliff); else cliff=(npx -y git-cliff@2); fi

# --- 1. version -------------------------------------------------------------
current=$(node -p "require('./src-tauri/tauri.conf.json').version")
if [ "$current" != "$version" ]; then
  echo "release: version $current → $version"
  node -e '
    const fs = require("fs"), v = process.argv[1];
    for (const f of ["package.json", "src-tauri/tauri.conf.json"]) {
      const s = fs.readFileSync(f, "utf8");
      fs.writeFileSync(f, s.replace(/("version":\s*")[^"]*(")/, `$1${v}$2`));
    }' "$version"
  # The first `version = ` line of each member's [package] table.
  for toml in src-tauri/Cargo.toml crates/*/Cargo.toml; do
    awk -v v="$version" '
      /^\[/ { pkg = ($0 == "[package]") }
      pkg && !done && /^version = / { print "version = \"" v "\""; done = 1; next }
      { print }' "$toml" > "$toml.tmp" && mv "$toml.tmp" "$toml"
  done
  cargo metadata --format-version 1 --offline >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null
fi

# --- 2. changelog -----------------------------------------------------------
# Take the "## Unreleased" section out (heading and text up to the next "## ").
highlights=$(mktemp)
rest=$(mktemp)
trap 'rm -f "$highlights" "$rest"' EXIT
awk -v hl="$highlights" '
  /^## / { inside = ($0 ~ /^## Unreleased[[:space:]]*$/); if (inside) next }
  inside { print > hl; next }
  { print }' CHANGELOG.md > "$rest"
cp "$rest" CHANGELOG.md

"${cliff[@]}" --config cliff.toml --tag "$tag" --unreleased --prepend CHANGELOG.md

# Put the hand-written text right under the new version's heading.
if grep -q '[^[:space:]]' "$highlights"; then
  awk -v hl="$highlights" -v ver="$version" '
    !done && index($0, "## [" ver "]") == 1 {
      print; print ""
      while ((getline line < hl) > 0) buf = buf line "\n"
      sub(/^\n+/, "", buf); sub(/\n+$/, "\n", buf)
      printf "%s", buf
      done = 1; next
    }
    { print }' CHANGELOG.md > "$rest"
  cp "$rest" CHANGELOG.md
fi

# --- 3. commit --------------------------------------------------------------
git add -A package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml crates/*/Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "chore(release): $tag"
echo
git --no-pager show --stat --format='%h %s' HEAD
echo
echo "Check CHANGELOG.md (git show HEAD), then tag and push:"
echo "  git tag -a $tag -m $tag"
echo "  git push origin HEAD $tag"
