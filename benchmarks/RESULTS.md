# Benchmarks: claw 0.12.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.296 s | 0.527 s | 0.566 s | 0.583 s | 1.9x | 2.0x |
| primes | 0.379 s | 0.394 s | 0.376 s | 0.367 s | 1.0x | 1.0x |
| collatz | 0.429 s | 2.147 s | 0.586 s | 0.595 s | 1.4x | 1.4x |
| sieve | 0.183 s | 0.284 s | 0.194 s | 0.195 s | 1.1x | 1.1x |
| mandel | 0.201 s | 0.641 s | 0.284 s | 0.304 s | 1.4x | 1.5x |
| list | 0.122 s | 0.117 s | 0.115 s | 0.116 s | 0.9x | 1.0x |
| vec | 0.047 s | 0.158 s | 0.066 s | 0.063 s | 1.4x | 1.3x |
| matmul | 0.263 s | 2.400 s | 0.706 s | 0.616 s | 2.7x | 2.3x |
| matmul_simd | 0.253 s | 2.404 s | 0.287 s | 0.278 s | 1.1x | 1.1x |

Compiler speed: 44004 lines in 293.7 ms, about 149,829 lines/s (full build to a signed binary).

Note on this run (0.12, 2026-11-13): the machine was not idle (load average 3-4 from other apps), so compare the ratio columns, not the seconds. They match 0.11 (fib 1.9x, primes 1.0x, collatz 1.4x, sieve 1.1x, mandel 1.4x, matmul 2.7x, matmul_simd 1.1x), as expected: 0.12 changes no optimization. The compile-speed figure (150,000 lines/s) is lower than 0.10's 320,000 for the same load reason; a back-to-back run of `claw-0.11` and 0.12 on the same input (`selfhost/main.claw`, best of 5) took 0.551 s and 0.560 s, so 0.12 is not slower to compile. New in 0.12 and not in this table: printing a float now tries up to 17 `snprintf` calls to find the shortest exact text, so a loop that prints millions of floats is slower than in 0.11; arithmetic is unaffected. Not measured here. Rerun on an idle machine before quoting absolute numbers.
