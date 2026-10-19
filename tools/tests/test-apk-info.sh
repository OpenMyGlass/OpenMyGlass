#!/usr/bin/env bash
# Tests for tools/re/apk-info.sh: rejects split bundles and missing files, prints package/hash/certificate for a real APK.
set -u
SCRIPT="$(cd "$(dirname "$0")/.." && pwd)/re/apk-info.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
SAMPLE="$(find "${ANDROID_HOME:-$HOME/Library/Android/sdk}/emulator" -name '*.apk' 2>/dev/null | head -1)"
pass=0; fail=0
ok() { echo "PASS $1"; pass=$((pass+1)); }
ko() { echo "FAIL $1"; echo "$2" | sed 's/^/    /'; fail=$((fail+1)); }

touch "$TMP/x.xapk"; out="$(bash "$SCRIPT" "$TMP/x.xapk" 2>&1)"; rc=$?
[ $rc -eq 2 ] && echo "$out" | grep -q 'split bundle' && ok "xapk rejected with exit 2" || ko "xapk rejected with exit 2" "rc=$rc $out"

touch "$TMP/x.apks"; out="$(bash "$SCRIPT" "$TMP/x.apks" 2>&1)"; rc=$?
[ $rc -eq 2 ] && ok "apks rejected with exit 2" || ko "apks rejected with exit 2" "rc=$rc $out"

out="$(bash "$SCRIPT" "$TMP/missing.apk" 2>&1)"; rc=$?
[ $rc -ne 0 ] && ok "missing file -> exit != 0" || ko "missing file -> exit != 0" "$out"

if [ -z "$SAMPLE" ]; then ko "sample APK available" "no .apk in the SDK"; else
  out="$(bash "$SCRIPT" "$SAMPLE" 2>&1)"; rc=$?
  [ $rc -eq 0 ] && echo "$out" | grep -q "^package: name='" && ok "prints package" || ko "prints package" "$out"
  echo "$out" | grep -qE '^file_sha256: [0-9a-f]{64}$' && ok "prints file sha256" || ko "prints file sha256" "$out"
  echo "$out" | grep -q 'certificate SHA-256 digest' && ok "prints certificate digest" || ko "prints certificate digest" "$out"
fi
echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
