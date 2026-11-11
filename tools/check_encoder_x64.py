#!/usr/bin/env python3
"""Checks our x86-64 encoder against LLVM's assembler, instruction by instruction.

Plain instructions are compared with `llvm-mc -show-encoding`. Branches ("jne L", "call L")
carry a rel32 that the assembler only settles when it lays out the code, so those are assembled
into an object file with the label 200 bytes ahead and the bytes are read back with otool.
"""
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = ROOT / "compiler/target/release/claw"
MC = "/opt/homebrew/opt/llvm/bin/llvm-mc"
TRIPLE = "-triple=x86_64-apple-macos"
HEAD = ".intel_syntax noprefix\n"

lines = subprocess.run([CLAW, "__encoder-golden-x64"], capture_output=True, text=True, check=True).stdout.splitlines()
pairs = [l.split("\t") for l in lines]
plain = [(h, t) for h, t in pairs if not t.endswith(" L")]
branches = [(h, t) for h, t in pairs if t.endswith(" L")]

bad = 0

# Plain instructions.
p = subprocess.run([MC, TRIPLE, "-show-encoding"], input=HEAD + "\n".join(t for _, t in plain) + "\n", capture_output=True, text=True)
if p.returncode != 0:
    sys.exit(f"llvm-mc failed:\n{p.stderr}")
encodings = re.findall(r"encoding: \[(.*?)\]", p.stdout)
if len(encodings) != len(plain):
    sys.exit(f"llvm-mc returned {len(encodings)} encodings for {len(plain)} instructions")
for (ours, text), enc in zip(plain, encodings):
    theirs = "".join(f"{int(x, 16):02x}" for x in enc.split(","))
    if ours != theirs:
        bad += 1
        print(f"MISMATCH {text}: ours {ours}, llvm {theirs}")

# Branches: assemble for real, so the displacement is resolved.
if branches:
    src = HEAD
    for i, (_, t) in enumerate(branches):
        src += f"{t.replace(' L', f' L{i}')}\n.space 200, 0x90\nL{i}:\n"
    with tempfile.TemporaryDirectory() as d:
        obj = pathlib.Path(d) / "b.o"
        p = subprocess.run([MC, TRIPLE, "-filetype=obj", "-o", str(obj)], input=src, capture_output=True, text=True)
        if p.returncode != 0:
            sys.exit(f"llvm-mc failed:\n{p.stderr}")
        dump = subprocess.run(["otool", "-t", str(obj)], capture_output=True, text=True).stdout
    data = bytearray()
    for l in dump.splitlines():
        m = re.match(r"^[0-9a-f]{16}[ \t](.*)$", l)
        if m:
            data += bytes.fromhex(m[1].replace(" ", ""))
    at = 0
    for ours, text in branches:
        n = len(ours) // 2
        theirs = data[at : at + n].hex()
        if ours != theirs:
            bad += 1
            print(f"MISMATCH {text}: ours {ours}, llvm {theirs}")
        at += n + 200

print(f"{len(pairs) - bad}/{len(pairs)} encodings match llvm-mc")
sys.exit(1 if bad else 0)
