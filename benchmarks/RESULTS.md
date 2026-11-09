# Benchmarks: claw 0.8.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.141 s | 0.264 s | 0.295 s | 0.301 s | 2.1x | 2.1x |
| primes | 0.179 s | 0.186 s | 0.189 s | 0.185 s | 1.1x | 1.0x |
| collatz | 0.191 s | 1.019 s | 0.320 s | 0.309 s | 1.7x | 1.6x |
| sieve | 0.085 s | 0.139 s | 0.109 s | 0.108 s | 1.3x | 1.3x |
| mandel | 0.095 s | 0.305 s | 0.151 s | 0.150 s | 1.6x | 1.6x |
| list | 0.059 s | 0.064 s | 0.056 s | 0.057 s | 0.9x | 1.0x |
| vec | 0.018 s | 0.082 s | 0.029 s | 0.037 s | 1.6x | 2.1x |

Compiler speed: 44004 lines in 142.0 ms, about 309,805 lines/s (full build to a signed binary).
