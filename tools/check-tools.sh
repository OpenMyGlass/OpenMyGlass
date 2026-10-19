#!/usr/bin/env bash
# Checks the tools required by phases 0–2.
set -u
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)"
missing=0
check() { if command -v "$1" >/dev/null 2>&1 || [ -x "$2" ]; then printf 'ok    %s\n' "$1"; else printf 'MISS  %s\n' "$1"; missing=1; fi; }
check jadx ""
check apktool ""
check protoc ""
check debugfs "$(brew --prefix e2fsprogs 2>/dev/null)/sbin/debugfs"
check simg2img ""
check 7z ""
check adb "$SDK/platform-tools/adb"
check apksigner "$BT/apksigner"
check emulator "$SDK/emulator/emulator"
exit $missing
