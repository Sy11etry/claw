# Коды ошибок: проверка по `docs/SPEC.md`

Программы: `TestingClaw/errors/`, генератор: `gen_e.py` и `gen_e2.py`, результаты: `errors_results2.json`.
Понятность: **3** = понятно, с местом и подсказкой; **2** = понятно не до конца (нет имени, подсказка неполная или не выполнима); **1** = сообщение неверное или код пропущен.

Из 88 программ код сработал в 86. Не сработали: `E0900` (резерв: 11 вариантов строки с `{}` в `errors/e0900_variants/` его не дали) и структурный случай E0144 (находка F31).

Не проверено: E0902 (форматтер отказался менять файл: на 4 входах `claw fmt` работал без отказа), E0907 (в SPEC помечен «не используется»).

| код | сработал | сообщение компилятора | понятность | claw fix | заметка |
|---|---|---|---|---|---|
| E0001 | да | unexpected character `@` | 3 |  |  |
| E0002 | да | string is not closed; add `"` | 3 |  |  |
| E0003 | да | number runs into a name; add a space | 3 |  |  |
| E0004 | да | unknown escape; use \n \t \r \0 \\ \" | 3 |  |  |
| E0005 | да | a character literal holds exactly one ASCII character; close it with `'` | 3 |  |  |
| E0101 | да | unknown name `zz` | 3 |  |  |
| E0102 | да | `len` is a built-in name | 3 |  |  |
| E0103 | да | `x` cannot change; fix: declare `mut x` | 3 | applied (компилируется) |  |
| E0104 | да | expected i32, found str | 3 |  |  |
| E0105 | да | `x` is never read; fix: rename `x` to `_` | 3 | not applied (остаётся E0105) | подсказка «rename to _» не применяется `claw fix` |
| E0106 | да | `f` takes 1 values, found 2 | 2 |  | «takes 1 values» (грамматика), не сказано, какая форма ожидается |
| E0107 | да | `f` can reach its end without `ret` | 3 |  |  |
| E0108 | да | `break` and `continue` only work inside `for` | 3 |  |  |
| E0109 | да | `main` takes no parameters and returns nothing | 3 |  |  |
| E0110 | да | function `f` is defined twice | 1 |  | для `const` и `fn` с одним именем: сообщение говорит «function defined twice»; код должен быть E0102 |
| E0111 | да | result is not used; fix: assign it to `_` if it really does not matter | 2 | not applied (остаётся E0111) | подсказка `_ = …` для `T ! E` даёт E0125; `claw fix` её не применяет |
| E0112 | да | unknown type `Foo` | 3 |  |  |
| E0113 | да | this code can never run | 3 |  |  |
| E0114 | да | this expression does nothing | 3 |  |  |
| E0115 | да | number does not fit in int | 3 |  |  |
| E0116 | да | `x` is `mut` but never changes; fix: remove `mut` | 3 | applied (компилируется) |  |
| E0117 | да | an `else` block must leave: end it with `ret`, `fail`, `break` or `continue` | 3 |  |  |
| E0118 | да | this pattern does not match the type being matched | 2 |  | не назван ни тип, ни образец |
| E0119 | да | `A` has 1 fields, found 2 names | 3 |  |  |
| E0120 | да | `match` misses: B | 3 |  |  |
| E0121 | да | this case is already handled | 2 |  | не назван вариант, который повторён |
| E0122 | да | `a` cannot change while it is being walked, matched or viewed | 3 |  |  |
| E0123 | да | cannot tell which `?T` this `none` is; give the variable a type | 3 |  |  |
| E0124 | да | `fail` needs a function returning `T ! E` | 3 |  |  |
| E0125 | да | an error cannot be ignored; use `?` or `else` | 3 |  |  |
| E0126 | да | array size must be at least 1 | 3 |  |  |
| E0127 | да | type `T` contains itself, so it would be infinitely large; use a pointer | 2 |  | подсказка «use a pointer» выполнима только внутри `claw fn` |
| E0128 | да | type `point` must start with a capital letter | 3 |  |  |
| E0129 | да | `P` has no field `zz` | 3 |  |  |
| E0130 | да | index 5 is outside `[2]int` | 3 |  |  |
| E0131 | да | this cannot be assigned to | 2 |  | не сказано, что вообще можно присваивать |
| E0132 | да | unknown effect `wat`; use `+mem` or `+io` | 3 |  |  |
| E0133 | да | `{` in this string is not closed; write `{{` for a literal brace | 3 |  |  |
| E0134 | да | `??T` is not allowed | 2 |  | не подсказано, что писать вместо `??T` (нужно `?T`) |
| E0135 | да | a range gives one value per step; use one loop variable | 3 |  |  |
| E0136 | да | `x...` fills a whole array, so it must be the only item | 3 |  |  |
| E0137 | да | field `x` appears twice | 3 |  |  |
| E0138 | да | `?T` has no error value to name; write `else { ... }` | 3 |  |  |
| E0139 | да | a `const` must be computable from literals and earlier constants; with a stated type it may call functions: `const x int = f()` | 3 |  |  |
| E0140 | да | `alloc` only exists inside `claw fn` | 3 |  |  |
| E0141 | да | `extern fn abs` can only be called from `claw fn` | 3 |  |  |
| E0142 | да | `defer` only exists inside `claw fn` | 3 |  |  |
| E0143 | да | `defer` cannot leave the function | 3 |  |  |
| E0144 | да | `pair` takes 1 type arguments, found 2 | 3 |  |  |
| E0145 | да | `f` keeps instantiating itself with new types | 3 |  |  |
| E0146 | да | cannot find module `nothere` | 2 |  | не сказано про `claw.pkg` (см. F22) |
| E0149 | да | `extern fn` takes only numbers, bool and pointers; pass a pointer to the struct | 3 |  |  |
| E0150 | да | Claw has no methods; write `len(a, ...)` | 2 |  | подсказка `len(a, ...)` неверна: нужно `len(a)` |
| E0151 | да | `a` was moved away at line 4; fix: write `copy(a)` there | 3 | applied (компилируется) |  |
| E0153 | да | `[x...]` would share one owner between many items; build the array item by item | 3 |  |  |
| E0154 | да | H has its own `drop`, so the compiler cannot copy it | 3 |  |  |
| E0156 | да | `drop` runs by itself; it cannot be called | 3 |  |  |
| E0157 | да | `&x` only works as an argument to a `&` parameter | 3 |  |  |
| E0158 | да | `x` is passed with `&` twice in one call | 3 |  |  |
| E0160 | да | `f` does not declare `+mem`, but `push` has it; fix: add `+mem` to the signature | 3 | applied (компилируется) |  |
| E0161 | да | a contract cannot have side effects, but `g` is `+io` | 3 |  |  |
| E0162 | да | this function returns nothing, so `sure` has no `ret` to look at | 3 |  |  |
| E0163 | да | `old` remembers numbers and bool, not []int; try `old(len(x))` | 3 |  |  |
| E0164 | да | a contract only looks: it cannot take `&` | 3 |  |  |
| E0165 | да | `s` was taken over with `own`, so it may be gone when `sure` runs; use `old(...)` | 3 |  |  |
| E0166 | да | `main` is called by the compiler, so it cannot have contracts | 3 |  |  |
| E0167 | да | there is already a test named "a" | 3 |  |  |
| E0168 | да | `need` belongs in a signature; inside a body write `sure` | 3 |  |  |
| E0171 | да | `x` is computed while compiling, so it cannot use `+io` | 3 |  |  |
| E0173 | да | a computed `const` cannot be []int: it must be plain data or `str` | 3 |  |  |
| E0174 | да | cannot compute `c`: division by zero at e0174_const_fails.claw:3:9 | 3 |  |  |
| E0175 | да | `b` needs `a`, which needs `b` back | 3 |  |  |
| E0176 | да | `twice` is a template, so it is not one function; wrap the use you want in a small `fn` | 3 |  |  |
| E0177 | да | `needy` has a `need`, and a call through a value could not check it; wrap it in a `fn` that handles every input | 3 |  |  |
| E0178 | да | `work` is `+io`, which this function type does not allow; add `+io` to the type | 3 |  | верно для типа функции; для `par` совет «добавь +io» не годится (F19) |
| E0179 | да | `T` contains itself, so it cannot be printed whole; write a function that walks it | 3 |  |  |
| E0201 | да | expected `)`, found `}` | 3 |  |  |
| E0202 | да | `in` is a keyword and cannot be used as a parameter name | 3 |  |  |
| E0203 | да | `else` must be on the same line as the closing `}` | 3 |  |  |
| E0204 | да | block is not closed; add `}` | 3 |  |  |
| E0205 | да | one statement per line; found name `y` after the statement | 3 |  |  |
| E0905 | да | SIMD vectors only exist inside `claw fn` | 3 |  |  |
| E0906 | да | a SIMD value cannot be a field; keep the lanes in an array and use `vload`/`vstore` | 3 |  |  |
| E0908 | да | `vload` needs a pointer to `f64`, `f32` or `i32` | 3 |  |  |
| E0909 | да | `/` is not available on i32x4: vectors have `+ - *`, and `/` on float lanes | 3 |  |  |
| E0900 | нет | (не сработал) | 2 |  | код зарезервирован: 11 вариантов строки с `{}` не дали его |
| E0901 | да | function `big` is too large for this version: at most 1 MB of code and 7 MB of local data | 3 |  |  |
| E0144 (struct case) | нет | (не сработал) | 1 |  | `Box[int, int]{1}` компилируется и печатает 1: проверка пропущена для структуры |
