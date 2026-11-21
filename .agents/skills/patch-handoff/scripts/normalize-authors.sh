#!/usr/bin/env bash
# normalize-authors.sh
#
# Rewrite authorship for a commit range so every commit carries the repository
# owner's real identity, keeping each author date intact.
#
# Usage:
#   normalize-authors.sh <base-ref> [name] [email] [--dry-run] [--print-identity]
#
#   <base-ref>        Exclusive lower bound; every commit in <base-ref>..HEAD is rewritten.
#   [name]            Author name. OMIT to derive it from the owner of `origin`.
#   [email]           Author email. OMIT to use `git config user.email`.
#   --dry-run, -n     Show the resolved target and current authors, then exit.
#   --print-identity  Print the identity that WOULD be used as `name <email>`, then exit.
#
# Identity resolution (per argument, independently):
#   name  -> <name> if given, else the owner segment of `git remote get-url origin`
#            (git@github.com:owner/repo.git, https://host/owner/repo.git -> owner)
#   email -> <email> if given, else `git config user.email`
# Never take either value from the handoff package or from the agent's guess.
#
# Notes:
#   - Uses `git filter-branch --env-filter`, which rewrites the range and leaves a
#     backup ref under refs/original/. Run it BEFORE pushing, never after.
#   - `--reset-author` is deliberately not used: it also resets author dates, which
#     destroys the timeline the handoff document describes.
#   - The filter reads exported variables instead of interpolating into its source,
#     so names/emails containing quotes or spaces stay safe.
#   - Requires a clean worktree, because filter-branch refuses to run otherwise.
#
# Exit codes:
#   0  authors normalized (or already exactly the target identity)
#   1  invalid input / dirty worktree / identity cannot be resolved
#   2  verification failed: the range does not carry exactly the target identity

set -euo pipefail

POSITIONAL=()
DRY_RUN=0
PRINT_IDENTITY=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY_RUN=1 ;;
    --print-identity) PRINT_IDENTITY=1 ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done

BASE_REF="${POSITIONAL[0]:?usage: normalize-authors.sh <base-ref> [name] [email] [--dry-run] [--print-identity]}"

cd "$(git rev-parse --show-toplevel)"

# --- Resolve the target identity from the repository, never from the package ---
resolve_owner() {
  local url path
  url="$(git remote get-url origin 2>/dev/null || true)"
  if [ -z "$url" ]; then
    echo "ERROR: no 'origin' remote; cannot derive the repo owner. Pass <name> explicitly." >&2
    return 1
  fi
  # SCP form:  git@host:owner/repo.git   -> after ':' is owner/repo.git
  # URL form:  https://host/owner/repo.git -> strip scheme//host/ -> owner/repo.git
  case "$url" in
    *://*) path="${url#*://}"; path="${path#*/}" ;;
    *)     path="${url#*:}" ;;
  esac
  path="${path%.git}"
  path="${path%/}"
  local owner="${path%%/*}"
  if [ -z "$owner" ] || [ "$owner" = "$path" ]; then
    echo "ERROR: cannot parse an owner from origin URL '$url'. Pass <name> explicitly." >&2
    return 1
  fi
  printf '%s' "$owner"
}

resolve_email() {
  local email
  email="$(git config user.email 2>/dev/null || true)"
  if [ -z "$email" ]; then
    echo "ERROR: git config user.email is unset; pass <email> explicitly." >&2
    return 1
  fi
  printf '%s' "$email"
}

AUTHOR_NAME="${POSITIONAL[1]:-}"
AUTHOR_EMAIL="${POSITIONAL[2]:-}"
[ -n "$AUTHOR_NAME" ] || AUTHOR_NAME="$(resolve_owner)"
[ -n "$AUTHOR_EMAIL" ] || AUTHOR_EMAIL="$(resolve_email)"
TARGET="${AUTHOR_NAME} <${AUTHOR_EMAIL}>"

if [ "$PRINT_IDENTITY" -eq 1 ]; then
  printf '%s\n' "$TARGET"
  exit 0
fi

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

echo "== Target identity (resolved from this repository) =="
echo "  $TARGET"
echo "== Authors currently in $BASE_REF..HEAD =="
git log --format="%an <%ae>" "$BASE_SHA"..HEAD | sort | uniq -c | sed 's/^/  /'
echo

if [ "$DRY_RUN" -eq 1 ]; then
  echo "Dry run: would rewrite $(git rev-list --count "$BASE_SHA"..HEAD) commit(s) to:"
  echo "  $TARGET"
  exit 0
fi

# Export instead of interpolating: the filter body stays single-quoted, so a name
# or email containing quotes/spaces cannot break or inject into the shell source.
export PT_AUTHOR_NAME="$AUTHOR_NAME"
export PT_AUTHOR_EMAIL="$AUTHOR_EMAIL"
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --env-filter '
  export GIT_AUTHOR_NAME="$PT_AUTHOR_NAME"
  export GIT_AUTHOR_EMAIL="$PT_AUTHOR_EMAIL"
' "$BASE_SHA"..HEAD >/dev/null

echo "== Authors after rewrite =="
remaining="$(git log --format="%an <%ae>" "$BASE_SHA"..HEAD | sort -u)"
echo "$remaining" | sed 's/^/  /'

if [ "$remaining" != "$TARGET" ]; then
  echo "ERROR: authors in $BASE_REF..HEAD are not exactly '$TARGET'; inspect with" >&2
  echo "       git log --format='%an <%ae>' before pushing." >&2
  exit 2
fi

echo
echo "OK. All commits in $BASE_REF..HEAD are authored by $TARGET (author dates kept)."
echo "Backup ref kept at refs/original/refs/heads/$(git rev-parse --abbrev-ref HEAD)."
echo "    Drop it after a successful push: git update-ref -d <that ref>"