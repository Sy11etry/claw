# Benchmarks: claw 0.13.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.306 s | 0.518 s | 0.557 s | 0.576 s | 1.8x | 1.9x |
| primes | 0.382 s | 0.383 s | 0.391 s | 0.376 s | 1.0x | 1.0x |
| collatz | 0.405 s | 2.151 s | 0.593 s | 0.596 s | 1.5x | 1.5x |
| sieve | 0.184 s | 0.298 s | 0.201 s | 0.201 s | 1.1x | 1.1x |
| mandel | 0.208 s | 0.647 s | 0.287 s | 0.288 s | 1.4x | 1.4x |
| list | 0.117 s | 0.119 s | 0.115 s | 0.112 s | 1.0x | 1.0x |
| vec | 0.052 s | 0.172 s | 0.066 s | 0.061 s | 1.3x | 1.2x |
| matmul | 0.265 s | 2.391 s | 0.736 s | 0.659 s | 2.8x | 2.5x |
| matmul_simd | 0.300 s | 2.505 s | 0.274 s | 0.272 s | 0.9x | 0.9x |

Compiler speed: 44004 lines in 303.1 ms, about 145,194 lines/s (full build to a signed binary).

Note on this run (0.13, 2026-11-14): the machine was not idle (load average about 4), so compare the ratio columns. They match 0.12. Compile speed is the same as in 0.12 within noise (the compiler's own work did not change). Printing floats, measured separately on 2 million `print(x)` calls (best of 3): 3.4 s in 0.11, 9.4 s in 0.12 (shortest-text search from one digit), 4.4 s in 0.13 (search starts at 15 digits, 6 for `f32`). The flight recorder costs nothing unless `CLAW_RECORD` is set: one `getenv` at the first `fs.read`, `os.env` or `os.args`. Not measured: the cost with recording on.

Note on 1.0 (2026-11-14): the compiler is the same as 0.13, so the numbers above stand; only the version number changed. Not re-measured.
