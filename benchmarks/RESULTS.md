# Benchmarks: claw 0.1.0

Real measurements on this machine, median of 5 runs. Claw keeps overflow checks on; the C versions have none.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw vs -O2 | Claw vs -O0 |
|---|---|---|---|---|---|
| fib | 0.147 s | 0.272 s | 0.684 s | 4.7x | 2.5x |
| primes | 0.187 s | 0.194 s | 0.954 s | 5.1x | 4.9x |
| collatz | 0.197 s | 1.086 s | 1.611 s | 8.2x | 1.5x |

Compiler speed: 40003 lines in 33.4 ms, about 1,197,751 lines/s (full build to a signed binary).
