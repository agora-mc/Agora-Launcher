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
9. Update checks have one core implementation
10. Live instance reads use the canonical manifest loader
11. Game API depends only on serde, semver and thiserror (all dependency sections)
12. Core must not reference the future Minecraft package's modules or crate
"""

import json
import os
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

DESKTOP_SRC = REPO_ROOT / "desktop" / "src-tauri" / "src"
CORE_SRC = REPO_ROOT / "crates" / "agora-core" / "src"
CLI_SRC = REPO_ROOT / "crates" / "agora" / "src"
# Game packages hold game logic that used to live in core; the rules that
# scanned core scan them too, or code moving out of core would escape them.
MC_SRC = REPO_ROOT / "crates" / "agora-game-minecraft" / "src"

CORE_CARGO = REPO_ROOT / "crates" / "agora-core" / "Cargo.toml"
GAME_API_CARGO = REPO_ROOT / "crates" / "agora-game-api" / "Cargo.toml"

# ---- Documented thin adapter modules ---------------------------------------
# These modules share a name with a crate/agora-core module but are explicitly
# allowed because they only re-export types and/or bridge AppHandle -> Ctx.
# They are NOT duplicates of core business logic.
THIN_ADAPTER_MODULES: set[str] = {
    "crash_export",
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

    core_files = {
        p.stem
        for root in (CORE_SRC, MC_SRC)
        if root.exists()
        for p in root.rglob("*.rs")
        if p.stem not in ("lib", "mod")
    }

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
        # ...and single-name re-exports, e.g.
        # `pub use agora_game_minecraft::loader_manifests;`
        for m in re.finditer(r'^\s*pub\s+use\s+(?:\w+::)+(\w+)\s*;', text, re.MULTILINE):
            declared.add(m.group(1))

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
UPDATE_CHECK_OWNER_DIR = MC_SRC

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
    `agora_game_minecraft::update_cache::check_single_instance_updates_with`.
    """
    search_roots = [DESKTOP_SRC, CLI_SRC, CORE_SRC, MC_SRC]
    hits: list[str] = []
    for root in search_roots:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            if path.parent == UPDATE_CHECK_OWNER_DIR and path.name == UPDATE_CHECK_OWNER:
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
            "Update-check logic found outside crates/agora-game-minecraft/src/"
            f"{UPDATE_CHECK_OWNER} — there must be exactly one implementation, "
            "or the update badge and the update panel can disagree. Call "
            "agora_game_minecraft::update_cache::check_single_instance_updates_with "
            "instead"
        )
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print(
            "OK: update-check matching rules live only in "
            f"agora-game-minecraft/src/{UPDATE_CHECK_OWNER}"
        )


# ---------------------------------------------------------------------------
# 10. Single InstanceManifest loader (lazy backfill)
# ---------------------------------------------------------------------------

# Only helpers.rs may deserialize an InstanceManifest from disk. Every other
# site must go through helpers::read_manifest so that heal_pack_managed and
# the Unknown-origin synthesis happen exactly once, lazily, and without a
# startup mass-rewrite. See helpers.rs: read_manifest.
INSTANCE_MANIFEST_OWNER = "helpers.rs"
ALLOW_RAW_MANIFEST_FILES = {
    # helpers.rs itself is the canonical loader
    CORE_SRC / "helpers.rs",
    # models.rs contains the struct definition and tests that deserialize
    # legacy JSON fixtures (e.g., legacy_manifest_json) — not instance files
    CORE_SRC / "models.rs",
    # snapshot.rs reads SnapshotManifest, not InstanceManifest; allow to avoid
    # false positives on similar names. If it ever reads InstanceManifest from
    # an archive, that is intentionally exempt (archived manifest, not live).
    CORE_SRC / "snapshot.rs",
}
# Pattern for raw deserialization of an InstanceManifest
INSTANCE_MANIFEST_PATTERNS = [
    re.compile(r"InstanceManifest"),
]


