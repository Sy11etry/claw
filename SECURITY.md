# Security

Claw is a compiler, so the serious bugs are the ones where it quietly builds the wrong program: a safety check that
is removed when it should not be, a memory-safe `fn` that can corrupt memory, an ownership rule that lets a dangling
reference through, or a compiler crash on hostile input.

If you find one, please report it **privately** through GitHub: *Security* tab, *Report a vulnerability*. Do not open a
public issue first. Include the smallest program that shows it and the compiler command line.

What to expect: an answer within a week, and a fix or a written reason why it is not one. Claw is a young language with
one maintainer, so there are no bug bounties and no promises of speed, but reports will be taken seriously.

What is **not** a vulnerability: anything inside `claw fn` (raw pointers, `alloc`/`free`, `extern`) that the language
says it trusts you with; `claw audit` lists every such place. Crashes with code 101 are checks working as designed.
