# Benchmarks: claw 0.9.0

Real measurements on this machine, median of 5 runs. `claw build` keeps every check it cannot prove unnecessary; the C versions have none. `--fast` removes them all.

| Benchmark | clang -O2 | clang -O0 | Claw | Claw `--fast` | Claw vs -O2 | `--fast` vs -O2 |
|---|---|---|---|---|---|---|
| fib | 0.141 s | 0.261 s | 0.293 s | 0.298 s | 2.1x | 2.1x |
| primes | 0.180 s | 0.191 s | 0.191 s | 0.188 s | 1.1x | 1.0x |
| collatz | 0.191 s | 1.023 s | 0.314 s | 0.311 s | 1.6x | 1.6x |
| sieve | 0.086 s | 0.138 s | 0.111 s | 0.110 s | 1.3x | 1.3x |
| mandel | 0.096 s | 0.307 s | 0.152 s | 0.151 s | 1.6x | 1.6x |
| list | 0.059 s | 0.057 s | 0.056 s | 0.057 s | 1.0x | 1.0x |
| vec | 0.019 s | 0.083 s | 0.030 s | 0.036 s | 1.6x | 1.9x |

Compiler speed: 44004 lines in 127.0 ms, about 346,370 lines/s (full build to a signed binary).

## The compiler written in Claw

Building `benchmarks/out/big.claw` (44,004 lines), median of 5. All four produce working programs; the two unoptimized ones produce identical bytes.

| Compiler | Time | Lines/s |
|---|---|---|
| `claw` (Rust), with optimizer | 131.8 ms | 333,976 |
| `claw` (Rust), `--no-opt` | 64.1 ms | 686,209 |
| `clawc` built by Rust with optimizer | 104.9 ms | 419,330 |
| `clawc` built by itself (no optimizer) | 554.1 ms | 79,422 |

`clawc` building itself (about 14,000 lines with std): 317 ms; `claw --no-opt` on the same: 45 ms.
