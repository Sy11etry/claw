#!/usr/bin/env python3
"""Checks our ARM64 encoder against LLVM's assembler, instruction by instruction."""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = ROOT / "compiler/target/release/claw"
MC = "/opt/homebrew/opt/llvm/bin/llvm-mc"

lines = subprocess.run([CLAW, "__encoder-golden"], capture_output=True, text=True, check=True).stdout.splitlines()
pairs = [l.split("\t") for l in lines]
asm = "\n".join(t for _, t in pairs) + "\n"
p = subprocess.run([MC, "-triple=arm64-apple-macos", "-show-encoding"], input=asm, capture_output=True, text=True)
if p.returncode != 0:
    sys.exit(f"llvm-mc failed:\n{p.stderr}")

encodings = re.findall(r"encoding: \[(.*?)\]", p.stdout)
if len(encodings) != len(pairs):
    sys.exit(f"llvm-mc returned {len(encodings)} encodings for {len(pairs)} instructions")

bad = 0
for (ours, text), enc in zip(pairs, encodings):
    b = [int(x, 16) for x in enc.split(",")]
    theirs = f"{b[3]:02x}{b[2]:02x}{b[1]:02x}{b[0]:02x}"
    if ours != theirs:
        bad += 1
        print(f"MISMATCH {text}: ours {ours}, llvm {theirs}")
print(f"{len(pairs) - bad}/{len(pairs)} encodings match llvm-mc")
sys.exit(1 if bad else 0)
