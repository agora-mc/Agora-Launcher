#!/usr/bin/env python3
"""Integration tests for compiler/compile.py — DB structure verification.

Runs the compiler once (via subprocess) in setUpClass, then opens the
resulting registry.db with stdlib sqlite3 and verifies structure and
content.

Run with:  python compiler/test_compile_integration.py -v
"""

from __future__ import annotations

import contextlib
import json
import os
import re
import pathlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


# ---------------------------------------------------------------------------
# Subprocess compilation
# ---------------------------------------------------------------------------

class _CompileFixtures(unittest.TestCase):
    """Shared fixture: compile the registry once, then open the DB."""

    db_path = os.path.join(os.path.dirname(os.path.dirname(__file__)), "registry.db")

    @classmethod
    def setUpClass(cls):
        repo_root = os.path.dirname(os.path.dirname(__file__))
        compile_script = os.path.join(repo_root, "compiler", "compile.py")
        result = subprocess.run(
            [
                sys.executable,
                compile_script,
                "--skip-sign",
                "--governance-mode",
                "off",
                "--no-governance-write",
            ],
            cwd=repo_root,
            capture_output=True,
            text=True,
            timeout=120,
        )
        cls._compile_result = result

    @classmethod
    def tearDownClass(cls):
        pass  # leave DB for inspection

    def _open_db(self):
        return sqlite3.connect(self.db_path)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestCompileExitCode(_CompileFixtures):
    """Test 1: compiler exits cleanly."""

    def test_compile_exits_zero(self):
        """compile.py --skip-sign should return exit code 0."""
        self.assertEqual(self._compile_result.returncode, 0,
                         f"compile.py failed: {self._compile_result.stderr}")


class TestRegistryDbExists(_CompileFixtures):
    """Test 2: registry.db is produced."""

    def test_registry_db_exists(self):
        """After compilation, registry.db should exist in the repo root."""
        self.assertTrue(os.path.exists(self.db_path),
                        f"registry.db not found at {self.db_path}")


class TestRegistryItemsPopulated(_CompileFixtures):
    """Test 3: registry_items has rows."""

    def test_registry_items_populated(self):
        """SELECT COUNT(*) FROM registry_items > 0."""
        conn = self._open_db()
        try:
            count = conn.execute("SELECT COUNT(*) FROM registry_items").fetchone()[0]
            self.assertGreater(count, 0,
                               "registry_items table is empty after compilation")
        finally:
            conn.close()


class TestKnownConflictsPopulated(_CompileFixtures):
    """Test 4: known_conflicts has exactly 2 rows."""

    def test_known_conflicts_populated(self):
        """known_conflicts should have 2 entries (optifine↔sodium, optifine↔rubidium)."""
        conn = self._open_db()
        try:
            count = conn.execute("SELECT COUNT(*) FROM known_conflicts").fetchone()[0]
            self.assertEqual(count, 2,
                             f"Expected 2 known_conflicts, got {count}")
        finally:
            conn.close()


class TestModManualDependenciesPopulated(_CompileFixtures):
    """Test 5: mod_manual_dependencies has >= 1 row."""

    def test_mod_manual_dependencies_populated(self):
        """mod_manual_dependencies should have at least 1 entry (fabric-api)."""
        conn = self._open_db()
        try:
            count = conn.execute(
                "SELECT COUNT(*) FROM mod_manual_dependencies"
            ).fetchone()[0]
            self.assertGreaterEqual(count, 1,
                                    "mod_manual_dependencies is empty")
        finally:
            conn.close()


class TestModJarAliasesPopulated(_CompileFixtures):
    """Test 6: mod_jar_aliases has exactly 2 rows."""

    def test_mod_jar_aliases_populated(self):
        """mod_jar_aliases should have >= 2 entries (fabric, fabric_api + sub-module aliases)."""
        conn = self._open_db()
        try:
            count = conn.execute(
                "SELECT COUNT(*) FROM mod_jar_aliases"
            ).fetchone()[0]
            self.assertGreaterEqual(count, 2,
                                    f"Expected >= 2 mod_jar_aliases, got {count}")
        finally:
            conn.close()


