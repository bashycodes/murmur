#!/usr/bin/env bash
# Run JVM unit tests and print a compact pass/fail summary.
# Usage: tools/unit.sh [extra gradle args, e.g. --tests 'dev.flowclone.insert.*']
set -uo pipefail
cd "$(dirname "$0")/.."
./gradlew testDebugUnitTest -q "$@" 2>&1 | grep -E "^e: |FAILED|Test.*>|BUILD|What went wrong" -A2 | grep -vE "^\s+at " | head -60
status=${PIPESTATUS[0]}
python3 -I - <<'EOF'
import glob, xml.etree.ElementTree as ET
t = f = 0
for p in glob.glob("app/build/test-results/testDebugUnitTest/*.xml"):
    r = ET.parse(p).getroot()
    t += int(r.get("tests")); f += int(r.get("failures")) + int(r.get("errors"))
    for c in r.iter("testcase"):
        if c.find("failure") is not None or c.find("error") is not None:
            el = c.find("failure") if c.find("failure") is not None else c.find("error")
            print("FAIL", r.get("name").split(".")[-1], c.get("name"), "::", (el.get("message") or "")[:200])
print(f"unit tests: {t} run, {f} failed")
EOF
exit $status
