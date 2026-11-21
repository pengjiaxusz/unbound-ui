#!/usr/bin/env bash
# replay-patch-series.sh
#
# Replay an ordered patch series with `git am`, then fold repair files back into
# the commit that introduced each defect (so the 1 commit == 1 ticket mapping holds).
#
# Usage:
#   replay-patch-series.sh <patches-dir> <fix-spec-dir> [base-ref]
#
#   <patches-dir>   Directory containing 0001-*.patch, 0002-*.patch, ...
#   <fix-spec-dir>  Layout: fixes/0003/<repo-relative-path>
#                   Meaning: after patch 0003 is applied, overwrite that path with
#                   the given file, then amend the commit.
#   [base-ref]      Branch to replay onto. Default: origin/main
#
# Preconditions:
#   - Working tree is clean and the current branch has no commits outside <base-ref>.
#   - Every overwritten file is touched by exactly one commit in the series:
#       git log --oneline <base-ref>..HEAD -- <file> | wc -l  ==  1
#
# Exit codes:
#   0  success
#   1  precondition failed
#   2  `git am` stopped on a conflict; the work tree is left in place for inspection

set -euo pipefail

PATCHES_DIR="${1:?usage: replay-patch-series.sh <patches-dir> <fix-spec-dir> [base-ref]}"
FIX_DIR="${2:?missing fix-spec-dir (use an empty directory to disable fixes)}"
BASE_REF="${3:-origin/main}"

cd "$(git rev-parse --show-toplevel)"

if [ -n "$(git status --porcelain -uall)" ]; then
  echo "ERROR: working tree is dirty; commit or stash before replaying" >&2
  exit 1
fi

if [ "$(git rev-list --count "$BASE_REF"..HEAD)" -ne 0 ]; then
  echo "ERROR: current branch has commits not in '$BASE_REF'; start from a clean base" >&2
  exit 1
fi

for patch in "$PATCHES_DIR"/*.patch; do
  name="$(basename "$patch")"
  num="${name%%-*}"
  echo "==> ${name}"

  if ! git am -q "$patch"; then
    echo "ERROR: ${name} failed to apply. Inspect with 'git am --show-current-patch'," >&2
    echo "       abort with 'git am --abort', then retry." >&2
    exit 2
  fi

  if [ -d "${FIX_DIR}/${num}" ]; then
    while IFS= read -r -d '' fixed; do
      rel="${fixed#"${FIX_DIR}/${num}/"}"
      cp "$fixed" "$rel"
      git add "$rel"
      echo "    folded fix: ${rel}"
    done < <(find "${FIX_DIR}/${num}" -type f -print0)
    git commit -q --amend --no-edit
  fi
done

echo
echo "Done. Resulting commits:"
git log --oneline "${BASE_REF}"..HEAD
echo
echo "Next: normalize author (filter-branch --env-filter), push --force-with-lease,"
echo "      then sync the local work tree with 'fetch --prune --force' + 'reset --hard'."
