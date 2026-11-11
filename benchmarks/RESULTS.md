# Benchmarks: claw 0.9.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.147 s | 0.269 s | 0.305 s | 0.310 s | 2.1x | 2.1x |
| primes | 0.182 s | 0.189 s | 0.196 s | 0.195 s | 1.1x | 1.1x |
| collatz | 0.206 s | 1.078 s | 0.324 s | 0.331 s | 1.6x | 1.6x |
| sieve | 0.089 s | 0.144 s | 0.130 s | 0.115 s | 1.5x | 1.3x |
| mandel | 0.096 s | 0.312 s | 0.136 s | 0.137 s | 1.4x | 1.4x |
| list | 0.058 s | 0.057 s | 0.061 s | 0.058 s | 1.0x | 1.0x |
| vec | 0.019 s | 0.081 s | 0.030 s | 0.037 s | 1.6x | 1.9x |

Compiler speed: 44004 lines in 132.6 ms, about 331,763 lines/s (full build to a signed binary).
