#!/usr/bin/env python3
"""Installs the current build into ~/.claw. Older versions are never removed.

~/.claw/versions/0.N/claw     the compiler of that version
~/.claw/bin/claw-0.N          stable name for that version
~/.claw/bin/claw              the newest installed version
"""
import os
import pathlib
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUILD = ROOT / "compiler/target/release/claw"
HOME = pathlib.Path.home() / ".claw"
PATH_LINE = 'export PATH="$HOME/.claw/bin:$PATH"  # claw'


def main():
    full = subprocess.run([BUILD, "--version"], capture_output=True, text=True, check=True).stdout.split()[1]
    short = ".".join(full.split(".")[:2])
    version_dir = HOME / "versions" / short
    version_dir.mkdir(parents=True, exist_ok=True)
    bin_dir = HOME / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    # Copy then rename: replacing an executable in place can break its signature cache.
    target = version_dir / "claw"
    tmp = version_dir / "claw.tmp"
    shutil.copy2(BUILD, tmp)
    os.replace(tmp, target)

    for name in (f"claw-{short}", "claw"):
        link = bin_dir / name
        tmp_link = bin_dir / (name + ".tmp")
        if tmp_link.is_symlink() or tmp_link.exists():
            tmp_link.unlink()
        tmp_link.symlink_to(target)
        os.replace(tmp_link, link)

    zshrc = pathlib.Path.home() / ".zshrc"
    existing = zshrc.read_text() if zshrc.exists() else ""
    if ".claw/bin" not in existing:
        with zshrc.open("a") as f:
            f.write(f"\n{PATH_LINE}\n")
        print(f"added ~/.claw/bin to PATH in {zshrc}")

    installed = sorted(p.name for p in (HOME / "versions").iterdir())
    print(f"installed claw {full} as claw-{short}; versions on disk: {', '.join(installed)}")


if __name__ == "__main__":
    main()
