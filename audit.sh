#!/usr/bin/env bash
# v0.3.0 consolidation audit: prove every example runs and (where applicable)
# native output == interp output.
#
# Examples now come in two shapes:
#   1. `examples/foo.lingo`           — single-file program
#   2. `examples/{name}/main.lingo`   — multi-file module program (v0.3.0+)
#
# Both shapes are picked up automatically; the stem reported in the table is
# `foo` for case 1 and `{name}` for case 2.
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

# Examples that exercise interp-only features tracked for v0.2.x:
# (empty as of v0.2.2 — `tour.lingo` graduated to interp ≡ native after the
# `? else <expr>` error-type coercion landed.  `wordcount.lingo` graduated
# in v0.2.1.  Keep the list around: future drift may re-introduce gaps.)
INTERP_ONLY_EXAMPLES=()
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

echo "## v0.3.0 audit"
echo
echo "| example | interp | native | output match | notes |"
echo "| --- | --- | --- | --- | --- |"

# Build the full list of entry files: single-file `examples/foo.lingo`
# plus multi-file `examples/{name}/main.lingo`.  Sort for a deterministic
# table order across runs.
ENTRIES=()
for f in examples/*.lingo; do
    [ -e "$f" ] || continue
    ENTRIES+=("$f")
done
for d in examples/*/; do
    main="${d}main.lingo"
    [ -f "$main" ] && ENTRIES+=("$main")
done
IFS=$'\n' ENTRIES=($(sort <<<"${ENTRIES[*]}")); unset IFS

TOTAL=0; PASS=0; FAIL=0
TMP=$(mktemp -d)
for f in "${ENTRIES[@]}"; do
    # Stem: single-file uses the filename; multi-file uses its directory
    # name so the table reads `modules_basic` instead of `main`.
    if [[ "$f" == */*/main.lingo ]]; then
        stem=$(basename "$(dirname "$f")")
    else
        stem=$(basename "$f" .lingo)
    fi
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

    # native — build inside its own subdir of $TMP so multi-file
    # programs (which all build to `./main`) don't clobber each
    # other's binaries.
    bin_dir="$TMP/$stem"
    mkdir -p "$bin_dir"
    cd "$bin_dir"
    "$OLDPWD/$LINGO" build "$OLDPWD/$f" >"$TMP/n.build.out" 2>"$TMP/n.build.err"
    n_build_rc=$?
    cd - >/dev/null

    # The compiler names the binary after the entry file's stem.
    if [[ "$f" == */*/main.lingo ]]; then
        bin_name="main"
    else
        bin_name="$stem"
    fi
    bin_path="$bin_dir/$bin_name"

    if [ $n_build_rc -ne 0 ] || [ ! -x "$bin_path" ]; then
        echo "| $stem | $([ $i_rc -eq 0 ] && echo ok || echo "exit $i_rc") | **build fail** | n/a | $(head -1 "$TMP/n.build.err") |"
        FAIL=$((FAIL+1))
        continue
    fi

    if [ -n "$args" ]; then
        n_out=$(timeout 10 "$bin_path" $args 2>"$TMP/n.err")
        n_rc=$?
    else
        n_out=$(timeout 10 "$bin_path" 2>"$TMP/n.err")
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
