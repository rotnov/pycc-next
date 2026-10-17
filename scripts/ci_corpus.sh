#!/usr/bin/env bash
# CI wrapper around bench/corpus.py: passes its arguments through with
# --show-errors, keeps the output in ci-logs/corpus.log, and on failure copies
# the report's tail (the failing programs and why) to the job summary.
#
# Usage: scripts/ci_corpus.sh [corpus.py args...]
set -uo pipefail
mkdir -p ci-logs
python3 bench/corpus.py --show-errors "$@" 2>&1 | tee ci-logs/corpus.log
status=${PIPESTATUS[0]}
if [ "$status" -ne 0 ]; then
    {
        echo "## corpus failed ($*) on $(uname -sm)"
        echo '```'
        tail -n 80 ci-logs/corpus.log
        echo '```'
    } >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
fi
exit "$status"
