#!/usr/bin/env bash
# sync-after-force-push.sh
#
# Bring the local worktree in line with the remote branch after a force push,
# without ever merging the old history back in.
#
# Usage:
#   sync-after-force-push.sh [remote] [branch] [--dry-run]
#
#   [remote]    Default: origin
#   [branch]    Default: current branch
#   --dry-run   Report the state and exit without resetting.
#
# Why not `git pull`: after a rewrite the local branch and the remote branch have
# no common descendant relationship; a pull would merge the discarded history back
# in. `fetch --force` (the remote ref is stale otherwise) plus `reset --hard` is
# the correct pair.
#
# Refuses to reset when:
#   - the worktree is dirty (would silently discard real work), or
#   - the local branch has commits the remote does not (you would lose them).
#
# Exit codes:
#   0  synced (or dry run completed)
#   1  refused: dirty worktree or unpushed commits
#   2  the remote branch cannot be resolved

set -euo pipefail

ARGS=()
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY_RUN=1 ;;
    *) ARGS+=("$arg") ;;
  esac
done

REMOTE="${ARGS[0]:-origin}"
BRANCH="${ARGS[1]:-$(git rev-parse --abbrev-ref HEAD)}"
TARGET="${REMOTE}/${BRANCH}"

[ "$BRANCH" = "HEAD" ] && { echo "ERROR: detached HEAD; pass the branch explicitly" >&2; exit 1; }

echo "== fetching ${REMOTE} (prune, force) =="
git fetch --prune --force "$REMOTE"

git rev-parse --verify --quiet "$TARGET" >/dev/null || {
  echo "ERROR: cannot resolve ${TARGET} after fetch" >&2
  exit 2
}

ahead="$(git rev-list --count "${TARGET}..HEAD")"
dirty="$(git status --porcelain -uall)"

echo "local  HEAD    : $(git rev-parse --short HEAD)"
echo "remote ${TARGET} : $(git rev-parse --short "$TARGET")"
echo "commits local-only: ${ahead}"
echo "worktree dirty    : $([ -n "$dirty" ] && echo yes || echo no)"
echo

if [ -n "$dirty" ]; then
  echo "REFUSED: worktree is dirty. Commit, stash, or discard deliberately, then re-run." >&2
  git status --short | sed 's/^/  /' >&2
  exit 1
fi

if [ "$ahead" -ne 0 ]; then
  echo "REFUSED: ${ahead} commit(s) exist only locally; resetting would discard them." >&2
  echo "         Inspect with: git log --oneline ${TARGET}..HEAD" >&2
  exit 1
fi

if [ "$DRY_RUN" -eq 1 ]; then
  echo "Dry run: would 'git reset --hard ${TARGET}'."
  exit 0
fi

git reset --hard "$TARGET"
echo
echo "OK. Worktree now matches ${TARGET}."
echo "    If this branch is a submodule pointer, the parent repo will show it as"
echo "    modified; that is a pointer difference, not damage — do not commit it for the user."