#!/usr/bin/env bash
# Fallisce se i file tracciati (o in stage) contengono binari o codice Google.
set -u
fail=0
files="$(git ls-files --cached)"
bad_ext="$(printf '%s\n' "$files" | grep -Ei '\.(apk|apks|xapk|dex|odex|img|jar)$' | grep -v 'gradle/wrapper/gradle-wrapper.jar$' || true)"
if [ -n "$bad_ext" ]; then echo "Binari tracciati:"; echo "$bad_ext"; fail=1; fi
leaks=""
while IFS= read -r f; do
  [ -f "$f" ] || continue
  if grep -qE '^(package[[:space:]]+com\.google|\.class.*[[:space:]]Lcom/google/)' "$f"; then leaks="$leaks$f"$'\n'; fi
done < <(printf '%s\n' "$files" | grep -E '\.(java|smali|kt)$' || true)
if [ -n "$leaks" ]; then echo "Sorgenti con package Google:"; printf '%s' "$leaks"; fail=1; fi
if printf '%s\n' "$files" | grep -q '^_re/'; then echo "_re/ è tracciata"; fail=1; fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
