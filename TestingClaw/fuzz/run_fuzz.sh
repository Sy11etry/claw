#!/bin/bash
# 20 seeds x 100 programs for each fuzzer. Full output of every run is kept in this folder.
cd "$(dirname "$0")/../.." || exit 1
for f in fuzz_opt fuzz_x64 fuzz_simd; do
  for k in $(seq 0 19); do
    seed=$((1000 + k * 100))
    out="TestingClaw/fuzz/${f}_${seed}.txt"
    python3 "tools/$f.py" 100 "$seed" > "$out" 2>&1
    echo "$f seed=$seed exit=$? : $(tail -1 "$out")"
  done
done
echo "fuzzing finished"
