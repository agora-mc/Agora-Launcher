#!/usr/bin/env python3
"""Deterministic Tauri command binding manifest generator and stale-check.

Scans the Rust `generate_handler![]` registration in `lib.rs` and all
TypeScript `invoke<'name'>(...)` calls under `desktop/src`, then either:

  --generate  : produce `tauri-commands.json` (checked-in manifest)
  --check     : verify the existing manifest matches current sources (CI step)

No heavy dependency is needed — only stdlib and simple regex patterns.  The
generator cannot statically derive Rust return types or TS wrapper *signatures*
without a full type-aware toolchain (e.g. `ts-rs` or a syn-based proc-macro),
so the manifest records only command *name* presence.  A true signature-level
binding generator would require a Rust proc-macro crate or a TypeScript
compiler plugin.

Usage:
    python scripts/check_tauri_bindings.py --generate
    python scripts/check_tauri_bindings.py --check
    python scripts/check_tauri_bindings.py --check --verbose
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

LIB_RS = REPO_ROOT / "desktop" / "src-tauri" / "src" / "lib.rs"
SRC_DIR = REPO_ROOT / "desktop" / "src-tauri" / "src"
TS_ROOT = REPO_ROOT / "desktop" / "src"
MANIFEST = REPO_ROOT / "desktop" / "src-tauri" / "gen" / "tauri-commands.json"

SCHEMA_VERSION = 1


def parse_rust_commands(text: str) -> set[str]:
    """Extract command names from `generate_handler![]` in lib.rs."""
    m = re.search(
        r"generate_handler!\s*\[([^]]*(?:\[[^\]]*\][^]]*)*)\]",
        text,
        re.DOTALL,
    )
    if not m:
        print("ERROR: could not find generate_handler![] block in lib.rs", file=sys.stderr)
        sys.exit(1)

    body = m.group(1).strip()
    commands: set[str] = set()
    if not body:
        return commands
    for line in body.splitlines():
        line = line.strip().rstrip(",")
        # Handlers may be qualified by any adapter module (`commands::foo`,
        # `technic::foo`, ...) or unqualified; take the final path segment.
        m2 = re.match(r"(?:\w+::)*(\w+)$", line)
        if m2:
            commands.add(m2.group(1))
    return commands


def parse_rust_defined_commands(text: str) -> set[str]:
    """Extract function names that have a preceding #[tauri::command]."""
    commands: set[str] = set()
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if re.match(r'^\s*#\s*\[tauri::command', line):
            for j in range(i + 1, min(i + 5, len(lines))):
                m = re.match(r'^\s*pub\s+(?:async\s+)?fn\s+(\w+)', lines[j])
                if m:
                    commands.add(m.group(1))
                    break
    return commands


def parse_ts_invoke_calls(text: str) -> set[str]:
    """Extract command names from `invoke<...>('name'` in tauri.ts."""
    commands: set[str] = set()
    # Walk text: find `invoke`, then scan forward past balanced angle brackets
    # until `>` followed by optional whitespace and `('cmd'`.
    pos = 0
    while True:
        idx = text.find("invoke", pos)
        if idx < 0:
            break
        rest = text[idx + 6:]  # past "invoke"
        # If followed immediately by `<`, scan for matching `>`
        if rest.startswith("<"):
            depth = 1
            j = 1
            while j < len(rest) and depth > 0:
                if rest[j] == "<":
                    depth += 1
                elif rest[j] == ">":
                    depth -= 1
                j += 1
            if depth != 0:
                pos = idx + 1
                continue
            after = rest[j:]
        else:
            after = rest

        # Expect: optional whitespace, `(`, optional whitespace, `'cmd'`
        m = re.match(r"\s*\(\s*'([^']+)'", after)
        if m:
            command_name = m.group(1)
            # Tauri plugin protocol calls are not application commands and
            # have no Rust entry in this application's generate_handler list.
            if not command_name.startswith("plugin:"):
                commands.add(command_name)
            pos = idx + 6 + len(rest.split(">")[0]) + 1
        else:
            pos = idx + 1
    return commands


