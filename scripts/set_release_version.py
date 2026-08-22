#!/usr/bin/env python3
"""Set (or verify) the release version across every file that carries it.

`docs/RELEASING.md` lists "Version metadata agrees across desktop package
files" as a manual pre-tag checklist item. It is easy to miss: the `v0.5.0`
release shipped installers named `Agora.Launcher_0.1.0_*` because the tag was
the only thing that moved. The release workflow now calls this script so the
tag is the single source of truth.

Six places carry the version:

  1. Cargo.toml                            [workspace.package] -> agora-core, agora-cli
  2. desktop/src-tauri/Cargo.toml          [package]           -> agora-desktop
  3. desktop/src-tauri/tauri.conf.json     bundle filenames, app metadata, updater
  4. desktop/package.json                  frontend package metadata
  5. desktop/src/components/Sidebar.tsx    read via __APP_VERSION__ (no literal)
  6. Cargo.lock                            workspace member entries

(5) needs no edit — it reads the value Vite injects from desktop/package.json.
(6) matters because CI builds with `cargo build --locked`: bumping a workspace
version without refreshing the lock makes every locked build fail outright.

Usage:
    python scripts/set_release_version.py v1.2.3   # write
    python scripts/set_release_version.py --check  # verify agreement, no writes
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


def read(path: Path) -> str:
    """Read without newline translation, so CRLF files stay CRLF."""
    with open(path, "r", encoding="utf-8", newline="") as fh:
        return fh.read()


def write(path: Path, text: str) -> None:
    """Write verbatim. `Path.write_text` would rewrite every LF to os.linesep
    on Windows, turning a version bump into a whole-file line-ending diff."""
    with open(path, "w", encoding="utf-8", newline="") as fh:
        fh.write(text)


ROOT = Path(__file__).resolve().parent.parent

# Tauri bundles Windows MSI/NSIS installers, whose version fields are numeric.
# Keep this strict rather than discovering it at package time on a tagged run.
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")

WORKSPACE_MEMBERS = ("agora-core", "agora-cli", "agora-desktop")


def _section_version(text: str, section: str) -> tuple[str | None, re.Pattern[str]]:
    """Match `version = "..."` inside one TOML section, not in dependencies."""
    pattern = re.compile(
        r"(?ms)^\[" + re.escape(section) + r"\]\s*$.*?^version\s*=\s*\"([^\"]+)\""
    )
    m = pattern.search(text)
    return (m.group(1) if m else None), pattern


def _read_toml_version(path: Path, section: str) -> str | None:
    return _section_version(read(path), section)[0]


def _write_toml_version(path: Path, section: str, version: str) -> bool:
    text = read(path)
    current, pattern = _section_version(text, section)
    if current is None:
        raise SystemExit(f"ERROR: no [{section}] version in {path}")
    if current == version:
        return False
    m = pattern.search(text)
    assert m
    start, end = m.span(1)
    write(path, text[:start] + version + text[end:])
    return True


# Anchored at two-space indentation so it only ever matches the top-level key,
# never a nested "version" inside dependencies or bundle config.
JSON_VERSION = re.compile(r'(?m)^(  "version":\s*")([^"]+)(")')


def _read_json_version(path: Path) -> str | None:
    m = JSON_VERSION.search(read(path))
    return m.group(2) if m else None


def _write_json_version(path: Path, version: str) -> bool:
    text = read(path)
    m = JSON_VERSION.search(text)
    if not m:
        raise SystemExit(f'ERROR: no top-level "version" in {path}')
    if m.group(2) == version:
        return False
    write(path, JSON_VERSION.sub(rf"\g<1>{version}\g<3>", text, count=1))
    return True


def _lock_pattern(member: str) -> re.Pattern[str]:
    return re.compile(
        # \r?\n, not \n: files are read with newline='' to preserve line
        # endings, so on Windows this text arrives as CRLF.
        r'(?m)(^\[\[package\]\]\r?\nname = "' + re.escape(member) + r'"\r?\nversion = ")([^"]+)(")'
    )


def _read_lock_versions(path: Path) -> dict[str, str | None]:
    """None for a member the lockfile does not list, so --check fails loudly
    rather than silently comparing a smaller set."""
    text = read(path)
    out: dict[str, str | None] = {}
    for member in WORKSPACE_MEMBERS:
        m = _lock_pattern(member).search(text)
        out[member] = m.group(2) if m else None
    return out


def _write_lock_versions(path: Path, version: str) -> bool:
    text = read(path)
    original = text
    for member in WORKSPACE_MEMBERS:
        pattern = _lock_pattern(member)
        if not pattern.search(text):
            raise SystemExit(f"ERROR: {member} missing from {path}")
        text = pattern.sub(rf"\g<1>{version}\g<3>", text, count=1)
    if text == original:
        return False
    write(path, text)
    return True


TARGETS = [
    ("Cargo.toml", "[workspace.package]", lambda: _read_toml_version(ROOT / "Cargo.toml", "workspace.package")),
    ("desktop/src-tauri/Cargo.toml", "[package]", lambda: _read_toml_version(ROOT / "desktop/src-tauri/Cargo.toml", "package")),
    ("desktop/src-tauri/tauri.conf.json", "version", lambda: _read_json_version(ROOT / "desktop/src-tauri/tauri.conf.json")),
    ("desktop/package.json", "version", lambda: _read_json_version(ROOT / "desktop/package.json")),
]


def current_versions() -> dict[str, str | None]:
    found = {label: getter() for label, _, getter in TARGETS}
    found.update({f"Cargo.lock [{k}]": v for k, v in _read_lock_versions(ROOT / "Cargo.lock").items()})
    return found


def cmd_check() -> int:
    found = current_versions()
    width = max(len(k) for k in found)
    for label, value in found.items():
        print(f"  {label:<{width}}  {value}")
    distinct = {v for v in found.values() if v is not None}
    missing = [k for k, v in found.items() if v is None]
    if missing:
        print(f"\nFAIL: version not found in: {', '.join(missing)}")
        return 1
    if len(distinct) != 1:
        print(f"\nFAIL: version metadata disagrees across files: {sorted(distinct)}")
        print("Run: python scripts/set_release_version.py vX.Y.Z")
        return 1
    print(f"\nOK: all {len(found)} version fields agree ({distinct.pop()}).")
    return 0


def cmd_set(version: str) -> int:
    # Pre-flight: confirm every field is locatable BEFORE writing any of them.
    # Writing as we go would leave the tree half-bumped if a later file failed
    # to parse, which is the exact inconsistency this script exists to prevent.
    unreadable = [label for label, _, getter in TARGETS if getter() is None]
    unreadable += [
        f"Cargo.lock [{k}]" for k, v in _read_lock_versions(ROOT / "Cargo.lock").items() if v is None
    ]
    if unreadable:
        print(f"ERROR: cannot locate the version field in: {', '.join(unreadable)}", file=sys.stderr)
        print("Nothing was written.", file=sys.stderr)
        return 1

    changed: list[str] = []
    if _write_toml_version(ROOT / "Cargo.toml", "workspace.package", version):
        changed.append("Cargo.toml")
    if _write_toml_version(ROOT / "desktop/src-tauri/Cargo.toml", "package", version):
        changed.append("desktop/src-tauri/Cargo.toml")
    if _write_json_version(ROOT / "desktop/src-tauri/tauri.conf.json", version):
        changed.append("desktop/src-tauri/tauri.conf.json")
    if _write_json_version(ROOT / "desktop/package.json", version):
        changed.append("desktop/package.json")
    if _write_lock_versions(ROOT / "Cargo.lock", version):
        changed.append("Cargo.lock")

    print(f"Set version to {version}")
    for path in changed:
        print(f"  updated  {path}")
    if not changed:
        print("  (already up to date)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("version", nargs="?", help="release tag or version, e.g. v1.2.3 or 1.2.3")
    ap.add_argument("--check", action="store_true", help="verify agreement without writing")
    args = ap.parse_args()

    if args.check:
        return cmd_check()
    if not args.version:
        ap.error("provide a version, or pass --check")

    version = args.version.lstrip("vV")
    if not SEMVER.match(version):
        print(f"ERROR: {args.version!r} is not X.Y.Z.", file=sys.stderr)
        print("Windows MSI/NSIS installers require a plain numeric version.", file=sys.stderr)
        return 2
    return cmd_set(version)


if __name__ == "__main__":
    raise SystemExit(main())
