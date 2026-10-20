#!/usr/bin/env python3
"""Collect the VFS DLLs and their helpers into the layout Agora finds at run time.

A game is started under `agora_vfs.dll` (MASTER_SPEC §26.5). The 64-bit DLL and its helper sit
beside the application; the 32-bit pair sits in an `x86` folder beside them, which is where
`locate_dll_x86` in agora-core looks. This script builds that layout from the two cargo outputs:

    <stage>/agora_vfs.dll               target/release/
    <stage>/agora-vfs-helper.exe        target/release/
    <stage>/x86/agora_vfs.dll           target/i686-pc-windows-msvc/release/
    <stage>/x86/agora-vfs-helper.exe    target/i686-pc-windows-msvc/release/

Each file's PE machine type is checked, so a 64-bit build cannot land in the x86 folder (or the
reverse) and be shipped as if it were the other bitness. Nothing is copied unless all four are
right. The stage folder is `desktop/src-tauri/vfs`, which `tauri.bundle-vfs.conf.json` names as the
bundle's resources and `scripts/package_portable.ps1` reads for the portable ZIP.

Usage (after `cargo build --release -p agora-vfs --lib --bin agora-vfs-helper` for both targets):
    python scripts/stage_vfs_files.py [--x64-build DIR] [--x86-build DIR] [--stage-dir DIR]

The desktop bundle and the portable ZIP use the defaults. The CLI archive points the x64 build at
target/x86_64-pc-windows-msvc/release and stages into its own folder.
"""

from __future__ import annotations

import argparse
import shutil
import struct
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
STAGE_DIR = REPO_ROOT / "desktop" / "src-tauri" / "vfs"

X64_BUILD = REPO_ROOT / "target" / "release"
X86_BUILD = REPO_ROOT / "target" / "i686-pc-windows-msvc" / "release"

DLL_NAME = "agora_vfs.dll"
HELPER_NAME = "agora-vfs-helper.exe"

IMAGE_FILE_MACHINE_I386 = 0x014C
IMAGE_FILE_MACHINE_AMD64 = 0x8664


def pe_machine(path: Path) -> int:
    """The PE machine type of `path`. Raises ValueError for a file that is not a PE image."""
    data = path.read_bytes()
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise ValueError(f"{path} is not a Windows executable")
    (pe_offset,) = struct.unpack_from("<I", data, 0x3C)
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise ValueError(f"{path} has no PE header")
    (machine,) = struct.unpack_from("<H", data, pe_offset + 4)
    return machine


def plan(x64_build: Path, x86_build: Path, stage: Path) -> list[tuple[Path, Path, int]]:
    """The (source, destination, expected machine) of every file, in copy order."""
    return [
        (x64_build / DLL_NAME, stage / DLL_NAME, IMAGE_FILE_MACHINE_AMD64),
        (x64_build / HELPER_NAME, stage / HELPER_NAME, IMAGE_FILE_MACHINE_AMD64),
        (x86_build / DLL_NAME, stage / "x86" / DLL_NAME, IMAGE_FILE_MACHINE_I386),
        (x86_build / HELPER_NAME, stage / "x86" / HELPER_NAME, IMAGE_FILE_MACHINE_I386),
    ]


def stage(x64_build: Path, x86_build: Path, stage_dir: Path) -> list[Path]:
    """Check every source, then copy them all. Returns the staged files.

    Nothing is copied when any source is missing or has the wrong bitness, and a stage folder left
    by an earlier run is cleared first so it cannot keep a stale file.
    """
    entries = plan(x64_build, x86_build, stage_dir)
    problems = []
    for source, _, machine in entries:
        if not source.is_file():
            problems.append(f"missing {source}; build agora-vfs for this target first")
            continue
        try:
            found = pe_machine(source)
        except (OSError, ValueError, struct.error) as why:
            problems.append(f"{source} could not be read: {why}")
            continue
        if found != machine:
            problems.append(
                f"{source} is machine 0x{found:04x}, expected 0x{machine:04x} for its folder"
            )
    if problems:
        raise SystemExit("error: " + "; ".join(problems))

    if stage_dir.exists():
        shutil.rmtree(stage_dir)
    staged = []
    for source, destination, _ in entries:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        staged.append(destination)
    return staged


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--x64-build",
        type=Path,
        default=X64_BUILD,
        help="folder holding the 64-bit agora_vfs.dll and helper (default: target/release)",
    )
    parser.add_argument(
        "--x86-build",
        type=Path,
        default=X86_BUILD,
        help="folder holding the 32-bit pair (default: target/i686-pc-windows-msvc/release)",
    )
    parser.add_argument(
        "--stage-dir",
        type=Path,
        default=STAGE_DIR,
        help="where the layout is written; replaced if it exists (default: desktop/src-tauri/vfs)",
    )
    args = parser.parse_args(argv)
    for path in stage(args.x64_build, args.x86_build, args.stage_dir):
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
