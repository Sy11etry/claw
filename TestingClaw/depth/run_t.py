#!/usr/bin/env python3
"""run_t.py SECONDS CMD...: runs CMD, kills it after SECONDS, prints the outcome."""
import subprocess, sys, time
secs = float(sys.argv[1])
start = time.time()
try:
    p = subprocess.run(sys.argv[2:], capture_output=True, text=True, timeout=secs)
    print(f"exit={p.returncode} time={time.time()-start:.2f}s stdout={p.stdout.strip()[:80]!r} stderr={p.stderr.strip()[:160]!r}")
except subprocess.TimeoutExpired as e:
    print(f"TIMEOUT after {secs}s (killed)")
