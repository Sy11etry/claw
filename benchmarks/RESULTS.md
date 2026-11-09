# Benchmarks: claw 0.7.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.152 s | 0.274 s | 0.313 s | 0.318 s | 2.1x | 2.1x |
| primes | 0.191 s | 0.195 s | 0.202 s | 0.197 s | 1.1x | 1.0x |
| collatz | 0.201 s | 1.094 s | 0.330 s | 0.336 s | 1.6x | 1.7x |
| sieve | 0.092 s | 0.145 s | 0.115 s | 0.116 s | 1.2x | 1.3x |
| mandel | 0.100 s | 0.321 s | 0.159 s | 0.158 s | 1.6x | 1.6x |
| list | 0.064 s | 0.059 s | 0.058 s | 0.059 s | 0.9x | 0.9x |
| vec | 0.027 s | 0.082 s | 0.038 s | 0.046 s | 1.4x | 1.7x |

Compiler speed: 44004 lines in 130.1 ms, about 338,355 lines/s (full build to a signed binary).
