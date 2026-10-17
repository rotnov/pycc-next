#!/usr/bin/env bash
# CI wrapper around `cargo test`: runs every test (no fail-fast) with the given
# test-binary arguments, keeping the full output in ci-logs/cargo-test.log.
# On failure it writes the failed tests and their panic output to the job
# summary ($GITHUB_STEP_SUMMARY) and reruns each failed test on its own with
# RUST_BACKTRACE=1 and --nocapture into ci-logs/failed/<test>.log (uploaded as
# an artifact), so a red job says what broke without re-running anything.
#
# Usage: scripts/ci_test.sh [test-binary args...]
#   e.g. scripts/ci_test.sh --include-ignored --skip slow_test
set -uo pipefail

logs=ci-logs
mkdir -p "$logs/failed"
summary=${GITHUB_STEP_SUMMARY:-/dev/null}

cargo test --release --workspace --no-fail-fast -- "$@" 2>&1 | tee "$logs/cargo-test.log"
status=${PIPESTATUS[0]}
[ "$status" -eq 0 ] && exit 0

# (No mapfile: macOS ships bash 3.2.)
failed=()
while IFS= read -r t; do failed+=("$t"); done \
    < <(sed -nE 's/^test (.+) \.\.\. FAILED$/\1/p' "$logs/cargo-test.log" | sort -u)
{
    echo "## cargo test failed (${#failed[@]} tests) on $(uname -sm)"
    if [ "${#failed[@]}" -eq 0 ]; then
        echo
        echo "No test reported FAILED: a test binary or the build failed. Last lines:"
        echo '```'
        tail -n 60 "$logs/cargo-test.log"
        echo '```'
    fi
    for t in ${failed[@]+"${failed[@]}"}; do
        echo
        echo "<details><summary><code>$t</code></summary>"
        echo
        echo '```'
        # The test's captured output: from its `---- <name> stdout ----` header
        # to the next header or the `failures:` list.
        awk -v t="---- $t stdout ----" '
            $0 == t { on = 1; next }
            on && (/^---- .* stdout ----$/ || /^failures:$/) { exit }
            on { print }
        ' "$logs/cargo-test.log" | head -n 80
        echo '```'
        echo "</details>"
    done
} >> "$summary"

# Rerun at most 20 failed tests alone, with a backtrace and live output. A
# test name may exist in several binaries; `--exact` keeps the match tight.
n=0
for t in ${failed[@]+"${failed[@]}"}; do
    n=$((n + 1))
    [ "$n" -gt 20 ] && break
    safe=$(printf '%s' "$t" | tr -c 'A-Za-z0-9_.-' '_')
    RUST_BACKTRACE=1 cargo test --release --workspace -- "$@" --exact "$t" --nocapture \
        > "$logs/failed/$safe.log" 2>&1 || true
done
# A test binary that crashed (an access violation, a signal) reports no
# FAILED line: rerun it on one thread, so the log ends at the test that
# crashed, into ci-logs/crashed/<binary>.log, and put its tail in the summary.
mkdir -p "$logs/crashed"
sed -nE "s/^ *process didn't exit successfully: \`([^ ]+) .*/\1/p" "$logs/cargo-test.log" | sort -u |
while IFS= read -r exe; do
    if command -v cygpath > /dev/null; then exe=$(cygpath -u "$exe"); fi
    name=$(basename "$exe")
    RUST_BACKTRACE=1 "$exe" "$@" --test-threads=1 > "$logs/crashed/$name.log" 2>&1 || true
    {
        echo
        echo "<details><summary>crashed: <code>$name</code> (rerun on one thread)</summary>"
        echo
        echo '```'
        tail -n 30 "$logs/crashed/$name.log"
        echo '```'
        echo "</details>"
    } >> "$summary"
done
echo "Full output and per-test reruns: artifact ci-logs-$(uname -m)" >> "$summary"
exit "$status"
