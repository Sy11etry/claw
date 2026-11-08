# Benchmarks: claw 0.5.0

Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |
|---|---|---|---|---|---|
| fib | 0.146 s | 0.265 s | 0.689 s | 4.7x | 2.6x |
| primes | 0.179 s | 0.194 s | 0.931 s | 5.2x | 4.8x |
| collatz | 0.193 s | 1.051 s | 1.572 s | 8.1x | 1.5x |
| sieve | 0.088 s | 0.141 s | 0.367 s | 4.2x | 2.6x |
| mandel | 0.095 s | 0.305 s | 0.783 s | 8.3x | 2.6x |
| list | 0.062 s | 0.062 s | 0.066 s | 1.1x | 1.1x |
| vec | 0.018 s | 0.080 s | 0.200 s | 10.9x | 2.5x |

Compiler speed: 40003 lines in 41.7 ms, about 960,193 lines/s (full build to a signed binary).
