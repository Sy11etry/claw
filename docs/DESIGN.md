# Design notes

Why Claw is shaped the way it is. The rules themselves are in [`SPEC.md`](SPEC.md), the whole language is
[`CARD.md`](CARD.md), and what works today is in [`STATUS.md`](STATUS.md). A choice that someone could reasonably have
made the other way belongs here, with its reason.

## The question

Almost every new systems language asks "how do we make C better". Claw asks something else: **what systems language do you
need when an AI writes the code and a human answers for it?** A model does not get tired of typing, but it does not know a
new language (it was not in its training data), it pays for every token it writes, reads and fixes, it is confidently wrong,
and it writes weak tests. Every choice below follows from one of those four facts.

## Choices and their reasons

**The whole language fits on one card.** `CARD.md` is at most 2500 tokens, so any model, even a small one, can be given the
entire reference in its prompt. It was 2000 tokens until 0.13; the extra 500 bought SIMD, C structs by value, slices and
the flight recorder, and each of those was judged worth its place. A feature that cannot pay for its lines on the card waits.

**Two gears, one language.** `fn` is safe: no pointers, no `free`, memory is released by the compiler at the end of the scope.
`claw fn` is plain C. There is one foundation, manual memory with `defer`; the safe gear is that foundation with the
compiler writing the frees. There is no runtime and no garbage collector, and the standard library is written in Claw.
`claw audit` lists every place where the compiler takes your word.

**Ownership without a borrow checker.** In safe code a reference can be neither stored nor returned: a parameter is read-only
for the duration of a call, `&T` may be changed during a call, `own T` is taken for good. Because of that there are no dangling
references to track, trees and lists work by ownership, graphs by indices, and there is no aliasing at all (the equivalent of
`restrict` for free). The price is a smaller language than Rust's; the gain is a check that fits in a few hundred lines.

**Moves are explicit and copies are visible.** Only a whole local variable can move, and the compiler's patch for a second
use is `copy(x)`. Expensive copies are never hidden.

**Contracts are the one mechanism for checks.** `need` and `sure` are documentation, run-time check, test generator and optimizer
hint at once. The compiler proves what it can and removes the check; a contract with a function call cannot be checked in
`claw build`, and the compiler says how many it skipped instead of staying silent.

**Overflow traps for signed numbers and wraps for unsigned.** A silent wrap of `int` hides bugs; a hash or a checksum needs
the wrap, so `u8..u64` wrap as in C. `int` is always 64 bits and `len` returns a signed value, which removes a class of
`size_t - 1` mistakes.

**Errors are values.** `T ! E` and `?T`, with `fail`, `?` and `else`. Ignoring an error is a compile error. There are no
exceptions and no null.

**No methods, closures, operator overloading or macros.** A function is found by the type of its first argument, so `push(&a, x)`
and not `a.push(x)`. This is less to learn, fewer tokens, and nothing for a model to confuse with another language.

**Syntax is plain, meaning is fresh.** `fn`, `if`, `for` are known to every model already. Only what earns it is new:
`need`, `sure`, `fail`, `+mem`, `claw fn`. Keywords are ordinary short English words that are one token each. A statement
ends at the end of the line, so there is no `;`.

**Effects in the signature.** `+mem` allocates, `+io` touches the world; no plus is a guarantee. A reader, or a model, does not
have to read the body to know that. Public functions must declare them; private ones are worked out by the compiler.

**The compiler writes the tests.** `claw test` fuzzes every function without effects: boundary values, random inputs, inputs
picked by code coverage, with the failure shrunk to a small example. That is only possible because we own the back end and
the interpreter, and it spares a weak model from having to write a good test.

**`claw why` explains the cause, not the address.** The program is run again in an interpreter that remembers where each
value came from, and only the lines that influenced the failure are shown. The interpreter must agree with machine code;
the test suite compares them.

**Own back end, no LLVM.** The compiler encodes ARM64 (and x86-64) instructions itself, writes the Mach-O or ELF file and signs
it with its own SHA-256, because macOS on ARM64 will not run an unsigned binary. This keeps the whole path from source to
binary in code we can read, and it keeps compilation fast (about 320,000 lines per second). The price is speed against
`clang -O2` (see [`../benchmarks/RESULTS.md`](../benchmarks/RESULTS.md)).

**The compiler is also written in Claw.** `selfhost/` compiles itself to a fixed point, and its output must match the Rust
compiler's byte for byte (`claw build --no-opt`). It is a subset on purpose: the optimizer, `why`, `test`, `fix` and `fmt`
stay in Rust.

## Things we chose not to do

**No optimization pass for `fib`.** `clang` is faster on `fib(40)` because it rewrites the second recursive call into a loop
with an accumulator, which reorders additions. With trapping signed overflow that would change where a program fails, so
we do not. The gap on `matmul` is automatic vectorization, which Claw does not have; hand-written SIMD in `claw fn` closes it.

**The flight recorder is opt-in.** `CLAW_RECORD=crash.clawtrace ./app` records the inputs from outside (arguments, the
environment variables asked for, the files read whole) and writes them only when the program ends in a trap. A runtime that
silently writes files next to your program would be an effect missing from every signature. It records no crashes by
signal, and it does not embed the source in the binary: `claw why` compares the text of the failure instead of a hash.
Measured cost, once, on one machine: about 0.5% on a program that reads 15,000 files in a loop.

**No proof-carrying binaries and no model swarm.** Proving "this program never traps" needs a constraint solver of its own
and is a project by itself. Letting external models rewrite code and hot-swap it needs models that are not in the
compiler's environment. Both were considered and deferred.

**Linux x86-64, Windows and WebAssembly are not built.** macOS on Apple Silicon is the main target; x86-64 (under Rosetta)
and ARM64 Linux are secondary, and `STATUS.md` says exactly how far each was checked.

## How a release is checked

Every iteration ends with the full test suite (`tools/runtests.py`), measurements against `clang`, and an update of
`STATUS.md`. A new compiler rule has a stable error code `E####`, a test in `tests/err`, and a patch for `claw fix`
where one is safe. Constructs that are reserved but not built give `E0900`, never a strange parse error.
An outside check was done once: a second model wrote about 290 programs against 0.11 and recorded 45 findings, all of which
were closed in 0.12 except those that need a new target.
