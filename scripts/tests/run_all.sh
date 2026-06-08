#!/usr/bin/env bash
# Test runner for scripts/tests/. Iterates over each `*_test.sh`
# in this directory and runs it. Returns non-zero if any
# test fails so CI can pin the suite.
#
# Run: bash scripts/tests/run_all.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"

FAIL=0
PASS=0
for t in "$HERE"/*_test.sh; do
    [ -e "$t" ] || continue
    name="$(basename "$t")"
    echo
    echo "==> $name"
    if bash "$t"; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
done

echo
echo "==> summary: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
