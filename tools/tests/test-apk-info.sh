#!/usr/bin/env bash
# Test di tools/re/apk-info.sh: rifiuta bundle split e file mancanti, stampa package/hash/certificato per un APK reale.
set -u
SCRIPT="$(cd "$(dirname "$0")/.." && pwd)/re/apk-info.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
SAMPLE="$(find "${ANDROID_HOME:-$HOME/Library/Android/sdk}/emulator" -name '*.apk' 2>/dev/null | head -1)"
pass=0; fail=0
ok() { echo "PASS $1"; pass=$((pass+1)); }
ko() { echo "FAIL $1"; echo "$2" | sed 's/^/    /'; fail=$((fail+1)); }

touch "$TMP/x.xapk"; out="$(bash "$SCRIPT" "$TMP/x.xapk" 2>&1)"; rc=$?
[ $rc -eq 2 ] && echo "$out" | grep -q 'bundle split' && ok "xapk rifiutato con exit 2" || ko "xapk rifiutato con exit 2" "rc=$rc $out"

touch "$TMP/x.apks"; out="$(bash "$SCRIPT" "$TMP/x.apks" 2>&1)"; rc=$?
[ $rc -eq 2 ] && ok "apks rifiutato con exit 2" || ko "apks rifiutato con exit 2" "rc=$rc $out"

out="$(bash "$SCRIPT" "$TMP/missing.apk" 2>&1)"; rc=$?
[ $rc -ne 0 ] && ok "file mancante -> exit != 0" || ko "file mancante -> exit != 0" "$out"

if [ -z "$SAMPLE" ]; then ko "APK di esempio disponibile" "nessun .apk nell'SDK"; else
  out="$(bash "$SCRIPT" "$SAMPLE" 2>&1)"; rc=$?
  [ $rc -eq 0 ] && echo "$out" | grep -q "^package: name='" && ok "stampa package" || ko "stampa package" "$out"
  echo "$out" | grep -qE '^file_sha256: [0-9a-f]{64}$' && ok "stampa sha256 file" || ko "stampa sha256 file" "$out"
  echo "$out" | grep -q 'certificate SHA-256 digest' && ok "stampa digest certificato" || ko "stampa digest certificato" "$out"
fi
echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
