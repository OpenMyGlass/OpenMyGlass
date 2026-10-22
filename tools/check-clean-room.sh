#!/usr/bin/env bash
# Fails if tracked (or staged) files contain Google binaries or Google source code.
# Also fails if a tracked file has the SHA-256 of a binary in the XE24 factory image
# (tools/factory-blobs.sha256, override with FACTORY_HASHES=<file>).
set -u
HASHES="${FACTORY_HASHES:-$(cd "$(dirname "$0")" && pwd)/factory-blobs.sha256}"
case "$HASHES" in /*) ;; *) HASHES="$PWD/$HASHES" ;; esac
cd "$(git rev-parse --show-toplevel)" || exit 1
fail=0
# NUL-separated, so names are never C-quoted; names with control characters are refused outright
files=""; odd=""
while IFS= read -r -d '' f; do
  case "$f" in *[[:cntrl:]]*) odd="$odd$(printf '%q' "$f")"$'\n'; continue ;; esac
  files="$files$f"$'\n'
done < <(git ls-files -z --cached)
files="${files%$'\n'}"
if [ -n "$odd" ]; then echo "Tracked names with control characters:"; printf '%s' "$odd"; fail=1; fi
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
# Same bytes as a factory-image binary, whatever the file is called (hashes the staged blobs)
known="$( { grep -Eo '^[0-9a-f]{64}' "$HASHES" || true; } 2>/dev/null | sort -u)"
if [ -z "$known" ]; then
  echo "factory hash list missing or without hashes: $HASHES"; fail=1
else
  if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum; }; else sha() { shasum -a 256; }; fi
  hits=""; errs=""
  while IFS= read -r -d '' entry; do
    meta="${entry%%$'\t'*}"; f="${entry#*$'\t'}"
    mode="${meta%% *}"; oid="$(printf '%s' "$meta" | cut -d' ' -f2)"
    [ "$mode" = 160000 ] && continue  # submodule, no blob
    h="$(git cat-file blob "$oid" | sha | cut -d' ' -f1)"
    if ! printf '%s' "$h" | grep -Eqx '[0-9a-f]{64}'; then errs="$errs$f"$'\n'; continue; fi
    printf '%s\n' "$known" | grep -qx "$h" && hits="$hits$f ($h)"$'\n'
  done < <(git ls-files -s -z --cached)
  if [ -n "$errs" ]; then echo "Could not hash:"; printf '%s' "$errs"; fail=1; fi
  if [ -n "$hits" ]; then echo "Files identical to a factory-image binary:"; printf '%s' "$hits"; fail=1; fi
fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
