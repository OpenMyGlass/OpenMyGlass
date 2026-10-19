#!/usr/bin/env bash
# Uso: apk-info.sh <file.apk> — stampa package, versione, hash del file e del certificato.
set -euo pipefail
apk="${1:?uso: apk-info.sh <file.apk>}"
case "$apk" in *.apks|*.xapk) echo "ERRORE: bundle split ($apk). Estrarre base.apk con 7z e rieseguire." >&2; exit 2;; esac
[ -f "$apk" ] || { echo "ERRORE: file non trovato: $apk" >&2; exit 1; }
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)"
"$BT/aapt" dump badging "$apk" | grep -E "^package:"
echo "file_sha256: $(shasum -a 256 "$apk" | cut -d' ' -f1)"
"$BT/apksigner" verify --print-certs "$apk" | grep -E '^(Signer #[0-9]+|V[0-9.]+ Signer): certificate (DN|SHA-256 digest)'
