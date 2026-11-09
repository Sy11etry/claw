# Token cycle

**These are estimates**, not a real tokenizer: two counting rules, applied the same way to all three languages. All three programs of each task print the same output (checked on every run).

| Task | Claw | C | Rust | Claw vs C | Claw vs Rust |
|---|---|---|---|---|---|
| apply_all | 103 | 149 | 128 | -31% | -20% |
| average | 87 | 149 | 127 | -42% | -31% |
| bank | 188 | 245 | 243 | -23% | -23% |
| binary_search | 117 | 158 | 173 | -26% | -32% |
| count_vowels | 70 | 87 | 58 | -20% | +21% |
| fib_table | 75 | 110 | 79 | -32% | -5% |
| first_negative | 64 | 157 | 94 | -59% | -32% |
| fizzbuzz | 65 | 101 | 77 | -36% | -16% |
| gcd_lcm | 75 | 102 | 92 | -26% | -18% |
| generic_max | 96 | 122 | 121 | -21% | -21% |
| insertion_sort | 119 | 171 | 119 | -30% | +0% |
| matrix | 152 | 208 | 180 | -27% | -16% |
| palindrome | 80 | 148 | 112 | -46% | -29% |
| parse_int | 154 | 232 | 210 | -34% | -27% |
| points | 131 | 175 | 177 | -25% | -26% |
| shapes | 98 | 165 | 122 | -41% | -20% |
| sieve | 67 | 107 | 80 | -37% | -16% |
| split_join | 51 | 225 | 82 | -77% | -38% |
| stack | 113 | 266 | 150 | -58% | -25% |
| tree | 250 | 319 | 247 | -22% | +1% |
| word_freq | 122 | 270 | 99 | -55% | +23% |

| Total over all tasks | Claw | C | Rust | Claw vs C | Claw vs Rust |
|---|---|---|---|---|---|
| `words` estimate | 2277 | 3666 | 2770 | -37.9% | -17.8% |
| `pessimistic` estimate | 2360 | 3764 | 2940 | -37.3% | -19.7% |
| Median task (`words`) | | | | -31.8% | -19.7% |

## Fixing: tokens in the compiler's message for the same mistake

`clang -Wall -Wextra` and `rustc`, default output. `-` means the compiler says nothing at all.

| Mistake | Claw | clang | rustc |
|---|---|---|---|
| change_immutable | 39 | 118 | 237 |
| missing_case | 31 | 118 | 192 |
| missing_return | 36 | 56 | 157 |
| unknown_name | 30 | 58 | 122 |
| unused_var | 42 | 52 | 135 |
| use_after_move | 46 | - | 168 |
| wrong_args | 34 | 122 | 184 |
| wrong_type | 30 | 127 | 148 |
| **Total** | **288** | **651** | **1343** |

## Reading

`claw sig` over these programs: 514 tokens instead of 2277 for the full source (23%).
