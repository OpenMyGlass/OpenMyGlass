#!/usr/bin/env bash
# Tests for the clean-room guard: must reject tracked Google binaries/sources and accept a clean repo.
set -u
GUARD="$(cd "$(dirname "$0")/.." && pwd)/check-clean-room.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
expect() { # expect <name> <expected exit> <command...>
  local name="$1" want="$2"; shift 2
  out="$("$@" 2>&1)"; got=$?
  if [ "$got" = "$want" ]; then echo "PASS $name"; pass=$((pass+1)); else echo "FAIL $name (exit $got, expected $want)"; echo "$out" | sed 's/^/    /'; fail=$((fail+1)); fi
}
repo() { rm -rf "$TMP/r" && mkdir "$TMP/r" && git -C "$TMP/r" init -q; }

repo; echo "# ok" > "$TMP/r/README.md"; git -C "$TMP/r" add README.md
expect "clean repo accepted" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'PK' > "$TMP/r/fake.apk"; git -C "$TMP/r" add -f fake.apk
expect "apk rejected" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'x' > "$TMP/r/classes.dex"; git -C "$TMP/r" add -f classes.dex
expect "dex rejected" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'package com.google.glass.companion;\n' > "$TMP/r/Leak.java"; git -C "$TMP/r" add Leak.java
expect "Google java source rejected" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf '.class public Lcom/google/glass/Foo;\n' > "$TMP/r/Foo.smali"; git -C "$TMP/r" add Foo.smali
expect "Google smali rejected" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; mkdir "$TMP/r/_re"; echo x > "$TMP/r/_re/notes.txt"; git -C "$TMP/r" add -f _re/notes.txt
expect "tracked _re rejected" 1 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; printf 'package org.openmyglass.core;\n' > "$TMP/r/Ok.kt"; git -C "$TMP/r" add Ok.kt
expect "own source accepted" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

repo; mkdir -p "$TMP/r/gradle/wrapper"; printf 'PK' > "$TMP/r/gradle/wrapper/gradle-wrapper.jar"; git -C "$TMP/r" add -f gradle/wrapper/gradle-wrapper.jar
expect "gradle wrapper jar accepted" 0 bash -c "cd '$TMP/r' && bash '$GUARD'"

echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
