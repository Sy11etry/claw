#!/bin/bash
# Builds each thread program for arm64 and x86-64, takes the first run as the reference,
# then reruns it many times. Only `claw build` binaries run tasks on threads; `claw run` does not.
cd "$(dirname "$0")" || exit 1
mkdir -p out
RUNS=${RUNS:-200}
for p in c02_stress c03_interp_tasks c12_many_tasks c14_task_alloc c01_sizes; do
  n=$RUNS
  [ "$p" = c01_sizes ] && n=50
  claw build "$p.claw" -o "out/$p" >/dev/null 2>&1 || { echo "BUILD FAIL arm64 $p"; continue; }
  claw build --target x86_64 "$p.claw" -o "out/${p}_x64" >/dev/null 2>&1 || { echo "BUILD FAIL x86-64 $p"; continue; }
  "./out/$p" > "out/$p.ref" 2>&1; echo "reference arm64 $p: exit $?"
  "./out/${p}_x64" > "out/${p}_x64.ref" 2>&1; echo "reference x86-64 $p: exit $?"
  cmp -s "out/$p.ref" "out/${p}_x64.ref" || echo "ARCH DIFF in $p (first run)"
  for target in "" "_x64"; do
    bad=0
    for i in $(seq "$n"); do
      "./out/$p$target" > out/run.txt 2>&1 || bad=$((bad + 1))
      cmp -s out/run.txt "out/$p.ref" || bad=$((bad + 1))
    done
    echo "${target:-_arm64}"" $p: $n runs, $bad bad"
  done
done
echo "stress finished"