class TestCrashSignaturesPopulated(_CompileFixtures):
    """Test 7: crash_signatures has >= 3 rows."""

    def test_crash_signatures_populated(self):
        """crash_signatures should have at least 3 entries."""
        conn = self._open_db()
        try:
            count = conn.execute(
                "SELECT COUNT(*) FROM crash_signatures"
            ).fetchone()[0]
            self.assertGreaterEqual(count, 3,
                                    f"Expected >= 3 crash_signatures, got {count}")
        finally:
            conn.close()


class TestAuditLogPopulated(_CompileFixtures):
    """Test 8: audit_log has rows."""

    def test_audit_log_populated(self):
        """audit_log should have at least 1 entry."""
        conn = self._open_db()
        try:
            count = conn.execute(
                "SELECT COUNT(*) FROM audit_log"
            ).fetchone()[0]
            self.assertGreater(count, 0,
                               "audit_log table is empty")
        finally:
            conn.close()


class TestSchemaVersion(_CompileFixtures):
    """Test 9: the compiled db records the compiler's own schema version."""

    def test_schema_version(self):
        """The stored version must track compile.SCHEMA_VERSION.

        Derived rather than pinned to a literal: a hardcoded expectation here
        fails on every legitimate schema bump and says nothing about whether
        the db actually matches the compiler that wrote it.
        """
        # Read the constant from source rather than importing compile.py:
        # importing it is import-path dependent (works under `discover -s
        # compiler`, not under a dotted module path) and pulls in the whole
        # compiler for one integer.
        repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        source = pathlib.Path(repo_root, "compiler", "compile.py").read_text(encoding="utf-8")
        match = re.search(r"^SCHEMA_VERSION\s*=\s*(\d+)", source, re.MULTILINE)
        self.assertIsNotNone(match, "SCHEMA_VERSION not found in compile.py")
        expected = int(match.group(1))

        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT version FROM schema_version"
            ).fetchone()
            self.assertIsNotNone(row, "schema_version table has no rows")
            self.assertEqual(
                row[0],
                expected,
                f"Expected schema_version={expected}, got {row[0]}",
            )
        finally:
            conn.close()


class TestRegistryItemsRequiredFields(_CompileFixtures):
    """Test 10: registry_items rows have non-null required fields."""

    def test_registry_items_have_required_fields(self):
        """SELECT id, name, content_type LIMIT 1 — all must be non-null."""
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT id, name, content_type FROM registry_items LIMIT 1"
            ).fetchone()
            self.assertIsNotNone(row, "registry_items is empty")
            for col_idx, col_name in enumerate(("id", "name", "content_type")):
                self.assertIsNotNone(row[col_idx],
                                     f"id={row[0]}: {col_name} is NULL")
        finally:
            conn.close()


class TestPackIdentity(_CompileFixtures):
    def test_pack_uses_canonical_registry_identity(self):
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT id, content_type FROM registry_items WHERE id = ?",
                ("optimized-survival",),
            ).fetchone()
            self.assertEqual(row, ("optimized-survival", "pack"))
        finally:
            conn.close()

    def test_pack_mod_rows_reference_canonical_id(self):
        conn = self._open_db()
        try:
            count = conn.execute(
                "SELECT COUNT(*) FROM pack_mods WHERE pack_id = ?",
                ("optimized-survival",),
            ).fetchone()[0]
            self.assertGreater(count, 0)
        finally:
            conn.close()


