#!/usr/bin/env bash
# Test della guardia clean-room: deve rifiutare binari e sorgenti Google tracciati, e accettare una repo pulita.
set -u
GUARD="$(cd "$(dirname "$0")/.." && pwd)/check-clean-room.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
expect() { # expect <nome> <exit atteso> <comando...>
  local name="$1" want="$2"; shift 2
  out="$("$@" 2>&1)"; got=$?
  if [ "$got" = "$want" ]; then echo "PASS $name"; pass=$((pass+1)); else echo "FAIL $name (exit $got, atteso $want)"; echo "$out" | sed 's/^/    /'; fail=$((fail+1)); fi
}
repo() { rm -rf "$TMP/r" && mkdir "$TMP/r" && git -C "$TMP/r" init -q; }

repo; echo "# ok" > "$TMP/r/README.md"; git -C "$TMP/r" add README.md
expect "repo pulita accettata" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'PK' > "$TMP/r/fake.apk"; git -C "$TMP/r" add -f fake.apk
expect "apk rifiutato" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'x' > "$TMP/r/classes.dex"; git -C "$TMP/r" add -f classes.dex
expect "dex rifiutato" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'package com.google.glass.companion;\n' > "$TMP/r/Leak.java"; git -C "$TMP/r" add Leak.java
expect "sorgente java Google rifiutato" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf '.class public Lcom/google/glass/Foo;\n' > "$TMP/r/Foo.smali"; git -C "$TMP/r" add Foo.smali
expect "smali Google rifiutato" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; mkdir "$TMP/r/_re"; echo x > "$TMP/r/_re/notes.txt"; git -C "$TMP/r" add -f _re/notes.txt
expect "_re tracciata rifiutata" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'package org.openmyglass.core;\n' > "$TMP/r/Ok.kt"; git -C "$TMP/r" add Ok.kt
expect "sorgente proprio accettato" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; mkdir -p "$TMP/r/gradle/wrapper"; printf 'PK' > "$TMP/r/gradle/wrapper/gradle-wrapper.jar"; git -C "$TMP/r" add -f gradle/wrapper/gradle-wrapper.jar
expect "gradle wrapper jar accettato" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
