# Benchmarks: claw 0.6.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.141 s | 0.258 s | 0.289 s | 0.295 s | 2.1x | 2.1x |
| primes | 0.178 s | 0.197 s | 0.186 s | 0.184 s | 1.0x | 1.0x |
| collatz | 0.190 s | 1.009 s | 0.318 s | 0.306 s | 1.7x | 1.6x |
| sieve | 0.084 s | 0.138 s | 0.107 s | 0.109 s | 1.3x | 1.3x |
| mandel | 0.094 s | 0.303 s | 0.150 s | 0.150 s | 1.6x | 1.6x |
| list | 0.062 s | 0.056 s | 0.058 s | 0.057 s | 0.9x | 0.9x |
| vec | 0.018 s | 0.075 s | 0.028 s | 0.035 s | 1.5x | 1.9x |

Compiler speed: 44004 lines in 122.5 ms, about 359,229 lines/s (full build to a signed binary).
