# Benchmarks: claw 0.3.0

Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |
|---|---|---|---|---|---|
| fib | 0.153 s | 0.276 s | 0.712 s | 4.6x | 2.6x |
| primes | 0.195 s | 0.200 s | 0.970 s | 5.0x | 4.8x |
| collatz | 0.207 s | 1.105 s | 1.636 s | 7.9x | 1.5x |
| sieve | 0.094 s | 0.148 s | 0.381 s | 4.1x | 2.6x |
| mandel | 0.113 s | 0.354 s | 0.840 s | 7.4x | 2.4x |
| list | 0.062 s | 0.058 s | 0.066 s | 1.1x | 1.1x |

Compiler speed: 40003 lines in 42.5 ms, about 941,661 lines/s (full build to a signed binary).
