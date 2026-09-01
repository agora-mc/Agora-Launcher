#!/usr/bin/env python3
"""Architecture boundary enforcement for the Agora monorepo.

No dependencies beyond Python 3 stdlib.

Checks:
1. Desktop uses no raw reqwest::Client::new/builder (must use agora_core::http_client)
2. Desktop uses no rusqlite::Connection/Connection::open (must use core db helpers)
3. CLI uses no rusqlite::Connection/Connection::open
4. Desktop modules don't duplicate core service logic (hard-fail for unknown dupes;
   documented thin adapters allowed, merely listed)
5. No orphaned source modules — every *.rs file under desktop/src-tauri/src/
   (except lib.rs, main.rs) must be declared as `pub mod` in lib.rs
6. No direct HTTP request building in mod_install/modrinth_raw/crash_investigator
7. Core crate has no tauri dependency
8. Tauri binding name-manifest check runs (typed signatures explicitly waived)
"""

import json
import os
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

DESKTOP_SRC = REPO_ROOT / "desktop" / "src-tauri" / "src"
CORE_SRC = REPO_ROOT / "crates" / "agora-core" / "src"
CLI_SRC = REPO_ROOT / "crates" / "agora" / "src"

CORE_CARGO = REPO_ROOT / "crates" / "agora-core" / "Cargo.toml"

# ---- Documented thin adapter modules ---------------------------------------
# These modules share a name with a crate/agora-core module but are explicitly
# allowed because they only re-export types and/or bridge AppHandle -> Ctx.
# They are NOT duplicates of core business logic.
THIN_ADAPTER_MODULES: set[str] = {
    "ai_assistant",
    "auth",
    "crash_diagnostics",
    "crash_investigator",
    "dependency_ops",
    "governance",
    "instances",
    "launcher_profiles",
    "loader_manifests",
    "modrinth_raw",
    "paths",
    "registry",
    "registry_sync",
    "technic",
    "version_cache",
}

# Modules that must not contain HTTP-request-building patterns.
# These should delegate all network access to core service methods.
HTTP_FREE_MODULES: set[str] = {
    "mod_install",
    "modrinth_raw",
    "crash_investigator",
}

EXIT_CODE = 0


def err(msg: str) -> None:
    global EXIT_CODE
    EXIT_CODE = 1
    print(f"ERROR: {msg}", file=sys.stderr)


def warn(msg: str) -> None:
    print(f"NOTICE: {msg}")


# ---------------------------------------------------------------------------
# 1. Raw reqwest::Client in desktop
# ---------------------------------------------------------------------------

