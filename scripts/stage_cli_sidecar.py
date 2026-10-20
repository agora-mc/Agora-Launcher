#!/usr/bin/env python3
"""Copy a built `agora` CLI to where Tauri bundles it into the desktop app.

Tauri's `bundle.externalBin` entry `binaries/agora` expects
`desktop/src-tauri/binaries/agora-<target-triple>[.exe]` at bundle time and
installs it beside the desktop executable as plain `agora[.exe]`. The entry
lives in `tauri.bundle-cli.conf.json`, which only release builds merge in, so
everyday builds do not need a CLI binary on disk.

Usage (after `cargo build --release -p agora-cli [--target <triple>]`):
    python scripts/stage_cli_sidecar.py [--target <triple>]

Without --target the host triple is used and the binary is read from
target/release/, matching a build without --target.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SIDECAR_DIR = REPO_ROOT / "desktop" / "src-tauri" / "binaries"


def exe_suffix(triple: str) -> str:
    return ".exe" if "windows" in triple else ""


def built_cli_path(target_root: Path, triple: str, explicit_target: bool) -> Path:
    """Where cargo put the CLI: target/<triple>/release with --target, else target/release."""
    release = target_root / triple / "release" if explicit_target else target_root / "release"
    return release / f"agora{exe_suffix(triple)}"


def sidecar_path(sidecar_dir: Path, triple: str) -> Path:
    return sidecar_dir / f"agora-{triple}{exe_suffix(triple)}"


def host_triple() -> str:
    output = subprocess.run(
        ["rustc", "-vV"], check=True, capture_output=True, text=True
    ).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise RuntimeError("rustc -vV did not report a host triple")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", help="Rust target triple the CLI was built for")
    args = parser.parse_args()

    triple = args.target or host_triple()
    source = built_cli_path(REPO_ROOT / "target", triple, explicit_target=bool(args.target))
    if not source.is_file():
        print(f"error: no CLI build at {source}; build agora-cli first", file=sys.stderr)
        return 1
    destination = sidecar_path(SIDECAR_DIR, triple)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    print(destination)
    return 0


if __name__ == "__main__":
    sys.exit(main())
