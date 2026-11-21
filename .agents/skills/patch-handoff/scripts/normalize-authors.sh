#!/usr/bin/env bash
# normalize-authors.sh
#
# Rewrite authorship for a commit range so every commit carries the same real
# identity, keeping each author date intact.
#
# Usage:
#   normalize-authors.sh <base-ref> <name> <email> [--dry-run]
#
#   <base-ref>  Exclusive lower bound; every commit in <base-ref>..HEAD is rewritten.
#   <name>      Author name, e.g. the repo's convention or `git config user.name`.
#   <email>     Author email.
#   --dry-run   Show what would change and exit.
#
# Notes:
#   - Uses `git filter-branch --env-filter`, which rewrites the range and leaves a
#     backup ref under refs/original/. Run it BEFORE pushing, never after.
#   - `--reset-author` is deliberately not used: it also resets author dates, which
#     destroys the timeline the handoff document describes.
#   - Requires a clean worktree, because filter-branch refuses to run otherwise.
#
# Exit codes:
#   0  authors normalized (or already uniform)
#   1  invalid input / dirty worktree
#   2  verification failed: more than one author remains in the range

set -euo pipefail

ARGS=()
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY_RUN=1 ;;
    *) ARGS+=("$arg") ;;
  esac
done

BASE_REF="${ARGS[0]:?usage: normalize-authors.sh <base-ref> <name> <email> [--dry-run]}"
AUTHOR_NAME="${ARGS[1]:?missing author name}"
AUTHOR_EMAIL="${ARGS[2]:?missing author email}"

cd "$(git rev-parse --show-toplevel)"

# Snapshot the base as a SHA; a moving ref would change meaning mid-run.
BASE_SHA="$(git rev-parse --verify --quiet "${BASE_REF}^{commit}")" || {
  echo "ERROR: cannot resolve base ref '$BASE_REF'" >&2
  exit 1
}

if [ "$(git rev-list --count "$BASE_SHA"..HEAD)" -eq 0 ]; then
  echo "Nothing to rewrite: $BASE_REF..HEAD is empty."
  exit 0
fi

if [ -n "$(git status --porcelain -uall)" ]; then
  echo "ERROR: working tree is dirty; filter-branch needs it clean" >&2
  exit 1
fi

echo "== Authors currently in $BASE_REF..HEAD =="
git log --format="%an <%ae>" "$BASE_SHA"..HEAD | sort | uniq -c | sed 's/^/  /'
echo

if [ "$DRY_RUN" -eq 1 ]; then
  echo "Dry run: would rewrite $(git rev-list --count "$BASE_SHA"..HEAD) commit(s) to:"
  echo "  ${AUTHOR_NAME} <${AUTHOR_EMAIL}>"
  exit 0
fi

FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --env-filter "
  export GIT_AUTHOR_NAME='${AUTHOR_NAME}'
  export GIT_AUTHOR_EMAIL='${AUTHOR_EMAIL}'
" "$BASE_SHA"..HEAD >/dev/null

echo "== Authors after rewrite =="
remaining="$(git log --format="%an <%ae>" "$BASE_SHA"..HEAD | sort -u)"
echo "$remaining" | sed 's/^/  /'

count="$(echo "$remaining" | sed '/^$/d' | wc -l | tr -d ' ')"
if [ "$count" -ne 1 ]; then
  echo "ERROR: ${count} distinct authors remain; inspect with 'git log --format=' before pushing." >&2
  exit 2
fi

echo
echo "OK. Backup ref kept at refs/original/refs/heads/$(git rev-parse --abbrev-ref HEAD)."
echo "    Drop it after a successful push: git update-ref -d <that ref>"