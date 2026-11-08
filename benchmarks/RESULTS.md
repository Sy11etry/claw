# Benchmarks: claw 0.4.0

Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |
|---|---|---|---|---|---|
| fib | 0.146 s | 0.270 s | 0.694 s | 4.7x | 2.6x |
| primes | 0.181 s | 0.194 s | 0.932 s | 5.1x | 4.8x |
| collatz | 0.191 s | 1.040 s | 1.580 s | 8.3x | 1.5x |
| sieve | 0.085 s | 0.141 s | 0.362 s | 4.2x | 2.6x |
| mandel | 0.095 s | 0.307 s | 0.782 s | 8.2x | 2.5x |
| list | 0.063 s | 0.060 s | 0.064 s | 1.0x | 1.1x |
| vec | 0.020 s | 0.074 s | 0.200 s | 10.0x | 2.7x |

Compiler speed: 40003 lines in 41.3 ms, about 968,293 lines/s (full build to a signed binary).