def check_instance_manifest_raw() -> None:
    """Fail if anything outside helpers.rs deserializes an InstanceManifest raw.

    The healed value (pack_managed, Unknown origin) must exist in memory after
    any read, but the file on disk must not be rewritten until the next write.
    That is only guaranteed if every live read goes through helpers::read_manifest.
    """
    search_roots = [DESKTOP_SRC, CLI_SRC, CORE_SRC, MC_SRC]
    hits: list[str] = []
    for root in search_roots:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            if path in ALLOW_RAW_MANIFEST_FILES:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            lines = text.splitlines()
            for lineno, line in enumerate(lines, 1):
                stripped = line.strip()
                if stripped.startswith("//") or stripped.startswith("///"):
                    continue
                # Allow an explicit escape hatch for archived manifests. rustfmt
                # moves a trailing comment onto its own line inside a multi-line
                # call, so look at a small window rather than just this line —
                # otherwise the hatch silently stops working after a reformat.
                window = lines[max(0, lineno - 2) : lineno + 2]
                if any("allow-raw-instance-manifest" in entry for entry in window):
                    continue
                # Any mention of InstanceManifest near a serde_json
                # deserialization is raw. The deserialization is frequently on
                # the *next* line, because rustfmt breaks
                #   let mut manifest: InstanceManifest =
                #       serde_json::from_str(&text)...
                # across two lines. Matching only the annotation's own line
                # missed 28 real sites, so scan a small window.
                if "InstanceManifest" not in line:
                    continue
                window = chr(10).join(lines[lineno - 1 : lineno + 2])
                if "from_str" in window or "from_slice" in window or "from_reader" in window:
                    rel = path.relative_to(REPO_ROOT)
                    hits.append(f"  {rel}:{lineno}: {stripped}")
                    continue
    if hits:
        err(
            "Raw InstanceManifest deserialization found outside "
            f"{INSTANCE_MANIFEST_OWNER} — use helpers::read_manifest so that "
            "heal_pack_managed and the Unknown-origin synthesis happen lazily. "
            "If this is an archived manifest (e.g., snapshot restore), add "
            "`// allow-raw-instance-manifest` to the line."
        )
        for h in hits:
            print(h, file=sys.stderr)
    else:
        print(f"OK: InstanceManifest deserialization only in {INSTANCE_MANIFEST_OWNER} (and allowed tests)")


# ---------------------------------------------------------------------------
# 11–12. Game package dependency boundaries
# ---------------------------------------------------------------------------

def dependency_tables(manifest: dict):
    """Include development/build and target-specific dependencies, too."""
    for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
        yield from manifest.get(kind, {}).items()
    for target in manifest.get("target", {}).values():
        yield from dependency_tables(target)


def resolved_dependency(name: str, specification, workspace: dict) -> tuple[str, dict]:
    if isinstance(specification, dict) and specification.get("workspace"):
        specification = workspace.get("workspace", {}).get("dependencies", {}).get(name, specification)
    details = specification if isinstance(specification, dict) else {}
    return details.get("package", name), details


def check_game_api_dependencies() -> None:
    allowed = {"serde", "semver", "thiserror"}
    if not GAME_API_CARGO.exists():
        err("agora-game-api Cargo.toml missing")
        return
    try:
        manifest = tomllib.loads(GAME_API_CARGO.read_text(encoding="utf-8"))
        workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        err(f"Cannot inspect agora-game-api dependencies: {exc}")
        return
    forbidden = []
    for alias, specification in dependency_tables(manifest):
        package, details = resolved_dependency(alias, specification, workspace)
        # A local crate named serde is not the approved contract dependency.
        if package not in allowed or "path" in details or "git" in details:
            forbidden.append(alias)
    if forbidden:
        err(f"agora-game-api dependencies must be serde, semver or thiserror only: {sorted(forbidden)}")
    else:
        print("OK: agora-game-api depends only on serde, semver and thiserror")


def is_game_package(name: str, path: str = "") -> bool:
    if name in ("agora-game-api", "agora_game_api"):
        return False
    if name.startswith("agora-game-") or name.startswith("agora_game_"):
        return True
    path_norm = path.replace("\\", "/").rstrip("/")
    if "agora-game-" in path_norm and not path_norm.endswith("agora-game-api"):
        return True
    return False


def check_core_no_game_packages() -> None:
    """Core never references a game package: adapters register packages into
    core's `GameRegistry` through agora-game-api, never the other way round.
    Check renamed dependencies as well as direct/import/include references."""
    hits = []
    if CORE_CARGO.exists():
        try:
            manifest = tomllib.loads(CORE_CARGO.read_text(encoding="utf-8"))
            workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as exc:
            err(f"Cannot inspect core game package dependencies: {exc}")
            return
        for alias, specification in dependency_tables(manifest):
            package, details = resolved_dependency(alias, specification, workspace)
            path_str = details.get("path", "")
            if is_game_package(package, path_str):
                hits.append(f"  {CORE_CARGO.relative_to(REPO_ROOT)}: dependency {alias}")
    pattern = re.compile(r"\bagora_game_(?!api\b)\w+\b|agora-game-(?!api\b)[\w-]+")
    # Tests and build scripts are part of core's dependency boundary too.
    for path in sorted(CORE_CARGO.parent.rglob("*.rs")):
        # Ignore comments (including multi-line block comments), but retain
        # newlines so reported source locations remain useful.
        source = path.read_text(encoding="utf-8")
        source = re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group().count("\n"), source, flags=re.S)
        for lineno, line in enumerate(source.splitlines(), 1):
            if pattern.search(line.split("//", 1)[0]):
                hits.append(f"  {path.relative_to(REPO_ROOT)}:{lineno}: {line.strip()}")
    if hits:
        err("agora-core references game package modules — packages register through agora-game-api")
        for hit in hits:
            print(hit, file=sys.stderr)
    else:
        print("OK: agora-core does not reference any game package modules")


