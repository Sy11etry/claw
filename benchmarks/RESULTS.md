# Benchmarks: claw 0.2.0

Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |
|---|---|---|---|---|---|
| fib | 0.152 s | 0.274 s | 0.709 s | 4.7x | 2.6x |
| primes | 0.196 s | 0.194 s | 0.975 s | 5.0x | 5.0x |
| collatz | 0.208 s | 1.113 s | 1.682 s | 8.1x | 1.5x |
| sieve | 0.093 s | 0.147 s | 0.386 s | 4.2x | 2.6x |
| mandel | 0.103 s | 0.329 s | 0.847 s | 8.2x | 2.6x |

Compiler speed: 40003 lines in 39.5 ms, about 1,012,768 lines/s (full build to a signed binary).
