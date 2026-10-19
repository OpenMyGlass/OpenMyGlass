#!/usr/bin/env bash
# Usage: apk-info.sh <file.apk> — prints package, version, file hash and signing certificate.
set -euo pipefail
apk="${1:?usage: apk-info.sh <file.apk>}"
case "$apk" in *.apks|*.xapk) echo "ERROR: split bundle ($apk). Extract base.apk with 7z and run again." >&2; exit 2;; esac
[ -f "$apk" ] || { echo "ERROR: file not found: $apk" >&2; exit 1; }
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)"
"$BT/aapt" dump badging "$apk" | grep -E "^package:"
echo "file_sha256: $(shasum -a 256 "$apk" | cut -d' ' -f1)"
"$BT/apksigner" verify --print-certs "$apk" | grep -E '^(Signer #[0-9]+|V[0-9.]+ Signer): certificate (DN|SHA-256 digest)'