def check_reqwest_desktop() -> None:
    pat = re.compile(r'reqwest::Client::(?:new|builder)\b')
    hits: list[str] = []
    for path in sorted(DESKTOP_SRC.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            if pat.search(line):
                rel = path.relative_to(REPO_ROOT)
                hits.append(f"  {rel}:{lineno}: {line.strip()}")
    if hits:
        err("Desktop uses raw reqwest::Client::new/builder — use agora_core::http_client checked helpers")
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print("OK: No raw reqwest::Client in desktop code")


# ---------------------------------------------------------------------------
# 2. rusqlite::Connection in desktop
# ---------------------------------------------------------------------------

def check_rusqlite_desktop() -> None:
    pat = re.compile(r'(?:rusqlite::Connection|Connection::open(?:_with_flags)?)\b')
    hits: list[str] = []
    for path in sorted(DESKTOP_SRC.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            if pat.search(line):
                rel = path.relative_to(REPO_ROOT)
                hits.append(f"  {rel}:{lineno}: {line.strip()}")
    if hits:
        err("Desktop uses rusqlite::Connection directly — use core db helpers via core services")
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print("OK: No direct rusqlite::Connection usage in desktop code")


# ---------------------------------------------------------------------------
# 3. rusqlite::Connection in CLI
# ---------------------------------------------------------------------------

def check_rusqlite_cli() -> None:
    if not CLI_SRC.exists():
        return
    pat = re.compile(r'(?:rusqlite::Connection|Connection::open(?:_with_flags)?)\b')
    hits: list[str] = []
    for path in sorted(CLI_SRC.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            if pat.search(line):
                rel = path.relative_to(REPO_ROOT)
                hits.append(f"  {rel}:{lineno}: {line.strip()}")
    if hits:
        err("CLI uses rusqlite::Connection directly — CLI must use core services")
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print("OK: No direct rusqlite::Connection usage in CLI code")


# ---------------------------------------------------------------------------
# 4. Duplicate module names between desktop and core
# ---------------------------------------------------------------------------

def check_duplicate_modules() -> None:
    desktop_mods: set[str] = set()
    lib_rs = DESKTOP_SRC / "lib.rs"
    if lib_rs.exists():
        text = lib_rs.read_text(encoding="utf-8")
        for m in re.finditer(r'^\s*pub\s+mod\s+(\w+)', text, re.MULTILINE):
            desktop_mods.add(m.group(1))

    core_files = {p.stem for p in CORE_SRC.rglob("*.rs") if p.stem != "lib"}

    shared = desktop_mods & core_files
    unknown = shared - THIN_ADAPTER_MODULES

    if unknown:
        err(
            f"Desktop modules {sorted(unknown)} duplicate core modules "
            "but are NOT in the documented thin-adapter allow-list. "
            "Either migrate to a thin adapter pattern or add to THIN_ADAPTER_MODULES"
        )
    elif shared:
        known = shared & THIN_ADAPTER_MODULES
        warn(
            f"Known thin-adapter modules found in both desktop and core: "
            f"{sorted(known)} (allowed)"
        )
    else:
        print("OK: No unexpected module-name duplication between desktop and core")


# ---------------------------------------------------------------------------
# 5. Orphaned source modules (files not declared in lib.rs)
# ---------------------------------------------------------------------------

def check_orphaned_modules() -> None:
    lib_rs = DESKTOP_SRC / "lib.rs"
    declared: set[str] = set()
    if lib_rs.exists():
        text = lib_rs.read_text(encoding="utf-8")
        # Extract names from `pub mod <name>;` lines
        for m in re.finditer(r'^\s*pub\s+mod\s+(\w+)', text, re.MULTILINE):
            declared.add(m.group(1))
        # Extract names from grouped `pub use` re-exports, e.g.
        # `pub use agora_core::{download, error, loader_manifests, models};`
        for m in re.finditer(r'pub\s+use\s+(?:\w+::)*\{([^}]+)\}', text):
            for name in m.group(1).split(","):
                name = name.strip()
                if name:
                    declared.add(name)

    found_files: set[str] = set()
    for p in DESKTOP_SRC.rglob("*.rs"):
        if p.name in ("lib.rs", "main.rs", "mod.rs"):
            continue
        found_files.add(p.stem)

    orphaned = found_files - declared
    if orphaned:
        err(
            f"Source files present on disk but missing from lib.rs `pub mod` "
            f"or `pub use` declarations: {sorted(orphaned)}"
        )
    else:
        print("OK: All desktop source files are declared in lib.rs")


# ---------------------------------------------------------------------------
# 6. HTTP request building in known adapter modules
# ---------------------------------------------------------------------------

def check_http_free_modules() -> None:
    suspicious_pats = [
        re.compile(r'\.(?:get|post|put|delete|patch|head|options)\s*\(\s*"https?://'),
        re.compile(r'\.send\s*\(\s*\)'),
        re.compile(r'reqwest::'),
    ]
    hits: list[str] = []
    for mod_name in HTTP_FREE_MODULES:
        mod_path = DESKTOP_SRC / f"{mod_name}.rs"
        if not mod_path.exists():
            continue
        text = mod_path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("//") or stripped.startswith("#["):
                continue
            for pat in suspicious_pats:
                if pat.search(stripped):
                    rel = mod_path.relative_to(REPO_ROOT)
                    hits.append(f"  {rel}:{lineno}: {stripped}")
                    break
    if hits:
        err(
            f"HTTP request-building patterns found in modules that should "
            f"only delegate to core ({', '.join(sorted(HTTP_FREE_MODULES))})"
        )
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print(
            f"OK: No HTTP request-building in {', '.join(sorted(HTTP_FREE_MODULES))}"
        )


# ---------------------------------------------------------------------------
# 7. Core crate must not depend on Tauri
# ---------------------------------------------------------------------------

def check_core_no_tauri() -> None:
    if not CORE_CARGO.exists():
        warn("agora-core Cargo.toml not found — cannot check tauri dependency")
        return
    text = CORE_CARGO.read_text(encoding="utf-8")
    if re.search(r'^\s*tauri\b', text, re.MULTILINE):
        err("agora-core Cargo.toml lists 'tauri' as a dependency — core must remain host-independent")
    else:
        print("OK: agora-core has no tauri dependency")


# ---------------------------------------------------------------------------
# 8. Tauri binding manifest check (name-presence only; typed sigs waived)
# ---------------------------------------------------------------------------

def check_tauri_bindings_manifest() -> None:
    """Verify the checked-in tauri-commands.json exists and has the expected
    schema_version, and warn that typed-signature generation is explicitly
    waived (deferred).  Actual diff-checking is delegated to
    check_tauri_bindings.py --check , which runs as a separate CI step."""
    manifest_path = (
        REPO_ROOT / "desktop" / "src-tauri" / "gen" / "tauri-commands.json"
    )
    if not manifest_path.exists():
        err(
            f"Tauri binding manifest not found at {manifest_path.relative_to(REPO_ROOT)}. "
            "Run `python scripts/check_tauri_bindings.py --generate` to create it."
        )
        return
    try:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as exc:
        err(f"Tauri binding manifest is not valid JSON: {exc}")
        return
    if data.get("schema_version") != 1:
        err(f"Unexpected tauri-commands schema version (got {data.get('schema_version')}, expected 1)")
    else:
        print(
            "OK: tauri-commands.json manifest exists (name-presence only; "
            "typed signatures are intentionally waived pending a proc-macro solution)"
        )


# ---------------------------------------------------------------------------
# 9. Single update-check implementation
# ---------------------------------------------------------------------------

# The one module allowed to decide "does this installed item have an update?".
UPDATE_CHECK_OWNER = "update_cache.rs"

# Entry points that answer that question. An adapter reaching for either is
# building a second implementation.
UPDATE_CHECK_PATTERNS = [
    # Only core constructs the IPC payload; adapters pass it through.
    (re.compile(r"\bUpdateInfo\s*\{"), "constructs UpdateInfo"),
    # The bounded resolver call the duplicate used to make directly. Matches
    # the call, not resolver.rs's definition of the method.
    (
        re.compile(r"\.list_curated_versions_for_update\s*\("),
        "calls Resolver::list_curated_versions_for_update",
    ),
]


def check_single_update_check() -> None:
    """Fail if anything outside agora-core's update_cache.rs re-implements the
    update-check matching rules.

    The rules once existed twice: in core's background sweep (which drives the
    instance update badge) and again in the desktop `check_instance_updates`
    command (which drives the update panel). They were identical when written
    and nothing enforced it, so a fix to one would have silently drifted from
    the other and shown up to the user as the badge disagreeing with the panel.
    Both now route through
    `agora_core::update_cache::check_single_instance_updates_with`.
    """
    search_roots = [DESKTOP_SRC, CLI_SRC, CORE_SRC]
    hits: list[str] = []
    for root in search_roots:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            if path.parent == CORE_SRC and path.name == UPDATE_CHECK_OWNER:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for lineno, line in enumerate(text.splitlines(), 1):
                stripped = line.strip()
                if stripped.startswith("//") or stripped.startswith("///"):
                    continue
                for pat, what in UPDATE_CHECK_PATTERNS:
                    if pat.search(stripped):
                        rel = path.relative_to(REPO_ROOT)
                        hits.append(f"  {rel}:{lineno}: {what}: {stripped}")
                        break
    if hits:
        err(
            "Update-check logic found outside crates/agora-core/src/"
            f"{UPDATE_CHECK_OWNER} — there must be exactly one implementation, "
            "or the update badge and the update panel can disagree. Call "
            "agora_core::update_cache::check_single_instance_updates_with "
            "instead"
        )
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print(
            "OK: update-check matching rules live only in "
            f"agora-core/src/{UPDATE_CHECK_OWNER}"
        )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    print("=== Architecture Boundary Checks ===\n")

    print("--- 1. Raw reqwest::Client in desktop ---")
    check_reqwest_desktop()
    print()

    print("--- 2. rusqlite::Connection in desktop ---")
    check_rusqlite_desktop()
    print()

    print("--- 3. rusqlite::Connection in CLI ---")
    check_rusqlite_cli()
    print()

    print("--- 4. Duplicate module names (desktop <-> core) ---")
    check_duplicate_modules()
    print()

    print("--- 5. Orphaned source modules in desktop ---")
    check_orphaned_modules()
    print()

    print("--- 6. HTTP request-building in adapter modules ---")
    check_http_free_modules()
    print()

    print("--- 7. Core crate tauri dependency ---")
    check_core_no_tauri()
    print()

    print("--- 8. Tauri binding name manifest ---")
    check_tauri_bindings_manifest()
    print()

    print("--- 9. Single update-check implementation ---")
    check_single_update_check()
    print()

    if EXIT_CODE == 0:
        print("All architecture boundary checks passed.")
    else:
        print(f"FAIL: {EXIT_CODE} architecture boundary violation(s) found.", file=sys.stderr)

    return EXIT_CODE


if __name__ == "__main__":
    sys.exit(main())
