# Benchmarks: claw 0.10.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.149 s | 0.269 s | 0.287 s | 0.290 s | 1.9x | 2.0x |
| primes | 0.183 s | 0.195 s | 0.185 s | 0.184 s | 1.0x | 1.0x |
| collatz | 0.195 s | 1.079 s | 0.327 s | 0.286 s | 1.7x | 1.5x |
| sieve | 0.089 s | 0.143 s | 0.093 s | 0.094 s | 1.1x | 1.1x |
| mandel | 0.098 s | 0.311 s | 0.137 s | 0.135 s | 1.4x | 1.4x |
| list | 0.061 s | 0.061 s | 0.060 s | 0.058 s | 1.0x | 1.0x |
| vec | 0.020 s | 0.084 s | 0.029 s | 0.028 s | 1.4x | 1.4x |
| matmul | 0.127 s | 1.213 s | 0.340 s | 0.312 s | 2.7x | 2.5x |
| matmul_simd | 0.126 s | 1.215 s | 0.143 s | 0.158 s | 1.1x | 1.3x |

Compiler speed: 44004 lines in 135.1 ms, about 325,817 lines/s (full build to a signed binary).
