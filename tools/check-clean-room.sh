#!/usr/bin/env bash
# Fails if tracked (or staged) files contain Google binaries or Google source code.
# Also fails if a tracked file has the SHA-256 of a binary in the XE24 factory image
# (tools/factory-blobs.sha256, override with FACTORY_HASHES=<file>).
set -u
HASHES="${FACTORY_HASHES:-$(cd "$(dirname "$0")" && pwd)/factory-blobs.sha256}"
cd "$(git rev-parse --show-toplevel)" || exit 1
fail=0
files="$(git -c core.quotePath=false ls-files --cached)"
bad_ext="$(printf '%s\n' "$files" | grep -Ei '\.(apk|apks|xapk|dex|odex|oat|vdex|img|jar|aar|class|zip|tgz|so)$' | grep -v 'gradle/wrapper/gradle-wrapper.jar$' || true)"
if [ -n "$bad_ext" ]; then echo "Tracked binaries:"; echo "$bad_ext"; fail=1; fi
leaks=""
while IFS= read -r f; do
  [ -f "$f" ] || continue
  if grep -qE '^(package[[:space:]]+com\.google|\.class.*[[:space:]]Lcom/google/)' -- "$f"; then
    # Exception: OpenMyGlass's own GDK compile-only stubs (written from the public docs, see glass-xe/README.md)
    case "$f" in
      glass-xe/gdk-stubs/*) grep -q 'Compile-only stub, written by OpenMyGlass' -- "$f" && continue ;;
    esac
    leaks="$leaks$f"$'\n'
  fi
done < <(printf '%s\n' "$files" | grep -E '\.(java|smali|kt)$' || true)
if [ -n "$leaks" ]; then echo "Sources with a Google package:"; printf '%s' "$leaks"; fail=1; fi
if printf '%s\n' "$files" | grep -q '^_re/'; then echo "_re/ is tracked"; fail=1; fi
dumps="$(printf '%s\n' "$files" | grep -Ei '\.(hprof|core|dmp)$' || true)"
if [ -n "$dumps" ]; then echo "Memory/crash dumps tracked:"; echo "$dumps"; fail=1; fi
# Anything over 50 MB is never source (GitHub rejects > 100 MB): binaries, images, dumps
big=""
while IFS= read -r f; do
  [ -f "$f" ] || continue
  size=$(wc -c < "$f" | tr -d ' ')
  [ "$size" -gt 52428800 ] && big="$big$f ($size bytes)"$'\n'
done < <(printf '%s\n' "$files")
if [ -n "$big" ]; then echo "Files over 50 MB:"; printf '%s' "$big"; fail=1; fi
# Same bytes as a factory-image binary, whatever the file is called
if [ -f "$HASHES" ]; then
  if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$@"; }; else sha() { shasum -a 256 "$@"; }; fi
  known="$(grep -Eo '^[0-9a-f]{64}' "$HASHES" | sort -u)"
  hits=""
  while IFS= read -r f; do
    [ -f "$f" ] && [ ! -L "$f" ] || continue
    h="$(sha "$f" | cut -d' ' -f1)"
    printf '%s\n' "$known" | grep -qx "$h" && hits="$hits$f ($h)"$'\n'
  done < <(printf '%s\n' "$files")
  if [ -n "$hits" ]; then echo "Files identical to a factory-image binary:"; printf '%s' "$hits"; fail=1; fi
else
  echo "factory hash list not found: $HASHES"; fail=1
fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
