# Exam: Claude Haiku writes Claw from the card alone

The model got `docs/CARD.md` and the task statements. No compiler, no other files, one try per program. `tools/exam.py` builds each answer and compares its output with the task's expected output. The tasks are in `benchmarks/exam/`, and the final programs are kept as `tests/run/exam_*.claw`.

| Round | Card and language | Tasks | Built and printed the right answer |
|---|---|---|---|
| First exam | 0.8 before the fixes below | 24 | **19 of 24 (79%)** |
| Same answers, regraded | after `x := a[i]` became legal | 24 | 20 of 24 (83%) |
| Second exam, new tasks | final | 12 | **11 of 12 (92%)** |
| One round of fixes, using only the compiler's messages | final | the 5 still failing | **5 of 5** |

All 36 tasks ended up solved, none after more than one round of fixes.

## What the exam found

- Four of the five failures in the first exam were the same thing: keeping an element or a slice in a variable (`sym := syms[i]`). The card promised `v := b[1..]`, which the compiler did not accept. Now `x := a[i]` is a read-only view; `mut x := a[i]` still needs `copy`, and the compiler's message says so.
- A `{` in a string is always an expression, and `"(a[b]{c})"` was written without doubling it. The message now says so.
- In the second exam, `pop` was used without noticing that it returns `?int`. The compiler's message was enough to fix it.

## How much weight this can bear

One run per exam, 36 small tasks, written by the author of the language. The second exam ran after the language and card were improved from the first, on new tasks. Treat it as evidence that the card works for a weak model, not as a precise rate.
