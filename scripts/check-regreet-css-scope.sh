#!/bin/sh
# Guard the published looks against the blank-notification-bar greeter bug.
#
# ReGreet's login card and its clock card carry the `background` CSS class,
# while the third frame -- the empty notification bar above Reboot/Power
# Off -- carries none (read against ReGreet 0.5.0's `src/gui/templates.rs`).
# A bare `frame` rule also inks that empty bar, leaving a blank card when
# there is nothing to report (GTK CSS has no `:empty`). Card rules must
# scope to `frame.background` (and `frame.background > border` for the
# inner border), never bare `frame`.
#
# Scans the published looks (`docs/examples/*/regreet.css`) plus every
# fenced ```css block under `site/src/content/docs/` (any CSS example a
# user would paste) and fails on any selector with an unscoped `frame`
# node: `frame`, `frame > border`, `frame, ...`, `frame:hover`, `.frame`
# -- but not `frame.background`, `frame#id`, `frame[attr]`. `/* */`
# comments and `"..."` string contents never count (the looks' own header
# comments name bare `frame` to warn against it).
#
# Run with no arguments (the CI shape: whole-tree scan from the repo root)
# or with explicit files (fixture testing without committing a bad
# `regreet.css`):
#
#   sh scripts/check-regreet-css-scope.sh
#   sh scripts/check-regreet-css-scope.sh /tmp/bad-look.css /tmp/bad-docs.md
#
# Only `sh` and `awk`: no seat, no user manager, no nix.
set -eu

ROOT_DIR=$(dirname -- "$0")
ROOT=$(cd -- "$ROOT_DIR/.." && pwd -P)
unset ROOT_DIR

AWK='
function check_text(text, loc,   n, parts, k, sel, m, subs, s, j, hit) {
    n = split(text, parts, "{")
    for (k = 1; k < n; k++) {
        sel = parts[k]
        sub(/.*}/, "", sel)
        m = split(sel, subs, ",")
        for (j = 1; j <= m; j++) {
            s = subs[j]
            gsub(/^[ \t\r\n]+|[ \t\r\n]+$/, "", s)
            if (s == "" || substr(s, 1, 1) == "@") continue
            # Unscoped `frame` node: a `frame` (or `.frame`) compound head
            # whose next character is not a class, id, attribute or
            # type-name continuation. `frame.background` passes;
            # `frame:hover`, `frame > border`, bare `frame` fail.
            if (match(s, /(^|[ \t\r\n,>+~])\.?frame([^A-Za-z0-9_#.\[-]|$)/)) {
                printf "%s: unscoped frame selector %s\n", loc, s
                hit = 1
            }
        }
    }
    return hit
}
function feed_line(line, loc,   rest, p) {
    gsub(/"[^"]*"/, "\"\"", line)
    rest = line
    while (rest != "") {
        if (in_comment) {
            p = index(rest, "*/")
            if (p == 0) { rest = "" }
            else { rest = substr(rest, p + 2); in_comment = 0 }
        } else {
            p = index(rest, "/*")
            if (p == 0) { buf = buf rest "\n"; rest = "" }
            else { buf = buf substr(rest, 1, p - 1); rest = substr(rest, p + 2); in_comment = 1 }
        }
    }
}
BEGIN { in_comment = 0; buf = ""; fails = 0; fence = 0; blockn = 0 }
{
    if (mode == "md") {
        if (fence) {
            if ($0 ~ /^```[ \t]*$/) {
                fence = 0
                blockn++
                if (check_text(buf, FILENAME " (css block " blockn ")")) fails = 1
                buf = ""
            } else {
                feed_line($0, "")
            }
        } else if ($0 ~ /^```[Cc][Ss][Ss]([ \t]+.*)?$/) {
            fence = 1
            buf = ""
        }
    } else {
        feed_line($0, "")
    }
}
END {
    if (mode != "md" && check_text(buf, FILENAME)) fails = 1
    exit (fails ? 1 : 0)
}
'

fail_list=""
pass_count=0

check_one() {
    # $1 = file path, $2 = mode (css|md)
    if out=$(awk -v mode="$2" "$AWK" "$1" 2>&1); then
        pass_count=$((pass_count + 1))
    else
        fail_list="${fail_list}${out}
"
    fi
}

if [ "$#" -gt 0 ]; then
    for f in "$@"; do
        case "$f" in
            *.md | *.mdx) check_one "$f" "md" ;;
            *) check_one "$f" "css" ;;
        esac
    done
else
    for f in "$ROOT"/docs/examples/*/regreet.css; do
        check_one "$f" "css"
    done
    # Test-look ReGreet leaves: a fixture that themes the login screen in
    # the Nix checks must obey the same scoping.
    for f in "$ROOT"/nix/test-looks/*/apps/regreet.css; do
        [ -e "$f" ] || continue
        check_one "$f" "css"
    done
    # Every docs page, nested sections included: a page this never scans
    # is a page whose CSS example can rot into the blank-card bug.
    # shellcheck disable=SC2044
    for f in $(find "$ROOT/site/src/content/docs" \( -name '*.md' -o -name '*.mdx' \)); do
        check_one "$f" "md"
    done
fi

if [ -n "$fail_list" ]; then
    printf 'regreet card-scope gate: FAIL\n'
    printf '%s' "$fail_list"
    printf 'Scope card rules to `frame.background` (and `frame.background > border`\n'
    printf 'for the inner border), never bare `frame`: a bare `frame` rule also inks\n'
    printf "ReGreet's empty notification bar above Reboot/Power Off, leaving a blank\n"
    printf 'card when there is nothing to report (GTK CSS has no `:empty`).\n'
    exit 1
fi

echo "regreet card-scope gate: pass (${pass_count} files scanned)"
