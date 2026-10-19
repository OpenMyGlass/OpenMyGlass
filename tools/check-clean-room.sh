#!/usr/bin/env bash
# Fails if tracked (or staged) files contain Google binaries or Google source code.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1
fail=0
files="$(git -c core.quotePath=false ls-files --cached)"
bad_ext="$(printf '%s\n' "$files" | grep -Ei '\.(apk|apks|xapk|dex|odex|oat|vdex|img|jar|aar|class|zip|tgz)$' | grep -v 'gradle/wrapper/gradle-wrapper.jar$' || true)"
if [ -n "$bad_ext" ]; then echo "Tracked binaries:"; echo "$bad_ext"; fail=1; fi
leaks=""
while IFS= read -r f; do
  [ -f "$f" ] || continue
  if grep -qE '^(package[[:space:]]+com\.google|\.class.*[[:space:]]Lcom/google/)' -- "$f"; then leaks="$leaks$f"$'\n'; fi
done < <(printf '%s\n' "$files" | grep -E '\.(java|smali|kt)$' || true)
if [ -n "$leaks" ]; then echo "Sources with a Google package:"; printf '%s' "$leaks"; fail=1; fi
if printf '%s\n' "$files" | grep -q '^_re/'; then echo "_re/ is tracked"; fail=1; fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
