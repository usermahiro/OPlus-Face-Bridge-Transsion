#!/usr/bin/env python3
"""Generate link-time stub libraries for CI.

build.rs links the HAL against vendor libraries that only exist inside the
ROM (libstfaceunlockppl.so, libcamera2ndk_vendor.so, libmediandk.so with the
vendor-only AImageReader symbols). CI has no ROM tree, so this script scans the
`extern "C"` blocks in src/*.rs and emits empty stub .so files that define
exactly those symbols, laid out like a ROM tree:

    <out>/vendor/lib64/libstfaceunlockppl.so
    <out>/vendor/lib64/libcamera2ndk_vendor.so
    <out>/vendor/lib64/libmediandk.so

Point ROM_TREE=<out> at it. The stubs are used for linking only; they are never
shipped. At runtime the device's real libraries satisfy the DT_NEEDED entries.

Usage: ci_stubs.py <src_dir> <out_dir> <cc> [extra cc args...]
"""
import os
import re
import subprocess
import sys
import tempfile

# symbol prefix -> library that provides it on the device
OWNERS = [
    (re.compile(r"^st_"), "libstfaceunlockppl.so"),
    (re.compile(r"^(ACamera|ACapture)"), "libcamera2ndk_vendor.so"),
    (re.compile(r"^AImage"), "libmediandk.so"),
]
# Everything else (ANativeWindow_*, __android_log_print, dlopen, ...) comes
# from the NDK sysroot (libnativewindow, liblog, libc) and needs no stub.


def extern_symbols(src_dir):
    names = set()
    for fn in sorted(os.listdir(src_dir)):
        if not fn.endswith(".rs"):
            continue
        text = open(os.path.join(src_dir, fn), encoding="utf-8").read()
        for blk in re.finditer(r'extern "C" \{(.*?)\n\}', text, re.S):
            names.update(re.findall(r"\bfn\s+(\w+)", blk.group(1)))
    return names


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    src_dir, out_dir, cc = sys.argv[1:4]
    cc_args = sys.argv[4:]

    libs = {}
    for name in sorted(extern_symbols(src_dir)):
        for rx, lib in OWNERS:
            if rx.search(name):
                libs.setdefault(lib, []).append(name)
                break

    if not libs:
        sys.exit("no extern symbols found - wrong src dir?")

    lib_dir = os.path.join(out_dir, "vendor", "lib64")
    os.makedirs(lib_dir, exist_ok=True)

    for lib, syms in sorted(libs.items()):
        with tempfile.TemporaryDirectory() as td:
            c_file = os.path.join(td, "stub.c")
            with open(c_file, "w") as f:
                for s in syms:
                    f.write(f"void {s}(void) {{}}\n")
            out = os.path.join(lib_dir, lib)
            cmd = [cc, *cc_args, "-shared", "-fPIC", "-o", out,
                   f"-Wl,-soname,{lib}", c_file]
            subprocess.check_call(cmd)
        print(f"stub {lib}: {len(syms)} symbols")


if __name__ == "__main__":
    main()
