#!/usr/bin/env bash
# inspect-handoff-package.sh
#
# Pre-flight audit of an offline handoff package BEFORE anything touches the repo.
# Read-only: it never modifies the worktree, the index, or any branch.
#
# Usage:
#   inspect-handoff-package.sh <package.zip | unpacked-dir> [base-ref]
#
#   [base-ref]  Default: origin/HEAD target branch, else origin/main
#
# Prints:
#   - every patch in series order, with subject, author and touched paths
#   - the set of authors the package would introduce
#   - the baseline SHA the handoff document claims, versus the real one
#   - files the series touches that do not exist at the base (new files)
#
# Exit codes:
#   0  inspection completed
#   1  no patches found / not a git repository

set -euo pipefail

PKG="${1:?usage: inspect-handoff-package.sh <package.zip|unpacked-dir> [base-ref]}"
BASE_REF="${2:-}"

git rev-parse --show-toplevel >/dev/null 2>&1 || { echo "ERROR: not inside a git repository" >&2; exit 1; }
REPO_ROOT="$(git rev-parse --show-toplevel)"

if [ -z "$BASE_REF" ]; then
  head_branch="$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null || true)"
  if [ -n "$head_branch" ]; then
    BASE_REF="$head_branch"
  else
    BASE_REF="origin/main"
  fi
fi

TMP_DIR=""
cleanup() { [ -n "$TMP_DIR" ] && rm -rf "$TMP_DIR"; }
trap cleanup EXIT

if [ -d "$PKG" ]; then
  WORK_DIR="$PKG"
else
  [ -f "$PKG" ] || { echo "ERROR: no such package: $PKG" >&2; exit 1; }
  TMP_DIR="$(mktemp -d)"
  unzip -q "$PKG" -d "$TMP_DIR" || { echo "ERROR: unzip failed" >&2; exit 1; }
  WORK_DIR="$TMP_DIR"
fi

PATCH_DIR="$(find "$WORK_DIR" -type d -name patches | head -1)"
[ -n "$PATCH_DIR" ] || { echo "ERROR: no 'patches' directory inside the package" >&2; exit 1; }

shopt -s nullglob
PATCHES=("$PATCH_DIR"/*.patch)
shopt -u nullglob
PATCH_COUNT=0
for _p in "${PATCHES[@]}"; do PATCH_COUNT=$((PATCH_COUNT + 1)); done
if [ "$PATCH_COUNT" -eq 0 ]; then
  echo "ERROR: no *.patch files in $PATCH_DIR" >&2
  exit 1
fi

echo "== Package =================================================="
echo "repo root : $REPO_ROOT"
echo "worktree  : $(git rev-parse --abbrev-ref HEAD)  ($(git rev-parse --short HEAD))"
echo "base ref  : $BASE_REF"
echo "patches   : $PATCH_COUNT"
echo

if [ -n "$(git status --porcelain -uall)" ]; then
  echo "WARN: working tree is dirty; replay will refuse until it is clean."
  echo
fi

echo "== Claimed baseline in the handoff document ================="
DOC="$(find "$WORK_DIR" -maxdepth 3 -type f -iname 'HANDOFF*.md' | head -1)"
if [ -z "$DOC" ]; then
  echo "no HANDOFF*.md found"
else
  claimed="$(grep -oE '\b[0-9a-f]{40}\b' "$DOC" | head -3 | tr '\n' ' ' || true)"
  if [ -z "$claimed" ]; then
    echo "$(basename "$DOC"): no 40-hex commit SHA mentioned"
  else
    for sha in $claimed; do
      if git cat-file -e "${sha}^{commit}" 2>/dev/null; then
        ancestor="no"
        git merge-base --is-ancestor "$sha" "${BASE_REF}" 2>/dev/null && ancestor="yes"
        echo "claimed $sha : exists locally, ancestor of $BASE_REF = $ancestor"
      else
        echo "claimed $sha : NOT present in this clone"
      fi
    done
  fi
  echo "actual  $BASE_REF : $(git rev-parse "$BASE_REF" 2>/dev/null || echo '<ref not found>')"
fi
echo

echo "== Series ==================================================="
AUTHORS_FILE="$(mktemp)"
TOUCHED_FILE="$(mktemp)"
trap 'cleanup; rm -f "$AUTHORS_FILE" "$TOUCHED_FILE"' EXIT

for patch in "${PATCHES[@]}"; do
  name="$(basename "$patch")"
  # mailinfo decodes RFC 2047 subjects and drops the [PATCH n/m] prefix.
  meta="$(git mailinfo /dev/null /dev/null < "$patch" 2>/dev/null || true)"
  subject="$(printf '%s
' "$meta" | sed -n 's/^Subject: //p' | head -1)"
  a_name="$(printf '%s
' "$meta" | sed -n 's/^Author: //p' | head -1)"
  a_mail="$(printf '%s
' "$meta" | sed -n 's/^Email: //p' | head -1)"
  if [ -n "$a_name" ]; then author="${a_name} <${a_mail}>"; else author='<none>'; fi
  echo "$author" >> "$AUTHORS_FILE"
  echo "--- $name"
  echo "    subject: $subject"
  echo "    author : $author"
  if numstat="$(git apply --numstat "$patch" 2>&1)"; then
    echo "$numstat" | awk '{print $3}' | sed '/^$/d' >> "$TOUCHED_FILE"
    echo "$numstat" | awk '{printf "    file   : %s (+%s -%s)\n", $3, $1, $2}'
  else
    echo "    WARN: cannot parse patch body (git apply --numstat failed)"
    echo "$numstat" | sed 's/^/           /'
  fi
done
echo

echo "== Authors this package would introduce ====================="
sort -u "$AUTHORS_FILE" | sed 's/^/  /'
echo

echo "== Files the series touches that do not exist at base ======="
sort -u "$TOUCHED_FILE" > "$TOUCHED_FILE.sorted"
new_count=0
while IFS= read -r path; do
  [ -n "$path" ] || continue
  if ! git cat-file -e "${BASE_REF}:${path}" 2>/dev/null; then
    echo "  NEW  $path"
    new_count=$((new_count + 1))
  fi
done < "$TOUCHED_FILE.sorted"
unique_count="$(wc -l < "$TOUCHED_FILE.sorted" | tr -d ' ')"
rm -f "$TOUCHED_FILE.sorted"
echo "  ($new_count new path(s), $unique_count unique path(s))"
echo

echo "== Next ====================================================="
echo "1. Confirm the base ref above is the commit the series was cut from."
echo "2. Replay with: scripts/replay-patch-series.sh \"$PATCH_DIR\" <fix-spec-dir> \"$BASE_REF\""
echo "3. Run the project gate chain top-down; expect the series NOT to compile as delivered."