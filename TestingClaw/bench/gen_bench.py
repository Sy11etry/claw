#!/usr/bin/env python3
"""Writes loop and par.build versions of the same work at three task sizes.

Each task runs `inner` iterations of a small integer loop; total work is about the same for each size:
tasks * inner = 2e7 .. 2e8 iterations. Both versions must print the same checksum.
"""
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
SIZES = [(10, 2_000_000), (1000, 200_000), (1_000_000, 200)]  # (inner, tasks)

HEAD = """use par

const inner = {inner}
const tasks = {tasks}

fn work(x int) int {{
    mut s := 0
    for i in 0..inner {{
        s += (x * i + i) % 7
    }}
    ret s
}}

"""
LOOP = """fn main() {
    mut out []int = []
    for i in 0..tasks {
        push(&out, work(i))
    }
    mut total := 0
    for v in out {
        total += v
    }
    print(total)
}
"""
PAR = """fn main() {
    out := par.build(tasks, work)
    mut total := 0
    for v in out {
        total += v
    }
    print(total)
}
"""

for inner, tasks in SIZES:
    (HERE / f"bench_loop_{inner}.claw").write_text(HEAD.format(inner=inner, tasks=tasks) + LOOP)
    (HERE / f"bench_par_{inner}.claw").write_text(HEAD.format(inner=inner, tasks=tasks) + PAR)
    print(f"inner={inner} tasks={tasks} iterations={inner * tasks}")