class TestPackVersions(_CompileFixtures):
    def test_locked_release_rows_are_compiled(self):
        conn = self._open_db()
        try:
            releases = conn.execute(
                "SELECT version, minecraft_version, loader FROM pack_versions WHERE pack_id = ?",
                ("optimized-survival",),
            ).fetchall()
            self.assertGreater(len(releases), 0)
            unpinned = conn.execute(
                "SELECT COUNT(*) FROM pack_version_mods WHERE pack_id = ? AND version = ''",
                ("optimized-survival",),
            ).fetchone()[0]
            self.assertEqual(unpinned, 0)
        finally:
            conn.close()

    def test_modrinth_sourced_entries_keep_their_project_id(self):
        conn = self._open_db()
        try:
            missing = conn.execute(
                "SELECT COUNT(*) FROM pack_mods WHERE source = 'modrinth_id' AND modrinth_id IS NULL"
            ).fetchone()[0]
            self.assertEqual(missing, 0)
        finally:
            conn.close()


class TestFabricApiAliases(_CompileFixtures):
    """Test 11: fabric-api has both 'fabric' and 'fabric_api' aliases."""

    def test_fabric_api_has_aliases(self):
        """mod_jar_aliases for fabric-api should include both 'fabric' and 'fabric_api'."""
        conn = self._open_db()
        try:
            aliases = sorted(
                row[0] for row in conn.execute(
                    "SELECT alias FROM mod_jar_aliases WHERE registry_id = ?",
                    ("fabric-api",)
                ).fetchall()
            )
            self.assertIn("fabric", aliases)
            self.assertIn("fabric_api", aliases)
            # The manifest has been expanded (per §19.5) to list all sub-module
            # aliases for cross-source jar-metadata matching, so there are more
            # than just these two canonical aliases.
            self.assertGreaterEqual(len(aliases), 2,
                                    f"Expected >= 2 aliases for fabric-api, got {len(aliases)}")
        finally:
            conn.close()


class TestFabricApiManualDeps(_CompileFixtures):
    """Test 12: fabric-api manual deps contain 'fabricloader'."""

    def test_fabric_api_has_manual_deps(self):
        """mod_manual_dependencies for fabric-api should reference fabricloader."""
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT required_json FROM mod_manual_dependencies WHERE item_id = ?",
                ("fabric-api",)
            ).fetchone()
            self.assertIsNotNone(row,
                                 "fabric-api has no entry in mod_manual_dependencies")
            required = json.loads(row[0])
            # required_json is a flat JSON array of loader names.
            self.assertIn("fabricloader", required,
                          f"Expected 'fabricloader' in {required}")
        finally:
            conn.close()


class TestGovernanceSummaryTable(_CompileFixtures):
    """Test 13: governance_summary table exists."""

    def test_governance_summary_table_exists(self):
        """governance_summary should be queryable."""
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT COUNT(*) FROM governance_summary"
            ).fetchone()
            self.assertIsNotNone(row)
        finally:
            conn.close()


class TestGovernanceEventsTable(_CompileFixtures):
    """Test 14: governance_events table exists."""

    def test_governance_events_table_exists(self):
        """governance_events should be queryable."""
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT COUNT(*) FROM governance_events"
            ).fetchone()
            self.assertIsNotNone(row)
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# Other games do not disturb the pre-existing tables (MASTER_SPEC §26.8)
# ---------------------------------------------------------------------------

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
_WALL_CLOCK_RE = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:\+00:00|Z)?")

SKYRIM_FIXTURE = {
    "id": "crash-logger",
    "name": "CrashLogger",
    "content_type": "mod",
    "author": "Example",
    "license": "MIT",
    "game": "skyrim-se",
    "download_strategy": "github_release",
    "source_identifier": "example-author/crash-logger",
    "sha256": "a" * 64,
    "game_compatibility": [
        {
            "stores": ["steam", "gog"],
            "game_versions": ["1.6.1170.*", "1.6.1179.0"],
            "requires": [{"framework": "skse", "min_version": "2.2.6"}],
            "asset": "CrashLogger-*.7z",
        }
    ],
    "curator_note": "Logs crashes.",
    "base_categories": ["tools"],
}


def _stable(value):
    """A cell as it should be compared: wall-clock stamps (the audit log's, the
    runtime catalog's generated_at) are masked, since they differ on every run."""
    if isinstance(value, str):
        return _WALL_CLOCK_RE.sub("<timestamp>", value)
    return value


