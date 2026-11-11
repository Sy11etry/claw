#!/usr/bin/env python3
"""Runs Claw programs built for `--target aarch64-linux-musl` inside a throwaway Alpine Linux VM.

usage: linux_check.py [--iso path/to/alpine-virt-aarch64.iso] [--keep] [--cmd 'shell commands to try first'] [test.claw ...]
Needs qemu-system-aarch64 (HVF) and an Alpine "virt" aarch64 ISO. Nothing is installed: the ISO boots as a live
system in RAM, the programs arrive on a second raw disk as a tar archive, and the output comes back on the serial line.
With no test files it takes every program in tests/run, tests/own and tests/par that has no `// exit:` or
`// build-only:` header. Output and exit code 0 must match the `// expect:` lines.
"""
import glob
import os
import pathlib
import re
import subprocess
import sys
import tarfile
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))
FIRMWARE = "/opt/homebrew/share/qemu/edk2-aarch64-code.fd"
VARS = "/opt/homebrew/share/qemu/edk2-arm-vars.fd"


def headers(text, key):
    return re.findall(rf"^// {key}: ?(.*)$", text, re.M)


class Vm:
    def __init__(self, iso, disk, workdir):
        vars_copy = pathlib.Path(workdir) / "vars.fd"
        vars_copy.write_bytes(pathlib.Path(VARS).read_bytes())
        cmd = [
            "qemu-system-aarch64", "-M", "virt", "-accel", "hvf", "-cpu", "host", "-m", "1024", "-smp", "2",
            "-nographic", "-nic", "none",
            "-drive", f"if=pflash,format=raw,readonly=on,file={FIRMWARE}",
            "-drive", f"if=pflash,format=raw,file={vars_copy}",
            "-cdrom", iso,
            "-drive", f"file={disk},format=raw,if=virtio",
        ]
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.buf = b""
        self.lock = threading.Lock()
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        while True:
            c = self.p.stdout.read1(4096)
            if not c:
                return
            with self.lock:
                self.buf += c

    def text(self):
        with self.lock:
            return self.buf.decode("utf-8", "replace")

    def send(self, s):
        self.p.stdin.write(s.encode())
        self.p.stdin.flush()

    def wait_for(self, pattern, start, timeout):
        end = time.time() + timeout
        while time.time() < end:
            m = re.search(pattern, self.text()[start:])
            if m:
                return start + m.end()
            if self.p.poll() is not None:
                break
            time.sleep(0.2)
        return None

    def close(self):
        if self.p.poll() is None:
            self.p.kill()


def main():
    args = sys.argv[1:]
    iso = None
    if "--iso" in args:
        i = args.index("--iso")
        iso = args[i + 1]
        del args[i:i + 2]
    keep = "--keep" in args
    args = [a for a in args if a != "--keep"]
    debug_cmd = None
    if "--cmd" in args:
        i = args.index("--cmd")
        debug_cmd = args[i + 1]
        del args[i:i + 2]
    if iso is None:
        found = sorted(glob.glob(os.path.expanduser("~/Downloads/alpine-virt-*-aarch64.iso")))
        if not found:
            sys.exit("no Alpine virt aarch64 ISO found; pass --iso")
        iso = found[-1]

    tests = [pathlib.Path(a) for a in args] or (
        sorted((ROOT / "tests/run").glob("*.claw"))
        + sorted((ROOT / "tests/own").glob("*.claw"))
        + [p for p in sorted((ROOT / "tests/par").glob("*.claw")) if not p.name.startswith("err_")]
    )
    work = pathlib.Path(tempfile.mkdtemp(prefix="claw-linux-"))
    built = []
    skipped = 0
    for t in tests:
        text = t.read_text()
        if headers(text, "exit") or headers(text, "build-only"):
            skipped += 1
            continue
        exe = work / (t.parent.name + "_" + t.stem + ".elf")
        p = subprocess.run([str(CLAW), "build", "--target", "aarch64-linux-musl", str(t), "-o", str(exe)], capture_output=True, text=True)
        if p.returncode != 0:
            print(f"FAIL {t.name}: compile failed: {p.stderr.strip()}")
            return 1
        built.append((t, exe, headers(text, "expect")))
    print(f"built {len(built)} programs for aarch64-linux-musl ({skipped} skipped)")

    tar_path = work / "tests.tar"
    with tarfile.open(tar_path, "w") as tf:
        for _, exe, _ in built:
            tf.add(exe, arcname=exe.name)
    disk = work / "disk.img"
    data = tar_path.read_bytes()
    disk.write_bytes(data + b"\0" * (-len(data) % 512 + 1024 * 1024))

    vm = Vm(iso, disk, work)
    failures = 0
    try:
        at = vm.wait_for(r"localhost login: ", 0, 240)
        if at is None:
            print("FAIL: the VM did not reach a login prompt\n" + vm.text()[-1500:])
            return 1
        vm.send("root\n")
        at = vm.wait_for(r"# $", at, 60) or at
        vm.send("mkdir -p /tmp/t && cd /tmp/t && for d in /dev/vd? ; do tar tf $d >/dev/null 2>&1 && tar xf $d && break; done; ls *.elf | wc -l && echo SETUP-DONE\n")
        at = vm.wait_for(r"SETUP-DONE\r?\n", at, 60)
        if at is None:
            print("FAIL: could not unpack the programs\n" + vm.text()[-1500:])
            return 1
        if debug_cmd:
            vm.send(f"{debug_cmd}; echo DEBUG-DONE\n")
            end = vm.wait_for(r"DEBUG-DONE\r?\n", at, 60)
            print(vm.text()[at:end])
        for t, exe, want in built:
            mark = f"@@{exe.stem}@@"
            vm.send(f"echo {mark}BEGIN; ./{exe.name}; echo {mark}END$?\n")
            end = vm.wait_for(rf"{re.escape(mark)}END(\d+)\r?\n", at, 60)
            if end is None:
                print(f"FAIL {t.name}: no result (hung?)")
                failures += 1
                at = len(vm.text())
                continue
            chunk = vm.text()[at:end]
            m = re.search(rf"{re.escape(mark)}BEGIN\r?\n(.*?){re.escape(mark)}END(\d+)", chunk, re.S)
            out = m[1].replace("\r", "").splitlines() if m else []
            code = int(m[2]) if m else -1
            if code != 0 or out != want:
                failures += 1
                print(f"FAIL {t.name}: exit {code}\n  got:  {out[:6]}\n  want: {want[:6]}")
            at = end
        vm.send("poweroff\n")
    finally:
        vm.close()
        if not keep:
            for f in work.glob("*"):
                f.unlink()
            work.rmdir()
    print(f"{len(built) - failures}/{len(built)} programs gave the same output on Linux")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