check_core_no_minecraft_package = check_core_no_game_packages


# ---------------------------------------------------------------------------
# 13. Game packages' remaining dependence on agora-core only shrinks
# ---------------------------------------------------------------------------
# MASTER_SPEC §26.12: a game package depends on agora-game-api, never on
# agora-core. Slice 2 moved Minecraft out of core but left
# agora-game-minecraft using core's types and services; Phase 2 replaces those
# uses with host services as the second game needs them. Until a package's
# budget reaches zero it may depend on agora-core, and its count of
# `agora_core` references may only fall. A lower count must be written back
# into the budget file, so a gain cannot be quietly given back.
GAME_PACKAGE_CORE_BUDGET = REPO_ROOT / "scripts" / "game_package_core_budget.json"
CRATES_DIR = REPO_ROOT / "crates"


def count_core_references(package_dir: Path) -> int:
    """`agora_core` identifiers in a package's Rust code, comments excluded."""
    total = 0
    for sub in ("src", "tests", "benches", "examples"):
        root = package_dir / sub
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            source = path.read_text(encoding="utf-8", errors="replace")
            source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
            for line in source.splitlines():
                total += len(re.findall(r"\bagora_core\b", line.split("//", 1)[0]))
    build = package_dir / "build.rs"
    if build.exists():
        total += len(re.findall(r"\bagora_core\b", build.read_text(encoding="utf-8")))
    return total


def depends_on_core(cargo_toml: Path) -> bool:
    try:
        manifest = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
        workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return True  # unreadable: assume the worst
    for alias, specification in dependency_tables(manifest):
        package, details = resolved_dependency(alias, specification, workspace)
        if package == "agora-core" or details.get("path", "").rstrip("/").endswith("agora-core"):
            return True
    return False


def check_game_package_core_budget() -> None:
    try:
        budget = json.loads(GAME_PACKAGE_CORE_BUDGET.read_text(encoding="utf-8"))
    except FileNotFoundError:
        budget = {}
    except (OSError, ValueError) as exc:
        err(f"Cannot read {GAME_PACKAGE_CORE_BUDGET.name}: {exc}")
        return
    problems = []
    report = []
    packages = sorted(
        p for p in CRATES_DIR.glob("agora-game-*")
        if p.name != "agora-game-api" and (p / "Cargo.toml").exists()
    )
    for package in packages:
        name = package.name
        count = count_core_references(package)
        allowed = budget.get(name)
        if allowed is None:
            if depends_on_core(package / "Cargo.toml") or count:
                problems.append(
                    f"  {name}: depends on agora-core ({count} references) but has no "
                    f"entry in {GAME_PACKAGE_CORE_BUDGET.name}; packages reach core "
                    "through agora-game-api only"
                )
            continue
        if count > allowed:
            problems.append(
                f"  {name}: {count} agora_core references, budget {allowed}; "
                "this budget only shrinks"
            )
        elif count < allowed:
            problems.append(
                f"  {name}: {count} agora_core references, below the budget of "
                f"{allowed}; lower it to {count} in {GAME_PACKAGE_CORE_BUDGET.name}"
            )
        elif count == 0 and depends_on_core(package / "Cargo.toml"):
            problems.append(
                f"  {name}: no agora_core references left; remove the agora-core "
                "dependency and the budget entry"
            )
        else:
            report.append(f"{name} {count}")
    for name in sorted(set(budget) - {p.name for p in packages}):
        problems.append(f"  {name}: budget entry for a package that does not exist")
    if problems:
        err("Game package dependence on agora-core (MASTER_SPEC §26.12)")
        for problem in problems:
            print(problem, file=sys.stderr)
    elif report:
        print("OK: game packages within their agora-core budget: " + ", ".join(report))
    else:
        print("OK: no game package depends on agora-core")


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

    print("--- 10. Single InstanceManifest loader (lazy backfill) ---")
    check_instance_manifest_raw()
    print()

    print("--- 11. Game API contract dependencies ---")
    check_game_api_dependencies()
    print()

    print("--- 12. Core has no game package references ---")
    check_core_no_game_packages()

    print("\n--- 13. Game packages' agora-core budget ---")
    check_game_package_core_budget()
    print()

    if EXIT_CODE == 0:
        print("All architecture boundary checks passed.")
    else:
        print(f"FAIL: {EXIT_CODE} architecture boundary violation(s) found.", file=sys.stderr)

    return EXIT_CODE


if __name__ == "__main__":
    sys.exit(main())
