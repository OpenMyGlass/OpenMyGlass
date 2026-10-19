#!/usr/bin/env bash
# Tests for tools/emulator/simulate-route.py (dry-run mode only; no emulator needed).
set -u
TOOL="$(cd "$(dirname "$0")/.." && pwd)/emulator/simulate-route.py"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
check() { if eval "$2"; then echo "PASS $1"; pass=$((pass+1)); else echo "FAIL $1"; fail=$((fail+1)); fi; }

# ~1112 m due north (0.01 degrees of latitude) at 36 km/h, 1 s interval = 10 m steps
printf '# test\n45.000000,9.000000\n45.010000,9.000000\n' > "$TMP/r.csv"
out="$(python3 -I "$TOOL" "$TMP/r.csv" --dry-run --speed-kmh 36 --interval 1)"
n="$(printf '%s\n' "$out" | wc -l | tr -d ' ')"
check "starts at the first point" '[ "$(printf "%s\n" "$out" | head -1)" = "45.000000,9.000000" ]'
check "ends at the last point" '[ "$(printf "%s\n" "$out" | tail -1)" = "45.010000,9.000000" ]'
check "about one point per 10 m" '[ "$n" -ge 110 ] && [ "$n" -le 115 ]'
printf '45,9\n' > "$TMP/one.csv"
check "rejects a one-point route" '! python3 -I "$TOOL" "$TMP/one.csv" --dry-run >/dev/null 2>&1'
echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
