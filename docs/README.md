# Documentation

| File | What it is |
|---|---|
| [`CARD.md`](CARD.md) | The whole language on one page, at most 2500 tokens. `claw card` prints it. Start here. |
| [`SPEC.md`](SPEC.md) | The exact rules: lexical rules, semantics, ownership, calling C, threads, SIMD, every error code. |
| [`STATUS.md`](STATUS.md) | What works today, what works partly, what does not exist yet, and the known limits. |
| [`DESIGN.md`](DESIGN.md) | Why the language is shaped this way: the choices, their reasons, and what was deliberately not done. |

Where the numbers are: speed against `clang` in [`../benchmarks/RESULTS.md`](../benchmarks/RESULTS.md), tokens in
[`../benchmarks/TOKENS.md`](../benchmarks/TOKENS.md), the exam for a weak model in [`../benchmarks/EXAM.md`](../benchmarks/EXAM.md).
The notes for people who work on the compiler itself are in [`../selfhost/PORTING.md`](../selfhost/PORTING.md).
