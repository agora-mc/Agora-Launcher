#!/usr/bin/env python3
"""Integration tests for compiler/compile.py — DB structure verification.

Runs the compiler once (via subprocess) in setUpClass, then opens the
resulting registry.db with stdlib sqlite3 and verifies structure and
content.

Run with:  python compiler/test_compile_integration.py -v
"""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
import unittest


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
    """Test 9: schema_version is 8."""

    def test_schema_version(self):
        """SELECT version FROM schema_version should return 8."""
        conn = self._open_db()
        try:
            row = conn.execute(
                "SELECT version FROM schema_version"
            ).fetchone()
            self.assertIsNotNone(row, "schema_version table has no rows")
            self.assertEqual(row[0], 8,
                             f"Expected schema_version=8, got {row[0]}")
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


if __name__ == "__main__":
    unittest.main()