def build_manifest() -> dict:
    lib_text = LIB_RS.read_text(encoding="utf-8")
    # Every adapter module can define #[tauri::command] fns, so scan the whole
    # src tree rather than commands.rs alone.
    cmds_text = "\n".join(
        path.read_text(encoding="utf-8") for path in sorted(SRC_DIR.rglob("*.rs"))
    )
    ts_text = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(TS_ROOT.rglob("*.ts"))
        if path.is_file()
    )
    ts_text += "\n" + "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(TS_ROOT.rglob("*.tsx"))
        if path.is_file()
    )

    registered = parse_rust_commands(lib_text)
    defined = parse_rust_defined_commands(cmds_text)
    ts_calls = parse_ts_invoke_calls(ts_text)

    rust_only = sorted(registered - ts_calls)
    ts_only = sorted(ts_calls - registered)
    defined_only = sorted(defined - registered)

    return {
        "schema_version": SCHEMA_VERSION,
        "generator": "scripts/check_tauri_bindings.py",
        "limitation": (
            "Name-presence only, not type signatures. A true generator would "
            "need a Rust proc-macro crate or TypeScript compiler plugin."
        ),
        "summary": {
            "registered_rust": len(registered),
            "defined_rust": len(defined),
            "ts_wrappers": len(ts_calls),
            "rust_only": len(rust_only),
            "ts_only": len(ts_only),
            "defined_not_registered": len(defined_only),
        },
        "commands": {
            "registered": sorted(registered),
            "defined": sorted(defined),
            "ts_wrappers": sorted(ts_calls),
            "missing_ts_wrapper": rust_only,
            "missing_rust_command": ts_only,
            "defined_not_registered": defined_only,
        },
    }


def check_manifest(manifest: dict, verbose: bool = False) -> int:
    fresh = build_manifest()
    errors = 0

    old_commands = manifest.get("commands", {})
    new_commands = fresh.get("commands", {})

    def report(section: str, label: str, key: str):
        nonlocal errors
        old_set = set(old_commands.get(key, []))
        new_set = set(new_commands.get(key, []))
        if old_set != new_set:
            added = new_set - old_set
            removed = old_set - new_set
            if added:
                print(f"ERROR [{section}] {label} added (not in manifest): {sorted(added)}", file=sys.stderr)
                errors += 1
            if removed:
                print(f"ERROR [{section}] {label} removed (in manifest but not in sources): {sorted(removed)}", file=sys.stderr)
                errors += 1

    report("binding", "Missing TS wrapper", "missing_ts_wrapper")
    report("binding", "Missing Rust command", "missing_rust_command")
    report("completeness", "Defined but not registered", "defined_not_registered")

    old_registered = set(old_commands.get("registered", []))
    new_registered = set(new_commands["registered"])
    if old_registered != new_registered:
        added = new_registered - old_registered
        removed = old_registered - new_registered
        if added:
            print(f"ERROR [registration] Commands added to generate_handler: {sorted(added)}", file=sys.stderr)
        if removed:
            print(f"ERROR [registration] Commands removed from generate_handler: {sorted(removed)}", file=sys.stderr)
        errors += bool(added) + bool(removed)

    if errors == 0:
        print("OK: All Tauri command bindings are consistent.")
    else:
        print(f"FAIL: {errors} binding error(s) found.", file=sys.stderr)

    if verbose or errors:
        summary = fresh["summary"]
        print(f"  Registered: {summary['registered_rust']} Rust commands")
        print(f"  Defined:    {summary['defined_rust']} Rust #[tauri::command] fns")
        print(f"  TS wrappers: {summary['ts_wrappers']} invoke<>() calls")
        print(f"  Rust-only:   {summary['rust_only']} (need TS wrapper)")
        print(f"  TS-only:     {summary['ts_only']} (need Rust command)")
        print(f"  Defined-not-registered: {summary['defined_not_registered']} (dead #[tauri::command])")

    return errors


def main() -> int:
    if len(sys.argv) < 2:
        print("Usage: python check_tauri_bindings.py --generate|--check [--verbose]", file=sys.stderr)
        return 1

    mode = sys.argv[1]
    verbose = "--verbose" in sys.argv

    if mode == "--generate":
        manifest = build_manifest()
        MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"Generated {MANIFEST}")
        summary = manifest["summary"]
        print(f"  {summary['registered_rust']} registered, {summary['ts_wrappers']} TS wrappers, "
              f"{summary['rust_only']} missing TS, {summary['ts_only']} missing Rust")
        return 0

    if mode == "--check":
        if not MANIFEST.exists():
            print(f"ERROR: {MANIFEST} not found. Run --generate first.", file=sys.stderr)
            return 1
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        return check_manifest(manifest, verbose)

    print(f"ERROR: unknown mode '{mode}'. Use --generate or --check.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
