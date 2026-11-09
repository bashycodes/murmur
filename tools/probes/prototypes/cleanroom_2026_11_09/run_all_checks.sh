#!/usr/bin/env bash
# usage: run_all_checks.sh — every pre-commit check for this branch, in a row
# (workspace test + clippy, Windows test + clippy, Gradle unit tests).
here="$(cd "$(dirname "$0")" && pwd)"
c="$here/run_checks.sh"
win=(--manifest-path apps/flow-windows/Cargo.toml --no-default-features --features local-engine,remote,tls,cloud,download)
echo "== test";    bash "$c" test 2>&1 | grep -E "^test result|FAILED|panicked|error(\[|:)|^rc=" | grep -v " 0 failed" | sort | uniq -c
echo "== clippy";  bash "$c" clippy 2>&1 | grep -E "^(warning|error)|^rc=" | sort | uniq -c
echo "== win test";   bash "$c" cargo test "${win[@]}" 2>&1 | grep -E "^test result|FAILED|panicked|error(\[|:)|^rc=" | grep -v " 0 failed" | sort | uniq -c
echo "== win clippy"; bash "$c" cargo clippy "${win[@]}" --all-targets -- -D warnings 2>&1 | grep -E "^(warning|error)|^rc=" | sort | uniq -c
echo "== gradle";  bash "$c" gradle 2>&1 | grep -E "BUILD|FAILED|tests completed|^rc=" | tail -5
