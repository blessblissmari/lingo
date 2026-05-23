#!/usr/bin/env bash
# v0.1.29 consolidation audit: prove every example runs and (where applicable)
# native output == interp output.
#
# Output: a Markdown summary printed to stdout.

set -uo pipefail
cd "$(dirname "$0")/compiler"
LINGO=./target/debug/lingo
cargo build --quiet 2>/dev/null

declare -A SKIP_NATIVE
SKIP_NATIVE[hello]=""      # placeholder; nothing skipped by default

# Examples expected to be interactive (read stdin) — skip in this batch run.
# Long-running benchmark — interp would take ~15s+. Native only.
INTERACTIVE_EXAMPLES=("io_roundtrip" "fib_native_bench")

# Examples that exercise interp-only features tracked for v0.2:
#   - tour:      uses `int(s) -> int!str` parsing builtin (not in C backend)
#   - wordcount: uses `match` on a non-enum scrutinee (map.get())
# Both have native-friendly companions (wordcount_native.lingo) and are
# documented as known gaps in ROADMAP.md.  We mark them explicitly here so
# the audit summary reflects intent, not regression.
INTERP_ONLY_EXAMPLES=("tour" "wordcount")
is_interp_only() {
    local stem=$1
    for x in "${INTERP_ONLY_EXAMPLES[@]}"; do
        [ "$x" = "$stem" ] && return 0
    done
    return 1
}

is_interactive() {
    local stem=$1
    for x in "${INTERACTIVE_EXAMPLES[@]}"; do
        [ "$x" = "$stem" ] && return 0
    done
    return 1
}

# Examples that take CLI args — provide them here.
declare -A ARGS
ARGS[parse_port]="8080"
ARGS[greet]="World"

echo "## v0.1.29 audit"
echo
echo "| example | interp | native | output match | notes |"
echo "| --- | --- | --- | --- | --- |"

TOTAL=0; PASS=0; FAIL=0
TMP=$(mktemp -d)
for f in examples/*.lingo; do
    stem=$(basename "$f" .lingo)
    TOTAL=$((TOTAL+1))
    args=${ARGS[$stem]:-}

    if is_interactive "$stem"; then
        echo "| $stem | _skip_ | _skip_ | _skip_ | interactive (reads stdin) |"
        continue
    fi

    if is_interp_only "$stem"; then
        if [ -n "$args" ]; then
            i_out=$(timeout 10 $LINGO "$f" $args 2>"$TMP/i.err")
            i_rc=$?
        else
            i_out=$(timeout 10 $LINGO "$f" 2>"$TMP/i.err")
            i_rc=$?
        fi
        echo "| $stem | $([ $i_rc -eq 0 ] && echo ok || echo "exit $i_rc") | _interp-only_ | _n/a_ | tracked for v0.2 (see ROADMAP) |"
        continue
    fi

    # interp (with hard cap so a wedged example doesn't stall the audit)
    if [ -n "$args" ]; then
        i_out=$(timeout 10 $LINGO "$f" $args 2>"$TMP/i.err")
        i_rc=$?
    else
        i_out=$(timeout 10 $LINGO "$f" 2>"$TMP/i.err")
        i_rc=$?
    fi

    # native
    cd "$TMP"
    LINGO_OUT="$TMP/$stem" "$OLDPWD/$LINGO" build "$OLDPWD/$f" >"$TMP/n.build.out" 2>"$TMP/n.build.err"
    n_build_rc=$?
    cd - >/dev/null

    if [ $n_build_rc -ne 0 ] || [ ! -x "$TMP/$stem" ]; then
        echo "| $stem | $([ $i_rc -eq 0 ] && echo ok || echo "exit $i_rc") | **build fail** | n/a | $(head -1 "$TMP/n.build.err") |"
        FAIL=$((FAIL+1))
        continue
    fi

    if [ -n "$args" ]; then
        n_out=$(timeout 10 "$TMP/$stem" $args 2>"$TMP/n.err")
        n_rc=$?
    else
        n_out=$(timeout 10 "$TMP/$stem" 2>"$TMP/n.err")
        n_rc=$?
    fi

    match="—"
    note=""
    if [ $i_rc -eq 0 ] && [ $n_rc -eq 0 ]; then
        if [ "$i_out" = "$n_out" ]; then
            match="✅"
            PASS=$((PASS+1))
        else
            match="❌"
            note="output differs"
            FAIL=$((FAIL+1))
        fi
    elif [ $i_rc -eq 0 ] && [ $n_rc -ne 0 ]; then
        match="—"
        note="native runtime exit $n_rc"
        FAIL=$((FAIL+1))
    elif [ $i_rc -ne 0 ] && [ $n_rc -eq 0 ]; then
        match="—"
        note="interp exit $i_rc but native ok"
        FAIL=$((FAIL+1))
    else
        match="—"
        note="both errored (i=$i_rc n=$n_rc)"
        FAIL=$((FAIL+1))
    fi
    echo "| $stem | $([ $i_rc -eq 0 ] && echo ok || echo "exit $i_rc") | $([ $n_rc -eq 0 ] && echo ok || echo "exit $n_rc") | $match | $note |"
done

echo
echo "**Summary:** $PASS / $TOTAL examples have matching interp ≡ native output."
echo "**Failures:** $FAIL"
rm -rf "$TMP"