def compile_registry_rows(compile_module, registry_root: pathlib.Path, out_dir: pathlib.Path) -> dict:
    """Compile *registry_root* and return every table's rows.

    Hydration (Modrinth, GitHub), the git-log manifest date and the GitHub token
    are stubbed, so the output depends only on the manifests and the compiler.
    """
    stubs = {
        "_hydrate_modrinth_metadata": lambda items: None,
        "_hydrate_modrinth_versions": lambda items: None,
        "_hydrate_version_changelogs": lambda items: [],
        "_load_github_token": lambda: None,
        "manifest_date_added": lambda path: "2026-01-01T00:00:00+00:00",
    }
    with contextlib.ExitStack() as stack:
        for name, fake in stubs.items():
            stack.enter_context(mock.patch.object(compile_module, name, fake))
        compile_module.compile_registry(
            out_dir / "registry.db",
            skip_sign=True,
            no_governance_write=True,
            governance_mode_str="off",
            registry_root=str(registry_root),
        )
    return dump_table_rows(out_dir / "registry.db")


def dump_table_rows(db_path: pathlib.Path) -> dict:
    """Every table's rows, sorted, with wall-clock stamps masked."""
    conn = sqlite3.connect(str(db_path))
    try:
        tables = [
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' "
                "AND name NOT LIKE 'sqlite_%' ORDER BY name"
            )
        ]
        dump = {}
        for table in tables:
            select = ", ".join(
                f'"{row[1]}"' for row in conn.execute(f'PRAGMA table_info("{table}")')
            )
            rows = [
                [_stable(cell) for cell in row]
                for row in conn.execute(f'SELECT {select} FROM "{table}"')
            ]
            dump[table] = sorted(rows, key=lambda row: json.dumps(row))
        return dump
    finally:
        conn.close()


class TestOtherGamesLeavePreExistingTablesAlone(unittest.TestCase):
    """Compiling the real registry/ with a Skyrim entry added changes only the
    new game_catalog_items table. Every pre-existing table is identical."""

    @staticmethod
    def _compile_module():
        sys.path.insert(0, str(REPO_ROOT / "compiler"))
        import compile as compile_module  # noqa: E402

        return compile_module

    def test_skyrim_entry_changes_only_game_catalog_items(self):
        compile_module = self._compile_module()
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            without = root / "without" / "registry"
            with_skyrim = root / "with" / "registry"
            shutil.copytree(REPO_ROOT / "registry", without)
            shutil.copytree(REPO_ROOT / "registry", with_skyrim)
            entry_dir = with_skyrim / "games" / "skyrim-se" / "mods"
            entry_dir.mkdir(parents=True)
            (entry_dir / "crash-logger.json").write_text(
                json.dumps(SKYRIM_FIXTURE), encoding="utf-8"
            )

            before = compile_registry_rows(compile_module, without, root / "out-without")
            after = compile_registry_rows(compile_module, with_skyrim, root / "out-with")

        self.assertEqual(sorted(before), sorted(after), "table sets differ")
        for table in before:
            if table == "game_catalog_items":
                continue
            self.assertEqual(before[table], after[table], f"{table} changed")
        self.assertEqual(before["game_catalog_items"], [])
        self.assertEqual([row[0] for row in after["game_catalog_items"]], ["crash-logger"])

    def test_registry_items_keeps_its_schema_9_columns(self):
        compile_module = self._compile_module()
        with tempfile.TemporaryDirectory() as tmp:
            compile_registry_rows(compile_module, REPO_ROOT / "registry", pathlib.Path(tmp))
            conn = sqlite3.connect(str(pathlib.Path(tmp) / "registry.db"))
            try:
                columns = [row[1] for row in conn.execute("PRAGMA table_info(registry_items)")]
                version = conn.execute("SELECT version FROM schema_version").fetchone()[0]
            finally:
                conn.close()
        self.assertNotIn("game", columns)
        self.assertNotIn("game_compatibility_json", columns)
        self.assertEqual(version, 9)


if __name__ == "__main__":
    unittest.main()
