#!/usr/bin/env bash
# Tests for tools/re/nano2proto.py: compares output on fixtures with the expected .proto and checks protoc compiles it.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"; FIX="$HERE/fixtures/nano"; TOOL="$HERE/../re/nano2proto.py"
OUT="$(mktemp -d)"; trap 'rm -rf "$OUT"' EXIT
pass=0; fail=0
python3 -I "$TOOL" --out "$OUT" "$FIX/Demo.java" "$FIX/Other.java" > "$OUT/run.log" 2>&1; rc=$?
if [ $rc -eq 0 ]; then echo "PASS run"; pass=$((pass+1)); else echo "FAIL run (rc=$rc)"; sed 's/^/    /' "$OUT/run.log"; fail=$((fail+1)); fi
if diff -u "$FIX/Demo.expected.proto" "$OUT/Demo.proto" > "$OUT/diff.txt" 2>&1; then echo "PASS expected output"; pass=$((pass+1)); else echo "FAIL expected output"; sed 's/^/    /' "$OUT/diff.txt"; fail=$((fail+1)); fi
if protoc -I "$OUT" --descriptor_set_out=/dev/null "$OUT/Demo.proto" "$OUT/Other.proto" > "$OUT/protoc.txt" 2>&1; then echo "PASS protoc compiles"; pass=$((pass+1)); else echo "FAIL protoc compiles"; sed 's/^/    /' "$OUT/protoc.txt"; fail=$((fail+1)); fi
python3 -I "$TOOL" --out "$OUT/m" "$FIX/Holder.java" "$FIX/Leaf.java" > "$OUT/m.log" 2>&1
for n in Leaf Holder; do
  if diff -u "$FIX/$n.expected.proto" "$OUT/m/$n.proto" > "$OUT/d.txt" 2>&1; then echo "PASS message-file $n"; pass=$((pass+1)); else echo "FAIL message-file $n"; sed 's/^/    /' "$OUT/d.txt" "$OUT/m.log"; fail=$((fail+1)); fi
done
if protoc -I "$OUT/m" --descriptor_set_out=/dev/null "$OUT/m/Holder.proto" "$OUT/m/Leaf.proto" > "$OUT/p2.txt" 2>&1; then echo "PASS protoc message-file"; pass=$((pass+1)); else echo "FAIL protoc message-file"; sed 's/^/    /' "$OUT/p2.txt"; fail=$((fail+1)); fi
echo "$pass passed, $fail failed"; [ $fail -eq 0 ]
