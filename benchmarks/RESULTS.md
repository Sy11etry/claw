# Benchmarks: claw 0.10.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.150 s | 0.272 s | 0.289 s | 0.297 s | 1.9x | 2.0x |
| primes | 0.189 s | 0.200 s | 0.189 s | 0.191 s | 1.0x | 1.0x |
| collatz | 0.198 s | 1.086 s | 0.286 s | 0.284 s | 1.4x | 1.4x |
| sieve | 0.091 s | 0.145 s | 0.098 s | 0.099 s | 1.1x | 1.1x |
| mandel | 0.103 s | 0.319 s | 0.139 s | 0.138 s | 1.4x | 1.3x |
| list | 0.061 s | 0.059 s | 0.064 s | 0.063 s | 1.0x | 1.0x |
| vec | 0.029 s | 0.083 s | 0.037 s | 0.037 s | 1.3x | 1.3x |
| matmul | 0.129 s | 1.224 s | 0.366 s | 0.320 s | 2.8x | 2.5x |
| matmul_simd | 0.130 s | 1.224 s | 0.144 s | 0.144 s | 1.1x | 1.1x |

Compiler speed: 44004 lines in 137.3 ms, about 320,596 lines/s (full build to a signed binary).
