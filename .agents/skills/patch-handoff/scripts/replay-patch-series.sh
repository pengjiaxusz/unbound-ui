#!/usr/bin/env bash
# replay-patch-series.sh
#
# Replay an ordered patch series with `git am`, then fold repair files back into
# the commit that introduced each defect (so one commit still maps to one ticket).
#
# Usage:
#   replay-patch-series.sh <patches-dir> [fix-spec-dir] [base-ref] [--dry-run]
#
#   <patches-dir>   Directory containing 0001-*.patch, 0002-*.patch, ...
#   [fix-spec-dir]  Layout: <fix-spec-dir>/0003/<repo-relative-path>
#                   Meaning: after patch 0003 is applied, overwrite that path with
#                   the given file, then amend that commit. Omit to disable folding.
#   [base-ref]      Branch to replay onto. Default: origin/main
#   --dry-run       Print the plan (patches, folds, fold preconditions) and exit.
#
# Preconditions:
#   - Working tree is clean and the current branch has no commits outside <base-ref>.
#   - Every overwritten file is touched by exactly one commit in the series:
#       git log --oneline <base-ref>..HEAD -- <file> | wc -l  ==  1
#     The script verifies this per file during replay and aborts (exit 3) instead
#     of misattributing a repair; nothing is ever force-folded.
#
# Exit codes:
#   0  success (or dry run completed)
#   1  precondition failed
#   2  `git am` stopped on a conflict; the worktree is left in place for inspection
#   3  a fold target violates the single-touch precondition

set -euo pipefail

ARGS=()
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY_RUN=1 ;;
    *) ARGS+=("$arg") ;;
  esac
done

PATCHES_DIR="${ARGS[0]:?usage: replay-patch-series.sh <patches-dir> [fix-spec-dir] [base-ref] [--dry-run]}"
FIX_DIR="${ARGS[1]:-}"
BASE_REF="${ARGS[2]:-origin/main}"

[ -d "$PATCHES_DIR" ] || { echo "ERROR: no such directory: $PATCHES_DIR" >&2; exit 1; }

cd "$(git rev-parse --show-toplevel)"

# Resolve the base to a SHA up front: a moving ref such as HEAD or a branch would
# otherwise advance with every `git am` and break the range arithmetic below.
BASE_SHA="$(git rev-parse --verify --quiet "${BASE_REF}^{commit}")" || {
  echo "ERROR: cannot resolve base ref '$BASE_REF'" >&2
  exit 1
}

if [ -n "$(git status --porcelain -uall)" ]; then
  echo "ERROR: working tree is dirty; commit or stash before replaying" >&2
  exit 1
fi

if [ "$(git rev-list --count "$BASE_SHA"..HEAD)" -ne 0 ]; then
  echo "ERROR: current branch has commits not in '$BASE_REF'; start from a clean base" >&2
  exit 1
fi

shopt -s nullglob
PATCHES=("$PATCHES_DIR"/*.patch)
shopt -u nullglob
PATCH_COUNT=0
for _p in "${PATCHES[@]}"; do PATCH_COUNT=$((PATCH_COUNT + 1)); done
if [ "$PATCH_COUNT" -eq 0 ]; then
  echo "ERROR: no *.patch in $PATCHES_DIR" >&2
  exit 1
fi

if [ "$DRY_RUN" -eq 1 ]; then
  echo "== dry run: base $BASE_REF, $PATCH_COUNT patch(es) =="
fi

for patch in "${PATCHES[@]}"; do
  name="$(basename "$patch")"
  num="${name%%-*}"
  subject="$(git mailinfo /dev/null /dev/null < "$patch" 2>/dev/null | sed -n 's/^Subject: //p' | head -1)"

  if [ "$DRY_RUN" -eq 1 ]; then
    echo "--- $name  $subject"
    if [ -n "$FIX_DIR" ] && [ -d "${FIX_DIR}/${num}" ]; then
      while IFS= read -r -d '' fixed; do
        rel="${fixed#"${FIX_DIR}/${num}/"}"
        echo "    fold: ${rel}"
      done < <(find "${FIX_DIR}/${num}" -type f -print0)
    fi
    continue
  fi

  echo "==> ${name}"
  if ! git am -q "$patch"; then
    echo "ERROR: ${name} failed to apply." >&2
    echo "       Inspect the intent with 'git am --show-current-patch'," >&2
    echo "       resume with 'git am --continue', or back out with 'git am --abort'." >&2
    exit 2
  fi

  if [ -n "$FIX_DIR" ] && [ -d "${FIX_DIR}/${num}" ]; then
    while IFS= read -r -d '' fixed; do
      rel="${fixed#"${FIX_DIR}/${num}/"}"
      touches="$(git log --oneline "$BASE_SHA"..HEAD -- "$rel" | wc -l | tr -d ' ')"
      if [ "$touches" -ne 1 ]; then
        echo "ERROR: ${rel} is touched by ${touches} commits in the series;" >&2
        echo "       folding would misattribute the repair. Fix it in a final commit instead." >&2
        exit 3
      fi
      cp "$fixed" "$rel"
      git add "$rel"
      echo "    folded fix: ${rel}"
    done < <(find "${FIX_DIR}/${num}" -type f -print0)
    git commit -q --amend --no-edit
  fi
done

if [ "$DRY_RUN" -eq 1 ]; then
  echo
  echo "Dry run only. Re-run without --dry-run to apply."
  exit 0
fi

echo
echo "Done. Resulting commits:"
git log --oneline "$BASE_SHA"..HEAD
echo
echo "Next: run the project gate chain, repair what it reports, fold each repair back,"
echo "      then normalize authorship and push with --force-with-lease."