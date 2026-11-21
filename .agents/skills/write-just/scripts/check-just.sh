#!/bin/sh
# check-just.sh - scan a project's justfile and managed .just modules for
# heavy commands that bypass the bque queue on Windows.
#
# This script is part of the write-just skill. Per repository convention,
# skill scripts are always English (identifiers, comments, log output).
# Read-only: it never modifies files.

set -u

usage() {
    cat <<'EOF'
Usage: check-just.sh [--dir <project-root>]

Scan <project-root>/justfile and <project-root>/.pengj-templates/just/*.just
for raw heavy commands (cargo run/build/clippy/test/bench/check, ninja,
cmake --build, ctest, pnpm build, tsc) that are not wrapped by bque.
Only a direct `bque run -- <command>` / `bque bench -- <command>` counts as
queued; the layers ship no ad-hoc `just` wrapper.

Rules:
  - [unix] recipes pass through raw commands by design and are skipped.
  - When the bque layer is active, an unwrapped command in a [windows]
    recipe or in an un-attributed recipe is an ERROR.
  - When the bque layer is absent, the same finding is only an INFO note
    (the legitimate downgrade of the current layer).
  - A direct bque call inside a [unix] recipe is a WARNING.

Exit codes:
  0  no errors (info/warning notes may still be printed)
  1  at least one error was found
  2  usage error (bad argument or missing directory)
EOF
}

PROJECT_DIR="."

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --dir)
            if [ "$#" -lt 2 ]; then
                echo "ERROR: --dir requires a path" >&2
                exit 2
            fi
            PROJECT_DIR="$2"
            shift 2
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ ! -d "$PROJECT_DIR" ]; then
    echo "ERROR: not a directory: $PROJECT_DIR" >&2
    exit 2
fi

# Detect whether the project selected the bque layer.
has_bque=0
if [ -f "$PROJECT_DIR/.pengj-templates/manifest.json" ]; then
    if grep -q '"bque"' "$PROJECT_DIR/.pengj-templates/manifest.json"; then
        has_bque=1
    fi
fi

# Collect target files into positional parameters (handles paths with spaces).
set --
if [ -f "$PROJECT_DIR/justfile" ]; then
    set -- "$@" "$PROJECT_DIR/justfile"
fi
for module in "$PROJECT_DIR"/.pengj-templates/just/*.just; do
    if [ -f "$module" ]; then
        set -- "$@" "$module"
    fi
done

if [ "$#" -eq 0 ]; then
    echo "No justfile or managed .just modules found under $PROJECT_DIR"
    exit 0
fi

awk -v has_bque="$has_bque" '
BEGIN { errors = 0; warnings = 0; infos = 0; files = 0; checked = 0 }

FNR == 1 {
    files++
    pending = "any"
    current = "any"
}

{
    line = $0
    trimmed = line
    sub(/^[[:space:]]+/, "", trimmed)

    # Platform attributes bind to the next recipe header.
    if (trimmed ~ /^\[windows\]/) { pending = "windows"; next }
    if (trimmed ~ /^\[unix\]/)    { pending = "unix"; next }
    if (trimmed ~ /^\[/)          { next }

    # Blank lines and comments are documentation, not commands.
    if (trimmed == "" || trimmed ~ /^#/) next

    # Top-level directives do not start a recipe body.
    if (trimmed ~ /^(set|import|mod|export|alias)[[:space:]]/) {
        current = "any"
        next
    }

    # Recipe header: consume the pending attribute for this recipe body.
    if (line !~ /^[[:space:]]/) {
        current = (trimmed ~ /:/) ? pending : "any"
        pending = "any"
        next
    }

    # Command body line.
    cmd = trimmed
    sub(/^@/, "", cmd)
    checked++

    heavy = (cmd ~ /(^|[^[:alnum:]_-])cargo[[:space:]]+(run|build|clippy|test|bench|check)([^[:alnum:]_-]|$)/ || \
             cmd ~ /(^|[^[:alnum:]_-])ninja([^[:alnum:]_-]|$)/ || \
             cmd ~ /cmake[[:space:]]+--build/ || \
             cmd ~ /(^|[^[:alnum:]_-])ctest([^[:alnum:]_-]|$)/ || \
             cmd ~ /pnpm[[:space:]]+(build|run)/ || \
             cmd ~ /(^|[^[:alnum:]_-])tsc([^[:alnum:]_-]|$)/)
    # A direct bque call (including --cwd style) is the only queued form.
    direct_bque = (cmd ~ /bque[[:space:]]+(run|bench)[[:space:]]+/)
    wrapped = direct_bque

    if (direct_bque && current == "unix") {
        warnings++
        print "WARN: " FILENAME ":" FNR ": bque call inside a [unix] recipe; Unix should pass through"
        next
    }

    if (!heavy || wrapped) next

    label = "heavy command"
    n = split(cmd, parts, /[[:space:]]+/)
    for (i = 1; i < n; i++) {
        if (parts[i] == "cargo") { label = "cargo " parts[i + 1]; break }
        if (parts[i] == "ninja")  { label = "ninja"; break }
        if (parts[i] == "cmake")  { label = "cmake --build"; break }
        if (parts[i] == "ctest")  { label = "ctest"; break }
        if (parts[i] == "pnpm")   { label = "pnpm " parts[i + 1]; break }
        if (parts[i] == "tsc")    { label = "tsc"; break }
    }

    where = (current == "windows") ? "[windows]" : (current == "unix") ? "[unix]" : "no attribute"

    # Unix branches pass raw commands through by design.
    if (current == "unix") next

    if (has_bque) {
        errors++
        print "ERROR: " FILENAME ":" FNR ": " where " recipe runs raw " label " - use bque run -- " label
    } else {
        infos++
        print "INFO: " FILENAME ":" FNR ": " where " recipe runs raw " label " - no bque layer; direct command is the current downgrade"
    }
}

END {
    print "Scanned " files " file(s), " checked " command line(s): " \
          errors " error(s), " warnings " warning(s), " infos " info note(s)."
    if (errors > 0) exit 1
    exit 0
}
' "$@"