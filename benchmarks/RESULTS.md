# Benchmarks: claw 0.11.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.294 s | 0.505 s | 0.554 s | 0.571 s | 1.9x | 1.9x |
| primes | 0.370 s | 0.372 s | 0.372 s | 0.380 s | 1.0x | 1.0x |
| collatz | 0.396 s | 2.103 s | 0.570 s | 0.576 s | 1.4x | 1.5x |
| sieve | 0.177 s | 0.280 s | 0.193 s | 0.196 s | 1.1x | 1.1x |
| mandel | 0.196 s | 0.630 s | 0.279 s | 0.279 s | 1.4x | 1.4x |
| list | 0.117 s | 0.114 s | 0.113 s | 0.112 s | 1.0x | 1.0x |
| vec | 0.046 s | 0.166 s | 0.067 s | 0.061 s | 1.5x | 1.3x |
| matmul | 0.241 s | 2.357 s | 0.693 s | 0.614 s | 2.9x | 2.5x |
| matmul_simd | 0.242 s | 2.336 s | 0.271 s | 0.270 s | 1.1x | 1.1x |

Compiler speed: 44004 lines in 265.0 ms, about 166,039 lines/s (full build to a signed binary).

Note on this run (0.11, 2026-11-13): the machine ran about twice as slow as for the 0.10 table above in absolute terms (load average near 3 from other apps; `clang -O2` itself took 0.294 s for `fib` against 0.150 s before), so compare the ratio columns, not the seconds. The ratios did not move (fib 1.9x, primes 1.0x, collatz 1.4x, matmul 2.9x, matmul_simd 1.1x), as expected: 0.11 adds no optimizer changes, and the compile speed figure (166,000 lines/s here, 320,000 before) halves for the same reason. Rerun on an idle machine before quoting absolute numbers.
