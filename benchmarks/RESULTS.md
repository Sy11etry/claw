# Benchmarks: claw 0.10.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.143 s | 0.269 s | 0.270 s | 0.276 s | 1.9x | 1.9x |
| primes | 0.179 s | 0.188 s | 0.180 s | 0.183 s | 1.0x | 1.0x |
| collatz | 0.192 s | 1.021 s | 0.277 s | 0.281 s | 1.4x | 1.5x |
| sieve | 0.084 s | 0.137 s | 0.093 s | 0.091 s | 1.1x | 1.1x |
| mandel | 0.095 s | 0.306 s | 0.135 s | 0.135 s | 1.4x | 1.4x |
| list | 0.062 s | 0.059 s | 0.059 s | 0.056 s | 0.9x | 0.9x |
| vec | 0.018 s | 0.073 s | 0.028 s | 0.027 s | 1.5x | 1.5x |
| matmul | 0.123 s | 1.168 s | 0.363 s | 0.308 s | 3.0x | 2.5x |
| matmul_simd | 0.124 s | 1.183 s | 0.140 s | 0.141 s | 1.1x | 1.1x |

Compiler speed: 44004 lines in 130.8 ms, about 336,334 lines/s (full build to a signed binary).
