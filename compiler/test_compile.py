#!/usr/bin/env python3
"""Standalone unit tests for compiler/compile.py pure functions.

Run with:  python compiler/test_compile.py
No pytest dependency â€” uses only stdlib (unittest, sys, tempfile, json, os, re).

Covers: validate_sha256, _get_registry_repo, _load_poll_blacklist,
        _extract_mod_id, _extract_review_text, _scrub_review_text,
        and regex DoS protections via insert_crash_signature.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import logging
import os
import re
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

# Ensure we can import the compiler module from the repo root.
sys.path.insert(0, os.path.dirname(__file__))
import compile as _compile  # noqa: E402


# ---------------------------------------------------------------------------
# pack identity normalization
# ---------------------------------------------------------------------------

class TestNormalizePackIdentity(unittest.TestCase):
    def test_legacy_pack_id_becomes_canonical_id(self):
        item = {"pack_id": "my-pack", "name": "My Pack"}

        result = _compile.normalize_pack_identity(item)

        self.assertEqual(result["id"], "my-pack")
        self.assertEqual(result["content_type"], "pack")

    def test_canonical_pack_is_unchanged(self):
        item = {"id": "my-pack", "content_type": "pack"}

        self.assertEqual(_compile.normalize_pack_identity(item), item)

    def test_matching_alias_is_accepted(self):
        item = {"id": "my-pack", "pack_id": "my-pack"}

        result = _compile.normalize_pack_identity(item)

        self.assertEqual(result["id"], "my-pack")
        self.assertEqual(result["content_type"], "pack")

    def test_mismatched_alias_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "must match"):
            _compile.normalize_pack_identity({"id": "one", "pack_id": "two"})

    def test_pack_alias_cannot_claim_another_content_type(self):
        with self.assertRaisesRegex(ValueError, "content_type 'pack'"):
            _compile.normalize_pack_identity({"pack_id": "my-pack", "content_type": "mod"})

    def test_regular_registry_item_is_unchanged(self):
        item = {"id": "sodium", "content_type": "mod"}

        self.assertEqual(_compile.normalize_pack_identity(item), item)


# ---------------------------------------------------------------------------
# validate_sha256
# ---------------------------------------------------------------------------

class TestDefaultCompatibleVersions(unittest.TestCase):
    def test_pack_defaults_to_its_declared_target(self):
        pack = {"content_type": "pack", "minecraft_version": "1.20.1", "loader": "forge"}
        self.assertEqual(
            _compile.default_compatible_versions(pack),
            [{"mc_version": "1.20.1", "loader": "forge", "mod_version": "latest"}],
        )

    def test_non_pack_keeps_generic_fallback(self):
        mod = {"content_type": "mod", "minecraft_version": "1.20.1", "loader": "forge"}
        self.assertEqual(
            _compile.default_compatible_versions(mod)[0]["mc_version"], "1.21"
        )


class TestValidatePackManifest(unittest.TestCase):
    def _pack(self, **extra):
        pack = {
            "id": "p",
            "content_type": "pack",
            "minecraft_version": "1.21",
            "loader": "fabric",
            "mods": [{"id": "sodium", "status": "required"}],
        }
        pack.update(extra)
        return pack

    def _release(self, **extra):
        release = {
            "version": "1.0.0",
            "minecraft_version": "1.21",
            "loader": "fabric",
            "loader_version": "0.19.5",
            "mods": [{"id": "sodium", "version": "0.6.0", "status": "required"}],
        }
        release.update(extra)
        return release

    def test_flexible_only_pack_is_valid(self):
        _compile.validate_pack_manifest(self._pack())

    def test_locked_release_is_valid(self):
        _compile.validate_pack_manifest(self._pack(versions=[self._release()]))

    def test_locked_release_requires_every_mod_pinned(self):
        release = self._release(mods=[{"id": "sodium", "status": "required"}])
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(self._pack(versions=[release]))

    def test_latest_is_not_a_pin(self):
        release = self._release(mods=[{"id": "sodium", "version": "latest"}])
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(self._pack(versions=[release]))

    def test_duplicate_release_versions_are_rejected(self):
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(
                self._pack(versions=[self._release(), self._release()])
            )

    def test_unknown_status_is_rejected(self):
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(
                self._pack(mods=[{"id": "sodium", "status": "must-have"}])
            )

    def test_modrinth_source_needs_a_project_id(self):
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(
                self._pack(mods=[{"id": "x", "source": "modrinth_id"}])
            )

    def test_release_needs_a_known_loader(self):
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(
                self._pack(versions=[self._release(loader="rift")])
            )

    def test_locked_releases_become_compatible_versions(self):
        pack = self._pack(
            versions=[self._release(version="2.0.0", minecraft_version="1.21.1"), self._release()]
        )
        self.assertEqual(
            [entry["mod_version"] for entry in _compile.default_compatible_versions(pack)],
            ["2.0.0", "1.0.0"],
        )


class TestValidateSha256(unittest.TestCase):
    """Tests for validate_sha256."""

    def test_valid_64_hex(self):
        """A valid 64-char hex string passes and is returned unchanged."""
        result = _compile.validate_sha256("a" * 64)
        self.assertEqual(result, "a" * 64)

    def test_valid_uppercase_hex(self):
        """Uppercase hex is accepted."""
        result = _compile.validate_sha256("A" * 64)
        self.assertEqual(result, "A" * 64)

    def test_valid_mixed_hex(self):
        """Mixed-case hex is accepted."""
        raw = "aB3dEf0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        self.assertEqual(len(raw), 64)
        result = _compile.validate_sha256(raw)
        self.assertEqual(result, raw)

    def test_none_rejected(self):
        """None raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256(None)

    def test_empty_rejected(self):
        """Empty string raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256("")

    def test_short_rejected(self):
        """32-char hex (too short) raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256("a" * 32)

    def test_long_rejected(self):
        """65-char hex (too long) raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256("a" * 65)

    def test_non_hex_rejected(self):
        """64-char string with non-hex chars raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")

    def test_non_string_rejected(self):
        """Non-string type raises SystemExit."""
        with self.assertRaises(SystemExit):
            _compile.validate_sha256(12345)


# ---------------------------------------------------------------------------
# validate_download_strategy
# ---------------------------------------------------------------------------

class TestValidateDownloadStrategy(unittest.TestCase):
    """Tests for validate_download_strategy.

    direct_hash has no resolving API, so the compiler is the only place an
    incomplete entry can be caught before it ships in a signed registry.
    """

    @staticmethod
    def _direct_hash(**overrides):
        item = {
            "id": "self-hosted-mod",
            "download_strategy": "direct_hash",
            "source_identifier": "https://example.com/files/self-hosted-mod-1.2.3.jar",
            "compatible_versions": [
                {"mc_version": "1.21", "loader": "fabric", "mod_version": "1.2.3"}
            ],
        }
        item.update(overrides)
        return item

    def test_known_strategies_pass_through(self):
        # hand-pinned strategies (direct_hash, technic_pack) are validated
        # separately below with their full contract
        for strategy in ("github_release", "modrinth_id", "curated_pack"):
            item = {"id": "x", "download_strategy": strategy}
            self.assertEqual(_compile.validate_download_strategy(item), strategy)

    def test_unknown_strategy_rejected(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy({"id": "x", "download_strategy": "ftp_mirror"})

    def test_missing_strategy_rejected(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy({"id": "x"})

    def test_complete_direct_hash_accepted(self):
        self.assertEqual(
            _compile.validate_download_strategy(self._direct_hash()), "direct_hash"
        )

    def test_direct_hash_requires_https(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._direct_hash(source_identifier="http://example.com/files/mod-1.0.0.jar")
            )

    def test_direct_hash_requires_a_filename_in_the_url(self):
        """The launcher names the file from the URL's last segment."""
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._direct_hash(source_identifier="https://example.com/download?id=12")
            )

    def test_direct_hash_requires_explicit_compatible_versions(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(self._direct_hash(compatible_versions=[]))
        item = self._direct_hash()
        del item["compatible_versions"]
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(item)

    def test_direct_hash_rejects_the_latest_placeholder(self):
        """'latest' is the unhydrated default; it cannot name a pinned file."""
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._direct_hash(
                    compatible_versions=[
                        {"mc_version": "1.21", "loader": "fabric", "mod_version": "latest"}
                    ]
                )
            )

    def test_direct_hash_rejects_incomplete_version_entries(self):
        for entry in (
            {"mc_version": "1.21", "loader": "fabric"},
            {"mc_version": "1.21", "mod_version": "1.0.0"},
            {"loader": "fabric", "mod_version": "1.0.0"},
            "not-an-object",
        ):
            with self.assertRaises(SystemExit):
                _compile.validate_download_strategy(
                    self._direct_hash(compatible_versions=[entry])
                )

    @staticmethod
    def _technic_pack(**overrides):
        item = {
            "id": "technic-promoted-pack",
            "download_strategy": "technic_pack",
            "source_identifier": "http://192.99.59.1/files/pack.zip",
            "compatible_versions": [
                {"mc_version": "1.21.1", "loader": "forge", "mod_version": "1.0.0"}
            ],
        }
        item.update(overrides)
        return item

    def test_complete_technic_pack_accepted(self):
        self.assertEqual(
            _compile.validate_download_strategy(self._technic_pack()), "technic_pack"
        )

    def test_technic_pack_permits_plain_http(self):
        # The curator-pinned SHA-256 is out-of-band, so a plain-HTTP host is
        # acceptable for technic_pack (unlike direct_hash).
        for url in (
            "http://example.com/files/pack.zip",
            "https://example.com/files/pack.zip",
        ):
            self.assertEqual(
                _compile.validate_download_strategy(self._technic_pack(source_identifier=url)),
                "technic_pack",
            )

    def test_technic_pack_rejects_other_schemes(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._technic_pack(source_identifier="ftp://example.com/files/pack.zip")
            )

    def test_technic_pack_requires_a_filename_in_the_url(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._technic_pack(source_identifier="http://example.com/download?id=12")
            )

    def test_technic_pack_requires_explicit_compatible_versions(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(self._technic_pack(compatible_versions=[]))

    def test_technic_pack_rejects_the_latest_placeholder(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._technic_pack(
                    compatible_versions=[
                        {"mc_version": "1.21", "loader": "forge", "mod_version": "latest"}
                    ]
                )
            )

    def test_technic_pack_rejects_incomplete_version_entries(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(
                self._technic_pack(compatible_versions=[{"mc_version": "1.21", "loader": "forge"}])
            )

    @staticmethod
    def _provider_pack(**overrides):
        item = {
            "id": "tekkit-classic",
            "content_type": "pack",
            "download_strategy": "provider_pack",
            "source_identifier": "technic:tekkit@3.1.2",
            "compatible_versions": [
                {"mc_version": "1.2.5", "loader": "forge", "mod_version": "3.1.2"}
            ],
        }
        item.update(overrides)
        return item

    def test_provider_pack_accepts_official_and_plugin_providers(self):
        for identifier in (
            "technic:tekkit@3.1.2",
            "acme.packs/shelf:project:12@build-7",
        ):
            self.assertEqual(
                _compile.validate_download_strategy(
                    self._provider_pack(source_identifier=identifier)
                ),
                "provider_pack",
            )

    def test_provider_pack_requires_a_pinned_version(self):
        for identifier in ("technic:tekkit", "tekkit@3.1.2", "technic:@1", "Technic:x@1"):
            with self.assertRaises(SystemExit, msg=identifier):
                _compile.validate_download_strategy(
                    self._provider_pack(source_identifier=identifier)
                )

    def test_provider_pack_requires_explicit_compatible_versions(self):
        with self.assertRaises(SystemExit):
            _compile.validate_download_strategy(self._provider_pack(compatible_versions=[]))

    def test_provider_pack_needs_no_recipe_and_accepts_none(self):
        _compile.validate_pack_manifest(self._provider_pack())
        with self.assertRaises(SystemExit):
            _compile.validate_pack_manifest(self._provider_pack(mods=[{"id": "sodium"}]))

    def test_provider_pack_is_only_for_packs_and_stands_alone(self):
        with self.assertRaises(SystemExit):
            _compile.normalize_download_sources(self._provider_pack(content_type="mod"))
        combined = self._provider_pack()
        combined["download_sources"] = [
            {"strategy": "provider_pack", "identifier": "technic:tekkit@3.1.2"},
            {"strategy": "modrinth_id", "identifier": "AANobbMI"},
        ]
        with self.assertRaises(SystemExit):
            _compile.normalize_download_sources(combined)
        self.assertEqual(
            _compile.normalize_download_sources(self._provider_pack()),
            [{"strategy": "provider_pack", "identifier": "technic:tekkit@3.1.2"}],
        )


# ---------------------------------------------------------------------------
# _get_registry_repo
# ---------------------------------------------------------------------------

class TestGetRegistryRepo(unittest.TestCase):
    """Tests for _get_registry_repo."""

    def setUp(self):
        self._saved: dict[str, str | None] = {}
        for key in ("AGORA_REGISTRY_REPO", "GITHUB_REPOSITORY"):
            self._saved[key] = os.environ.pop(key, None)

    def tearDown(self):
        for key, val in self._saved.items():
            if val is not None:
                os.environ[key] = val
            else:
                os.environ.pop(key, None)

    def test_env_var_agora_registry_repo(self):
        """AGORA_REGISTRY_REPO takes precedence and is returned."""
        os.environ["AGORA_REGISTRY_REPO"] = "test/repo"
        self.assertEqual(_compile._get_registry_repo(), "test/repo")

    def test_github_fallback(self):
        """When AGORA_REGISTRY_REPO is unset, GITHUB_REPOSITORY is used."""
        os.environ.pop("AGORA_REGISTRY_REPO", None)
        os.environ["GITHUB_REPOSITORY"] = "gh/test"
        self.assertEqual(_compile._get_registry_repo(), "gh/test")

    def test_missing_configuration_fails_closed(self):
        """When both env vars are unset, compilation fails clearly."""
        os.environ.pop("AGORA_REGISTRY_REPO", None)
        os.environ.pop("GITHUB_REPOSITORY", None)
        with self.assertRaisesRegex(RuntimeError, "AGORA_REGISTRY_REPO"):
            _compile._get_registry_repo()

    def test_priority_agora_over_github(self):
        """AGORA_REGISTRY_REPO wins over GITHUB_REPOSITORY when both are set."""
        os.environ["AGORA_REGISTRY_REPO"] = "owner/first"
        os.environ["GITHUB_REPOSITORY"] = "owner/second"
        self.assertEqual(_compile._get_registry_repo(), "owner/first")


# ---------------------------------------------------------------------------
# _load_poll_blacklist
# ---------------------------------------------------------------------------

class TestLoadPollBlacklist(unittest.TestCase):
    """Tests for _load_poll_blacklist."""

    def test_valid_json_returns_lowercase_set(self):
        """Valid JSON with usernames is returned as a lowercase set."""
        blacklist_dir = _compile.REGISTRY_DIR / "governance"
        target = blacklist_dir / "poll_blacklist.json"
        # Back up if exists.
        backup = None
        if target.exists():
            backup = target.read_bytes()
        try:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps({"usernames": ["Alice", "BOB"]}), encoding="utf-8")
            result = _compile._load_poll_blacklist()
            self.assertEqual(result, {"alice", "bob"})
        finally:
            if backup is not None:
                target.write_bytes(backup)
            else:
                target.unlink(missing_ok=True)

    def test_missing_file_returns_empty_set(self):
        """When the file does not exist, returns empty set (no crash)."""
        blacklist_dir = _compile.REGISTRY_DIR / "governance"
        target = blacklist_dir / "poll_blacklist.json"
        backup = None
        if target.exists():
            backup = target.read_bytes()
        try:
            target.unlink(missing_ok=True)
            result = _compile._load_poll_blacklist()
            self.assertEqual(result, set())
        finally:
            if backup is not None:
                target.write_bytes(backup)

    def test_malformed_json_returns_empty_set(self):
        """Invalid JSON returns empty set (no crash)."""
        blacklist_dir = _compile.REGISTRY_DIR / "governance"
        target = blacklist_dir / "poll_blacklist.json"
        backup = None
        if target.exists():
            backup = target.read_bytes()
        try:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("{not valid json!!!", encoding="utf-8")
            result = _compile._load_poll_blacklist()
            self.assertEqual(result, set())
        finally:
            if backup is not None:
                target.write_bytes(backup)
            else:
                target.unlink(missing_ok=True)


# ---------------------------------------------------------------------------
# _extract_mod_id
# ---------------------------------------------------------------------------

class TestExtractModId(unittest.TestCase):
    """Tests for _extract_mod_id."""

    def test_from_realistic_body(self):
        """A review-form body with Mod Registry ID returns the ID."""
        body = (
            "### Mod Registry ID\n"
            "sodium\n"
            "\n"
            "### Your Technical Review\n"
            "Great mod.\n"
        )
        self.assertEqual(_compile._extract_mod_id(body), "sodium")

    def test_none_body(self):
        """None body returns None."""
        self.assertIsNone(_compile._extract_mod_id(None))

    def test_empty_body(self):
        """Empty string body returns None."""
        self.assertIsNone(_compile._extract_mod_id(""))

    def test_no_field_returns_none(self):
        """Body without the Mod Registry ID field returns None."""
        body = "### Feature Request\nAdd mod X.\n"
        self.assertIsNone(_compile._extract_mod_id(body))

    def test_case_insensitive_heading(self):
        """Heading casing is ignored; ID is lowercased."""
        body = "### mod registry ID\n" "CaveClient\n"
        self.assertEqual(_compile._extract_mod_id(body), "caveclient")

    def test_with_crlf(self):
        """Windows CRLF line endings parse correctly."""
        body = "### Mod Registry ID\r\n" "sodium\r\n"
        self.assertEqual(_compile._extract_mod_id(body), "sodium")

    def test_trailing_whitespace_trimmed(self):
        """Trailing whitespace after the ID is trimmed."""
        body = "### Mod Registry ID\n" "sodium   \n"
        self.assertEqual(_compile._extract_mod_id(body), "sodium")


# ---------------------------------------------------------------------------
# _extract_review_text
# ---------------------------------------------------------------------------

class TestExtractReviewText(unittest.TestCase):
    """Tests for _extract_review_text."""

    def test_from_realistic_body(self):
        """Body with review heading â†’ extracted text."""
        body = (
            "### Mod Registry ID\n"
            "sodium\n"
            "\n"
            "### Your Technical Review (50 character minimum)\n"
            "Excellent performance improvement over vanilla rendering.\n"
            "\n"
            "### Additional Comments\n"
            "None.\n"
        )
        result = _compile._extract_review_text(body)
        self.assertIsNotNone(result)
        self.assertIn("Excellent performance improvement", result)

    def test_none_body(self):
        """None body returns None."""
        self.assertIsNone(_compile._extract_review_text(None))

    def test_empty_body(self):
        """Empty string body returns None."""
        self.assertIsNone(_compile._extract_review_text(""))

    def test_no_review_field_returns_none(self):
        """Body without the review field returns None."""
        body = "### Mod Registry ID\n" "sodium\n"
        self.assertIsNone(_compile._extract_review_text(body))

    def test_strips_whitespace(self):
        """Leading/trailing whitespace in the extracted text is stripped."""
        body = "### Your Technical Review\n" "  lots of text  \n"
        result = _compile._extract_review_text(body)
        self.assertEqual(result, "lots of text")


# ---------------------------------------------------------------------------
# _scrub_review_text
# ---------------------------------------------------------------------------

class TestScrubReviewText(unittest.TestCase):
    """Tests for _scrub_review_text."""

    def test_version_begging_filtered(self):
        """Version-begging text is filtered out."""
        passed, cleaned, reason = _compile._scrub_review_text("Please update to 1.21")
        self.assertFalse(passed)
        self.assertEqual(reason, "version-begging")

    def test_legitimate_review_preserved(self):
        """A substantive review passes the scrub pipeline."""
        passed, cleaned, reason = _compile._scrub_review_text(
            "This mod adds great features and runs smoothly."
        )
        self.assertTrue(passed)
        self.assertEqual(reason, "")
        self.assertEqual(cleaned, "This mod adds great features and runs smoothly.")

    def test_empty_praise_filtered(self):
        """Short empty praise is filtered."""
        passed, cleaned, reason = _compile._scrub_review_text("Good mod.")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty-praise")

    def test_empty_text_filtered(self):
        """Empty text is filtered."""
        passed, cleaned, reason = _compile._scrub_review_text("")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty")

    def test_whitespace_only_filtered(self):
        """Whitespace-only text is filtered."""
        passed, cleaned, reason = _compile._scrub_review_text("   \t\n  ")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty")

    def test_strips_clean_text(self):
        """Passed text is stripped of leading/trailing whitespace."""
        passed, cleaned, reason = _compile._scrub_review_text("  hello world  ")
        self.assertTrue(passed)
        self.assertEqual(cleaned, "hello world")


# ---------------------------------------------------------------------------
# Regex DoS protections (via insert_crash_signature)
# ---------------------------------------------------------------------------

class TestRegexDosProtection(unittest.TestCase):
    """Tests for regex DoS protections in insert_crash_signature."""

    def _create_schema(self, conn):
        """Create the crash_signatures table in *conn*."""
        conn.execute("""
            CREATE TABLE IF NOT EXISTS crash_signatures (
                id TEXT PRIMARY KEY,
                name TEXT,
                regex_pattern TEXT,
                solution_markdown TEXT,
                action_button_json TEXT
            )
        """)

    def setUp(self):
        self._conn = _compile.sqlite3.connect(":memory:")
        self._create_schema(self._conn)
        self._rejected: list[int] = [0]

    def tearDown(self):
        self._conn.close()

    def test_long_pattern_rejected(self):
        """A pattern >256 characters is rejected."""
        long_pattern = "a" * 257
        sig = {
            "id": "long_test",
            "name": "Long Pattern",
            "regex_pattern": long_pattern,
            "solution_markdown": "",
            "action_button_json": "[]",
        }
        _compile.insert_crash_signature(self._conn, sig, self._rejected)
        self.assertEqual(self._rejected[0], 1)
        # Should not have been inserted.
        row = self._conn.execute(
            "SELECT id FROM crash_signatures WHERE id = ?", ("long_test",)
        ).fetchone()
        self.assertIsNone(row)

    def test_valid_pattern_accepted(self):
        """A normal short pattern is accepted and inserted."""
        sig = {
            "id": "nullptr_test",
            "name": "Null Pointer",
            "regex_pattern": r"java\.lang\.NullPointerException",
            "solution_markdown": "Check for nulls.",
            "action_button_json": "[]",
        }
        _compile.insert_crash_signature(self._conn, sig, self._rejected)
        self.assertEqual(self._rejected[0], 0)
        row = self._conn.execute(
            "SELECT id, regex_pattern FROM crash_signatures WHERE id = ?",
            ("nullptr_test",),
        ).fetchone()
        self.assertIsNotNone(row)
        self.assertEqual(row[0], "nullptr_test")
        self.assertEqual(row[1], r"java\.lang\.NullPointerException")

    def test_invalid_regex_rejected(self):
        """An invalid regex pattern is rejected."""
        sig = {
            "id": "bad_regex",
            "name": "Bad Regex",
            "regex_pattern": "[invalid(regex",
            "solution_markdown": "",
            "action_button_json": "[]",
        }
        _compile.insert_crash_signature(self._conn, sig, self._rejected)
        self.assertEqual(self._rejected[0], 1)

    def test_exact_256_pattern_accepted(self):
        """A pattern exactly 256 characters is accepted (boundary)."""
        pattern_256 = "a" * 256
        sig = {
            "id": "boundary_test",
            "name": "Boundary",
            "regex_pattern": pattern_256,
            "solution_markdown": "",
            "action_button_json": "[]",
        }
        _compile.insert_crash_signature(self._conn, sig, self._rejected)
        self.assertEqual(self._rejected[0], 0)
        row = self._conn.execute(
            "SELECT id FROM crash_signatures WHERE id = ?", ("boundary_test",)
        ).fetchone()
        self.assertIsNotNone(row)



# ---------------------------------------------------------------------------
# Ported from _test_social_metrics.py (no equivalents in this file)
# ---------------------------------------------------------------------------

class TestParseReaction(unittest.TestCase):
    """Tests for _parse_reaction."""

    def test_parse_reaction_upvote(self):
        """A +1 reaction on the issue itself is an upvote with comment_id=None."""
        obj = {
            "user": {"login": "alice"},
            "content": "+1",
            "created_at": "2026-01-15T12:00:00Z",
        }
        r = _compile._parse_reaction(obj, comment_id=None)
        self.assertIsNotNone(r)
        self.assertEqual(r.user, "alice")
        self.assertTrue(r.is_upvote)
        self.assertIsNone(r.comment_id)
        self.assertEqual(r.timestamp, datetime(2026, 1, 15, 12, 0, 0, tzinfo=timezone.utc))

    def test_parse_reaction_downvote(self):
        """A -1 reaction is a downvote."""
        obj = {
            "user": {"login": "bob"},
            "content": "-1",
            "created_at": "2026-02-20T08:30:00Z",
        }
        r = _compile._parse_reaction(obj, comment_id=42)
        self.assertIsNotNone(r)
        self.assertFalse(r.is_upvote)
        self.assertEqual(r.comment_id, 42)

    def test_parse_reaction_neutral(self):
        """Neutral emoji (laugh, heart, etc.) sets is_upvote to None."""
        for content in ("laugh", "hooray", "confused", "heart", "rocket", "eyes"):
            obj = {
                "user": {"login": "charlie"},
                "content": content,
                "created_at": "2026-03-01T00:00:00Z",
            }
            r = _compile._parse_reaction(obj, comment_id=None)
            self.assertIsNotNone(r)
            self.assertIsNone(r.is_upvote)

    def test_parse_reaction_malformed_no_user(self):
        """Missing user field returns None."""
        obj = {"content": "+1", "created_at": "2026-01-01T00:00:00Z"}
        self.assertIsNone(_compile._parse_reaction(obj, comment_id=None))

    def test_parse_reaction_malformed_no_content(self):
        """Missing content field returns None."""
        obj = {"user": {"login": "dave"}, "created_at": "2026-01-01T00:00:00Z"}
        self.assertIsNone(_compile._parse_reaction(obj, comment_id=None))

    def test_parse_reaction_malformed_no_created_at(self):
        """Missing created_at field returns None."""
        obj = {"user": {"login": "eve"}, "content": "+1"}
        self.assertIsNone(_compile._parse_reaction(obj, comment_id=None))

    def test_parse_reaction_malformed_created_at(self):
        """Unparseable created_at returns None."""
        obj = {
            "user": {"login": "frank"},
            "content": "+1",
            "created_at": "not-a-date",
        }
        self.assertIsNone(_compile._parse_reaction(obj, comment_id=None))

    def test_parse_reaction_user_login_lowercased(self):
        """User login is always lowercased."""
        obj = {
            "user": {"login": "AliceWunderland"},
            "content": "+1",
            "created_at": "2026-01-01T00:00:00Z",
        }
        r = _compile._parse_reaction(obj, comment_id=None)
        self.assertEqual(r.user, "alicewunderland")


class TestUserReactionDataclass(unittest.TestCase):
    """Tests for UserReaction dataclass."""

    def test_user_reaction_dataclass_defaults(self):
        """comment_id defaults to None."""
        ts = datetime(2026, 1, 1, tzinfo=timezone.utc)
        r = _compile.UserReaction(user="test", is_upvote=True, timestamp=ts)
        self.assertIsNone(r.comment_id)

    def test_user_reaction_dataclass_with_comment_id(self):
        """comment_id can be provided explicitly."""
        ts = datetime(2026, 1, 1, tzinfo=timezone.utc)
        r = _compile.UserReaction(user="test", is_upvote=False, timestamp=ts, comment_id=99)
        self.assertEqual(r.comment_id, 99)


class TestModSocialMetrics(unittest.TestCase):
    """Tests for ModSocialMetrics dataclass."""

    def test_mod_social_metrics_defaults(self):
        """reactions defaults to empty list; each instance gets its own list."""
        m1 = _compile.ModSocialMetrics(mod_id="foo", issue_number=1)
        m2 = _compile.ModSocialMetrics(mod_id="bar", issue_number=2)
        self.assertEqual(m1.reactions, [])
        self.assertEqual(m2.reactions, [])
        self.assertIsNot(m1.reactions, m2.reactions)
        m1.reactions.append("x")
        self.assertEqual(m2.reactions, [])

    def test_mod_social_metrics_with_reactions(self):
        """Reactions can be appended after construction."""
        ts = datetime(2026, 1, 1, tzinfo=timezone.utc)
        r = _compile.UserReaction(user="alice", is_upvote=True, timestamp=ts)
        m = _compile.ModSocialMetrics(mod_id="sodium", issue_number=5)
        m.reactions.append(r)
        self.assertEqual(len(m.reactions), 1)
        self.assertEqual(m.reactions[0].user, "alice")


class TestHydrateGithubSocialMetricsLabelGating(unittest.TestCase):
    """The registry-vote label is REQUIRED to harvest reactions (votes)."""

    def _issue(self, num: int, mod_id: str, labels: list[str]) -> dict:
        return {
            "number": num,
            "body": f"### Mod Registry ID\n{mod_id}\n\n",
            "labels": [{"name": name} for name in labels],
            "user": {"login": "alice"},
            "created_at": "2026-01-01T00:00:00Z",
        }

    def test_reactions_only_harvested_from_registry_vote_labeled_issues(self):
        """Votes are harvested only from registry-vote-labeled issues."""
        issues = [
            self._issue(1, "sodium", ["registry-vote"]),
            self._issue(2, "sodium", ["community-review"]),
        ]
        with mock.patch.object(_compile, "_load_github_token", return_value="token"), \
             mock.patch.object(_compile, "_get_registry_repo", return_value="owner/repo"), \
             mock.patch.object(_compile, "_load_poll_blacklist", return_value=set()), \
             mock.patch.object(_compile, "_github_paginate", return_value=issues), \
             mock.patch.object(_compile, "_fetch_reactions_for_issue", return_value=[]) as fetch:
            items = [{"id": "sodium", "name": "Sodium"}]
            _compile._hydrate_github_social_metrics(items)
        called_issue_numbers = [call.args[2] for call in fetch.call_args_list]
        self.assertEqual(called_issue_numbers, [1])

    def test_unlabeled_issue_harvests_no_reactions(self):
        """An issue with neither label must not contribute votes."""
        issues = [self._issue(1, "sodium", [])]
        with mock.patch.object(_compile, "_load_github_token", return_value="token"), \
             mock.patch.object(_compile, "_get_registry_repo", return_value="owner/repo"), \
             mock.patch.object(_compile, "_load_poll_blacklist", return_value=set()), \
             mock.patch.object(_compile, "_github_paginate", return_value=issues), \
             mock.patch.object(_compile, "_fetch_reactions_for_issue", return_value=[]) as fetch:
            items = [{"id": "sodium", "name": "Sodium"}]
            _compile._hydrate_github_social_metrics(items)
        fetch.assert_not_called()


class TestSybilDiversityWeight(unittest.TestCase):
    """Tests for _sybil_diversity_weight."""

    def test_sybil_diversity_weight_single_mod(self):
        """User who only reacted on one mod gets 0.5 weight."""
        self.assertEqual(_compile._sybil_diversity_weight("alice", ["sodium"]), 0.5)

    def test_sybil_diversity_weight_multiple_mods(self):
        """User who reacted on multiple mods gets 1.0 weight."""
        self.assertEqual(_compile._sybil_diversity_weight("alice", ["sodium", "iris"]), 1.0)

    def test_sybil_diversity_weight_repeated_single_mod(self):
        """Repeated reactions on a single mod still count as one distinct mod."""
        self.assertEqual(_compile._sybil_diversity_weight("alice", ["sodium", "sodium", "sodium"]), 0.5)


class TestUserInteractionCounts(unittest.TestCase):
    """Tests for _user_interaction_counts."""

    def test_user_interaction_counts_counts_per_user(self):
        """Counts are correct for shared and exclusive users across mods."""
        ts = datetime(2026, 1, 1, tzinfo=timezone.utc)
        m1 = _compile.ModSocialMetrics(mod_id="sodium", issue_number=1)
        m1.reactions.extend([
            _compile.UserReaction(user="alice", is_upvote=True, timestamp=ts),
            _compile.UserReaction(user="bob", is_upvote=False, timestamp=ts),
        ])
        m2 = _compile.ModSocialMetrics(mod_id="iris", issue_number=2)
        m2.reactions.extend([
            _compile.UserReaction(user="alice", is_upvote=True, timestamp=ts),
            _compile.UserReaction(user="charlie", is_upvote=True, timestamp=ts),
        ])
        by_mod = {"sodium": m1, "iris": m2}
        cache: dict[str, int] = {}
        result = _compile._user_interaction_counts(by_mod, token="", org="", cache=cache)
        self.assertEqual(cache["alice"], 0)
        self.assertEqual(cache["bob"], 0)
        self.assertEqual(cache["charlie"], 0)
        self.assertIs(result, cache)


class TestComputeVelocity(unittest.TestCase):
    """Tests for _compute_velocity."""

    def test_compute_velocity_zero_history_zero_recent(self):
        """With zero history and zero recent, velocity is clamped."""
        now_dt = datetime(2026, 6, 22, tzinfo=timezone.utc)
        velocity, is_anomaly, anomaly_start = _compile._compute_velocity([], [], now_dt)
        self.assertGreaterEqual(velocity, -1.5)
        self.assertLessEqual(velocity, 0.0)
        self.assertFalse(is_anomaly)
        self.assertIsNone(anomaly_start)

    def test_compute_velocity_anomaly_fires_on_large_recent_downvote_burst(self):
        """25 downvotes in the last 6h with minimal historical context fires anomaly."""
        now_dt = datetime(2026, 6, 22, tzinfo=timezone.utc)
        six_h_ago = now_dt - timedelta(hours=6)
        down_ts = [
            now_dt - timedelta(hours=1, minutes=i)
            for i in range(25)
        ]
        historical_ts = [
            now_dt - timedelta(days=3),
            now_dt - timedelta(days=5),
        ]
        down_ts.extend(historical_ts)
        velocity, is_anomaly, anomaly_start = _compile._compute_velocity([], down_ts, now_dt)
        self.assertTrue(is_anomaly)
        self.assertIsNotNone(anomaly_start)
        self.assertAlmostEqual(anomaly_start, six_h_ago, delta=timedelta(minutes=1))
        self.assertGreater(velocity, 0.0)
        self.assertLessEqual(velocity, 10.0)

    def test_compute_velocity_no_anomaly_at_low_recent_count(self):
        """10 downvotes in 6h vs. historical 5/7d: ratio > 5 but recent_count <= 20 -> no anomaly."""
        now_dt = datetime(2026, 6, 22, tzinfo=timezone.utc)
        down_6h = [
            now_dt - timedelta(hours=1, minutes=i)
            for i in range(10)
        ]
        down_7d = [
            now_dt - timedelta(days=d)
            for d in [1, 2, 3, 4, 5]
        ]
        down_ts = down_6h + down_7d
        velocity, is_anomaly, anomaly_start = _compile._compute_velocity([], down_ts, now_dt)
        self.assertFalse(is_anomaly)
        self.assertIsNone(anomaly_start)


class TestRegexFilterComment(unittest.TestCase):
    """Tests for _regex_filter_comment."""

    def test_version_begging_regex_drops(self):
        """Version-begging comments are rejected."""
        passed, reason = _compile._regex_filter_comment("when is 1.21 release?")
        self.assertFalse(passed)
        self.assertEqual(reason, "version-begging")

    def test_empty_praise_regex_drops(self):
        """Empty praise comments are rejected."""
        passed, reason = _compile._regex_filter_comment("nice mod!")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty-praise")

    def test_legit_review_passes_regex(self):
        """A substantive technical review passes regex filters."""
        passed, reason = _compile._regex_filter_comment(
            "This mod significantly improved my framerate from 30 to 120 FPS."
        )
        self.assertTrue(passed)
        self.assertEqual(reason, "")

    def test_empty_text_dropped(self):
        """Empty or whitespace-only text is dropped."""
        passed, reason = _compile._regex_filter_comment("")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty")
        passed, reason = _compile._regex_filter_comment("   ")
        self.assertFalse(passed)
        self.assertEqual(reason, "empty")

    def test_port_to_regex_drops(self):
        """'port to' and 'update to' variants are also dropped."""
        for text in ("update to 1.21?", "port to 1.20 release", "for 1.22"):
            passed, reason = _compile._regex_filter_comment(text)
            self.assertFalse(passed, f"Expected drop for '{text}', got pass")
            self.assertEqual(reason, "version-begging")


class TestNlpFilterComment(unittest.TestCase):
    """Tests for _nlp_filter_comment."""

    def test_nlp_filter_comment_handles_missing_deps_gracefully(self):
        """If profanity-check import fails, _nlp_filter_comment returns (True, "")."""
        try:
            import profanity_check  # noqa: F401
            has_deps = True
        except ImportError:
            has_deps = False
        passed, reason = _compile._nlp_filter_comment("This mod is fantastic and very well made.")
        self.assertTrue(passed)
        self.assertEqual(reason, "")


class TestPass3Constants(unittest.TestCase):
    """Tests for Pass 3 circuit-breaker constants."""

    def test_organic_under_review_threshold_constant(self):
        """ORGANIC_UNDER_REVIEW_THRESHOLD must equal -10."""
        self.assertEqual(_compile.ORGANIC_UNDER_REVIEW_THRESHOLD, -10)

    def test_triage_poll_duration_constant(self):
        """TRIAGE_POLL_DURATION_DAYS must equal 7."""
        self.assertEqual(_compile.TRIAGE_POLL_DURATION_DAYS, 7)


class TestAppendAuditEntry(unittest.TestCase):
    """Tests for _append_audit_entry."""

    def setUp(self):
        # Redirect REGISTRY_DIR at a tempdir: _append_audit_entry writes both
        # audit_log.json AND (on rotation) audit_log_archive.<date>.json under
        # it. Backing up only the former left rotation archives behind in the
        # real, git-tracked registry/governance/, which the governance pipeline
        # reads and which AGENTS.md says must carry only governance-state.json.
        self._tmp = tempfile.TemporaryDirectory()
        self._prev_registry_dir = _compile.REGISTRY_DIR
        _compile.REGISTRY_DIR = Path(self._tmp.name)
        self._audit_path = _compile.REGISTRY_DIR / "governance" / "audit_log.json"

    def tearDown(self):
        _compile.REGISTRY_DIR = self._prev_registry_dir
        self._tmp.cleanup()

    def test_append_audit_entry_creates_file_when_absent(self):
        """_append_audit_entry creates the file if it doesn't exist."""
        if self._audit_path.exists():
            self._audit_path.unlink()
        _compile._append_audit_entry("test_action", "test_details")
        self.assertTrue(self._audit_path.exists())
        with self._audit_path.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
        self.assertIn("entries", data)
        self.assertEqual(len(data["entries"]), 1)
        self.assertEqual(data["entries"][0]["action"], "test_action")
        self.assertEqual(data["entries"][0]["details"], "test_details")
        self.assertIn("timestamp", data["entries"][0])

    def test_append_audit_entry_rotates_at_10000(self):
        """After 10000 entries, adding one more rotates to keep <= 10000."""
        data = {"log_format_version": 1, "entries": [{"timestamp": "2026-01-01T00:00:00Z", "action": f"dummy_{i}", "details": ""} for i in range(10000)]}
        self._audit_path.parent.mkdir(parents=True, exist_ok=True)
        with self._audit_path.open("w", encoding="utf-8") as fh:
            json.dump(data, fh)
        _compile._append_audit_entry("rotate_test", "should rotate")
        with self._audit_path.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
        self.assertLessEqual(len(data["entries"]), 10000)
        self.assertIn("log_format_version", data)


class TestFindTriageDiscussionCategory(unittest.TestCase):
    """Tests for _find_triage_discussion_category."""

    def test_find_triage_discussion_category_returns_none_without_network(self):
        """With a dummy nonexistent repo, the function should return None."""
        result = _compile._find_triage_discussion_category(
            "og-nonexistent-xyz", "og-nonexistent-xyz", token="invalid-token-for-test",
        )
        self.assertIsNone(result)


class TestCreateAdminAlertIssue(unittest.TestCase):
    """Tests for _create_admin_alert_issue."""

    def test_create_admin_alert_issue_failure_logged_not_raised(self):
        """Invalid token should log a warning but not raise."""
        _compile._create_admin_alert_issue(
            "og-nonexistent-xyz", "og-nonexistent-xyz",
            mod_id="test", offending_reactions=[], token="invalid-token",
        )
        self.assertTrue(True)


class TestDiscordAlert(unittest.TestCase):
    """Tests for Discord webhook notification channel."""

    _prev_discord_url: str | None = None

    def setUp(self):
        self._prev_discord_url = os.environ.pop("DISCORD_WEBHOOK_URL", None)

    def tearDown(self):
        if self._prev_discord_url is not None:
            os.environ["DISCORD_WEBHOOK_URL"] = self._prev_discord_url

    def test_load_discord_webhook_url_returns_none_when_unset(self):
        """When DISCORD_WEBHOOK_URL is absent, _load_discord_webhook_url returns None."""
        os.environ.pop("DISCORD_WEBHOOK_URL", None)
        result = _compile._load_discord_webhook_url()
        self.assertIsNone(result)

    def test_load_discord_webhook_url_returns_value_when_set(self):
        """When DISCORD_WEBHOOK_URL is set, _load_discord_webhook_url returns it."""
        os.environ["DISCORD_WEBHOOK_URL"] = "https://discord.com/api/webhooks/test/abc"
        result = _compile._load_discord_webhook_url()
        self.assertEqual(result, "https://discord.com/api/webhooks/test/abc")

    def test_post_discord_alert_is_noop_when_url_unset(self):
        """When DISCORD_WEBHOOK_URL is unset, _post_discord_alert returns without making a network call."""
        os.environ.pop("DISCORD_WEBHOOK_URL", None)
        _compile._post_discord_alert(mod_id="testmod", reason="test", severity="spike")
        self.assertTrue(True)

    @unittest.skipIf(os.environ.get("CI") == "true", "skip network test on CI")
    def test_post_discord_alert_swallows_invalid_webhook_failures(self):
        """An invalid webhook URL must not raise."""
        os.environ["DISCORD_WEBHOOK_URL"] = "https://discord.com/api/webhooks/INVALID/INVALID"
        _compile._post_discord_alert(
            mod_id="testmod",
            reason="test reason",
            severity="spike",
            offending_reactions=[{"user": "bob"}],
        )
        self.assertTrue(True)

    def test_post_discord_alert_accepts_optional_fields(self):
        """Calling _post_discord_alert with optional fields still returns cleanly when no webhook is configured."""
        os.environ.pop("DISCORD_WEBHOOK_URL", None)
        _compile._post_discord_alert(
            mod_id="testmod",
            reason="test reason",
            severity="spike",
            offending_reactions=[{"user": "alice"}],
            admin_alert_issue_url="https://example.com/issues/1",
        )
        self.assertTrue(True)


# ---------------------------------------------------------------------------
# normalize_download_sources
# ---------------------------------------------------------------------------

class TestNormalizeDownloadSources(unittest.TestCase):
    """Tests for normalize_download_sources.

    The ordered source list is what the launcher walks at install time, so a
    manifest that states its sources ambiguously must fail here rather than
    ship a fallback that silently resolves to the wrong file.
    """

    def test_explicit_list_keeps_curator_order(self):
        item = {
            "id": "sodium",
            "download_sources": [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "github_release", "identifier": "CaffeineMC/sodium"},
            ],
        }
        sources = _compile.normalize_download_sources(item)
        self.assertEqual(
            sources,
            [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "github_release", "identifier": "CaffeineMC/sodium"},
            ],
        )

    def test_legacy_columns_are_derived_from_the_preferred_source(self):
        item = {
            "id": "sodium",
            "download_sources": [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "github_release", "identifier": "CaffeineMC/sodium"},
            ],
        }
        _compile.normalize_download_sources(item)
        self.assertEqual(item["download_strategy"], "modrinth_id")
        self.assertEqual(item["source_identifier"], "AANobbMI")

    def test_modrinth_id_is_surfaced_from_the_list(self):
        # Hydration and the website's Modrinth link both read modrinth_id, so a
        # project id stated only inside the list must still reach them.
        item = {
            "id": "iris",
            "download_sources": [
                {"strategy": "github_release", "identifier": "IrisShaders/Iris"},
                {"strategy": "modrinth_id", "identifier": "YL57xq9U"},
            ],
        }
        _compile.normalize_download_sources(item)
        self.assertEqual(item["modrinth_id"], "YL57xq9U")

    def test_legacy_manifest_keeps_its_implicit_modrinth_fallback(self):
        item = {
            "id": "iris",
            "download_strategy": "github_release",
            "source_identifier": "IrisShaders/Iris",
            "modrinth_id": "YL57xq9U",
        }
        self.assertEqual(
            _compile.normalize_download_sources(item),
            [
                {"strategy": "github_release", "identifier": "IrisShaders/Iris"},
                {"strategy": "modrinth_id", "identifier": "YL57xq9U"},
            ],
        )

    def test_modrinth_primary_is_not_duplicated_as_its_own_fallback(self):
        item = {
            "id": "sodium",
            "download_strategy": "modrinth_id",
            "source_identifier": "AANobbMI",
            "modrinth_id": "AANobbMI",
        }
        self.assertEqual(
            _compile.normalize_download_sources(item),
            [{"strategy": "modrinth_id", "identifier": "AANobbMI"}],
        )

    def test_repeated_source_is_dropped(self):
        item = {
            "id": "dup",
            "download_sources": [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
            ],
        }
        self.assertEqual(len(_compile.normalize_download_sources(item)), 1)

    def test_contradicting_legacy_pair_is_rejected(self):
        item = {
            "id": "conflict",
            "download_strategy": "github_release",
            "source_identifier": "owner/repo",
            "download_sources": [{"strategy": "modrinth_id", "identifier": "AANobbMI"}],
        }
        with self.assertRaises(SystemExit):
            _compile.normalize_download_sources(item)

    def test_invalid_entries_are_rejected(self):
        cases = [
            ("empty list", {"id": "x", "download_sources": []}),
            ("not a list", {"id": "x", "download_sources": {"strategy": "modrinth_id"}}),
            (
                "unknown strategy",
                {"id": "x", "download_sources": [{"strategy": "ftp", "identifier": "a"}]},
            ),
            (
                "missing identifier",
                {"id": "x", "download_sources": [{"strategy": "modrinth_id"}]},
            ),
        ]
        for why, item in cases:
            with self.subTest(why=why), self.assertRaises(SystemExit):
                _compile.normalize_download_sources(item)

    def test_a_pinned_fallback_is_held_to_the_pinned_contract(self):
        # The whole point of a fallback is that it runs when the preferred
        # source is down; an unchecked one would only fail then.
        item = {
            "id": "mirrored",
            "download_sources": [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "direct_hash", "identifier": "http://mirror.example.com/mod.jar"},
            ],
            "compatible_versions": [
                {"mc_version": "1.21", "loader": "fabric", "mod_version": "1.0.0"}
            ],
        }
        with self.assertRaises(SystemExit):
            _compile.normalize_download_sources(item)

    def test_a_valid_pinned_fallback_is_accepted(self):
        item = {
            "id": "mirrored",
            "download_sources": [
                {"strategy": "modrinth_id", "identifier": "AANobbMI"},
                {"strategy": "direct_hash", "identifier": "https://mirror.example.com/mod-1.0.0.jar"},
            ],
            "compatible_versions": [
                {"mc_version": "1.21", "loader": "fabric", "mod_version": "1.0.0"}
            ],
        }
        self.assertEqual(len(_compile.normalize_download_sources(item)), 2)


class TestRegexTimeoutBudget(unittest.TestCase):
    """Tests for _test_regex_timeout.

    The budget is meant to measure the *regex*. On Windows it used to time the
    whole subprocess, so Python's interpreter startup was charged to the
    pattern and a linear regex got rejected as catastrophic backtracking
    whenever the build host was busy -- silently dropping a valid crash
    signature from the shipped registry.
    """

    def test_linear_pattern_is_accepted(self):
        # Comfortably longer than the search, far shorter than interpreter
        # startup on Windows: this fails if startup is being counted.
        self.assertTrue(
            _compile._test_regex_timeout(re.compile(r"Mixin apply failed"), timeout_secs=0.5)
        )

    def test_catastrophic_pattern_is_rejected(self):
        # The corpus is 100k 'a' with no 'b', so the engine must try every
        # partition of the run before failing: genuinely exponential. The
        # backtracking is the point -- this pattern is the fixture that proves
        # the timeout guard keeps such a signature out of the shipped registry.
        self.assertFalse(
            _compile._test_regex_timeout(re.compile(r"(a+)+b"))  # codeql[py/redos]
        )


class TestCleanVersionWindow(unittest.TestCase):
    """Tests for _clean_version_window.

    A window narrows a curated conflict to the releases it actually applies to.
    Getting this wrong in the narrowing direction is the dangerous one: the
    launcher would stop warning about a pair a curator vouched for, so anything
    unrecognizable has to collapse to "any version".
    """

    def test_ranges_are_trimmed_and_kept_in_order(self):
        self.assertEqual(
            _compile._clean_version_window(["  >=2.3 ", "<3.0"]),
            [">=2.3", "<3.0"],
        )

    def test_missing_or_wrong_type_is_unconditional(self):
        for raw in (None, "", ">=2.3", {}, 5):
            self.assertEqual(_compile._clean_version_window(raw), [])

    def test_wildcard_collapses_to_unconditional(self):
        self.assertEqual(_compile._clean_version_window(["*"]), [])
        self.assertEqual(_compile._clean_version_window([">=1.0", "*"]), [])

    def test_non_string_members_are_dropped(self):
        self.assertEqual(_compile._clean_version_window([">=1.0", 7, None]), [">=1.0"])


class TestLoadKnownConflicts(unittest.TestCase):
    """Tests for load_known_conflicts, focused on the pair-normalization swap.

    Rows are stored with the lexicographically smaller id in mod_a_id. The
    version windows belong to the mods, so they have to travel with them --
    applying A's window to B would silently mis-scope every reversed entry.
    """

    def _load(self, entries):
        conn = sqlite3.connect(":memory:")
        conn.execute(
            """CREATE TABLE known_conflicts (
                   mod_a_id TEXT, mod_b_id TEXT, severity TEXT,
                   mitigated_by_json TEXT, notes TEXT,
                   mod_a_versions_json TEXT, mod_b_versions_json TEXT,
                   version_grammar TEXT,
                   PRIMARY KEY (mod_a_id, mod_b_id))"""
        )
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        gov = Path(tmp.name) / "governance"
        gov.mkdir(parents=True)
        (gov / "known_conflicts.json").write_text(json.dumps(entries), encoding="utf-8")
        with mock.patch.object(_compile, "REGISTRY_DIR", Path(tmp.name)):
            count = _compile.load_known_conflicts(conn)
        rows = conn.execute(
            "SELECT mod_a_id, mod_b_id, mod_a_versions_json,"
            " mod_b_versions_json, version_grammar FROM known_conflicts"
        ).fetchall()
        return count, rows

    def test_window_follows_its_mod_through_the_swap(self):
        # "zebra" sorts after "alpha", so the pair is stored reversed.
        _, rows = self._load([
            {"a": "zebra", "b": "alpha", "severity": "hard",
             "a_versions": [">=2.3"], "b_versions": ["<1.0"]},
        ])
        self.assertEqual(len(rows), 1)
        mod_a, mod_b, a_json, b_json, _ = rows[0]
        self.assertEqual((mod_a, mod_b), ("alpha", "zebra"))
        self.assertEqual(json.loads(a_json), ["<1.0"])
        self.assertEqual(json.loads(b_json), [">=2.3"])

    def test_absent_windows_store_null(self):
        _, rows = self._load([{"a": "alpha", "b": "zebra", "severity": "hard"}])
        _, _, a_json, b_json, grammar = rows[0]
        self.assertIsNone(a_json)
        self.assertIsNone(b_json)
        self.assertEqual(grammar, "fabric")

    def test_unknown_grammar_falls_back_to_fabric(self):
        _, rows = self._load([
            {"a": "alpha", "b": "zebra", "severity": "hard",
             "version_grammar": "gradle", "a_versions": [">=1"]},
        ])
        self.assertEqual(rows[0][4], "fabric")

    def test_maven_grammar_is_preserved(self):
        _, rows = self._load([
            {"a": "alpha", "b": "zebra", "severity": "hard",
             "version_grammar": "Maven", "a_versions": ["[1.0,2.0)"]},
        ])
        self.assertEqual(rows[0][4], "maven")


# ---------------------------------------------------------------------------
# Catalog entries for other games (MASTER_SPEC §26.8, REGISTRY_CURATION_REFERENCE)
# ---------------------------------------------------------------------------

# A stand-in for a game package. The real Skyrim package declares no frameworks
# yet, so the fixture declares SKSE to exercise a satisfiable requirement.
FIXTURE_PACKAGE = {
    "id": "agora.test-creation",
    "games": [
        {
            "id": "skyrim-se",
            "stores": [{"store": "steam", "product": "1"}, {"store": "gog", "product": "2"}],
            "framework_ids": [],
        }
    ],
    "frameworks": [
        {"id": "skse", "game": "skyrim-se", "name": "SKSE", "version": "2.2.6"},
    ],
}


def _skyrim_github_entry(item_id="crash-logger"):
    return {
        "id": item_id,
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


def _skyrim_direct_entry(item_id="skyrim-archive-mod", url="https://example.com/files/Mod-1.0.zip"):
    return {
        "id": item_id,
        "name": "Archive Mod",
        "content_type": "mod",
        "author": "Example",
        "license": "LicenseRef-Proprietary",
        "game": "skyrim-se",
        "download_strategy": "direct_hash",
        "source_identifier": url,
        "sha256": "b" * 64,
        "game_compatibility": [{"stores": ["steam"], "game_versions": ["1.6.1170.0"]}],
        "curator_note": "",
        "base_categories": ["content"],
    }


def _minecraft_direct_entry(item_id="minecraft-pinned-mod"):
    return {
        "id": item_id,
        "name": "Pinned Mod",
        "content_type": "mod",
        "author": "Example",
        "license": "LicenseRef-Proprietary",
        "download_strategy": "direct_hash",
        "source_identifier": "https://example.com/files/pinned-1.0.0.jar",
        "sha256": "c" * 64,
        "compatible_versions": [
            {"mc_version": "1.21", "loader": "fabric", "mod_version": "1.0.0"},
        ],
        "curator_note": "",
        "base_categories": ["content"],
    }


class OtherGameDeclarationTests(unittest.TestCase):
    """What the game packages declare, read from their JSON."""

    def test_real_packages_declare_skyrim_and_the_tracer_games(self):
        declared = _compile.load_game_declarations()
        self.assertEqual(declared["skyrim-se"].stores, frozenset({"steam", "gog"}))
        self.assertEqual(declared["valheim"].stores, frozenset({"steam"}))
        self.assertEqual(declared["witcher-3"].stores, frozenset({"gog"}))
        self.assertNotIn("minecraft", declared)

    def test_minecraft_cannot_be_declared_by_a_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp) / "package.json"
            package.write_text(json.dumps({"games": [{"id": "minecraft", "stores": []}]}), encoding="utf-8")
            with self.assertRaises(SystemExit) as caught:
                _compile.load_game_declarations([package])
        self.assertIn("minecraft", str(caught.exception.code))

    def test_a_game_declared_twice_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            first, second = Path(tmp) / "a.json", Path(tmp) / "b.json"
            game = {"games": [{"id": "skyrim-se", "stores": []}]}
            first.write_text(json.dumps(game), encoding="utf-8")
            second.write_text(json.dumps(game), encoding="utf-8")
            with self.assertRaises(SystemExit) as caught:
                _compile.load_game_declarations([first, second])
        self.assertIn("skyrim-se", str(caught.exception.code))


class OtherGameEntryTests(unittest.TestCase):
    """Entries for other games compile through the real pipeline. Every rule they
    break is refused, and the refusal names the file and the field."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.registry = self.tmp / "registry"
        self.registry.mkdir()
        self.package = self.tmp / "package.json"
        self.package.write_text(json.dumps(FIXTURE_PACKAGE), encoding="utf-8")
        patches = [
            mock.patch.object(_compile, "REGISTRY_DIR", self.registry),
            mock.patch.object(_compile, "game_package_paths", lambda: [self.package]),
            mock.patch.object(_compile, "_hydrate_modrinth_metadata", lambda items: None),
            mock.patch.object(_compile, "_hydrate_modrinth_versions", lambda items: None),
            mock.patch.object(_compile, "_hydrate_version_changelogs", lambda items: []),
            mock.patch.object(_compile, "_load_github_token", lambda: None),
            mock.patch.object(
                _compile, "manifest_date_added", lambda path: "2026-01-01T00:00:00+00:00"
            ),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def reset_registry(self):
        shutil.rmtree(self.registry)
        self.registry.mkdir()

    def write(self, rel, data):
        path = self.registry / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def skyrim_entry_at(self, data, rel=None):
        return self.write(rel or f"games/skyrim-se/mods/{data['id']}.json", data)

    def compile(self):
        out = self.tmp / "out" / "registry.db"
        _compile.compile_registry(out, skip_sign=True, no_governance_write=True, governance_mode_str="off")
        return out

    def _rows_of(self, out, table):
        conn = sqlite3.connect(str(out))
        conn.row_factory = sqlite3.Row
        try:
            return {row["id"]: dict(row) for row in conn.execute(f"SELECT * FROM {table}")}
        finally:
            conn.close()

    def game_rows(self, out):
        """Entries for other games, from game_catalog_items."""
        return self._rows_of(out, "game_catalog_items")

    def minecraft_rows(self, out):
        """Minecraft entries, from registry_items."""
        return self._rows_of(out, "registry_items")

    def assertRefused(self, *needles):
        """The compile must exit. The refusal, whether raised with its message or
        logged first (the Minecraft URL checks log then exit 1), must name each needle."""
        logged = []
        handler = logging.Handler(level=logging.ERROR)
        handler.emit = lambda record: logged.append(record.getMessage())
        compiler_logger = logging.getLogger("compiler")
        compiler_logger.addHandler(handler)
        try:
            with self.assertRaises(SystemExit) as caught:
                self.compile()
        finally:
            compiler_logger.removeHandler(handler)
        message = " ".join([str(caught.exception.code), *logged])
        for needle in needles:
            self.assertIn(needle, message)
        return message

    # -- valid entries -------------------------------------------------------

    def test_valid_github_and_direct_hash_entries_compile(self):
        self.skyrim_entry_at(_skyrim_github_entry())
        self.skyrim_entry_at(_skyrim_direct_entry())
        out = self.compile()
        rows = self.game_rows(out)

        github = rows["crash-logger"]
        self.assertEqual(github["game"], "skyrim-se")
        self.assertEqual(github["download_strategy"], "github_release")
        self.assertEqual(github["status"], "active")
        self.assertEqual(github["license_id"], "MIT")
        compat = json.loads(github["game_compatibility_json"])
        self.assertEqual(compat[0]["asset"], "CrashLogger-*.7z")
        self.assertEqual(compat[0]["requires"], [{"framework": "skse", "min_version": "2.2.6"}])
        self.assertEqual(
            json.loads(github["download_sources_json"]),
            [{"strategy": "github_release", "identifier": "example-author/crash-logger"}],
        )

        direct = rows["skyrim-archive-mod"]
        self.assertEqual(direct["game"], "skyrim-se")
        self.assertEqual(direct["download_strategy"], "direct_hash")
        self.assertNotIn("asset", json.loads(direct["game_compatibility_json"])[0])

    def test_other_game_entries_stay_out_of_registry_items(self):
        self.skyrim_entry_at(_skyrim_github_entry())
        out = self.compile()
        self.assertNotIn("crash-logger", self.minecraft_rows(out))
        conn = sqlite3.connect(str(out))
        try:
            columns = {row[1] for row in conn.execute("PRAGMA table_info(registry_items)")}
        finally:
            conn.close()
        self.assertNotIn("game", columns)
        self.assertNotIn("game_compatibility_json", columns)

    def test_archive_names_are_accepted_for_direct_hash(self):
        for index, name in enumerate(("Mod-1.0.7z", "Mod-1.0.zip", "Mod-1.0.rar")):
            self.skyrim_entry_at(
                _skyrim_direct_entry(item_id=f"archive-{index}", url=f"https://example.com/files/{name}")
            )
        rows = self.game_rows(self.compile())
        self.assertEqual({"archive-0", "archive-1", "archive-2"}, set(rows))

    def test_minecraft_entry_may_name_its_game(self):
        entry = _minecraft_direct_entry()
        entry["game"] = "minecraft"
        self.write("mods/minecraft-pinned-mod.json", entry)
        out = self.compile()
        self.assertIn("minecraft-pinned-mod", self.minecraft_rows(out))
        self.assertNotIn("minecraft-pinned-mod", self.game_rows(out))

    def test_compiled_schema_version_is_the_compiler_constant(self):
        self.skyrim_entry_at(_skyrim_github_entry())
        conn = sqlite3.connect(str(self.compile()))
        try:
            version = conn.execute("SELECT version FROM schema_version").fetchone()[0]
        finally:
            conn.close()
        self.assertEqual(version, _compile.SCHEMA_VERSION)
        # Schema 9 stays: registry_items is unchanged, and the new table is additive.
        self.assertEqual(_compile.SCHEMA_VERSION, 9)

    # -- folder and game ----------------------------------------------------

    def test_an_entry_whose_game_disagrees_with_its_folder_is_refused(self):
        entry = _skyrim_github_entry()
        entry["game"] = "valheim"
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", '"game" must be', "skyrim-se")

    def test_a_folder_for_an_undeclared_game_is_refused(self):
        entry = _skyrim_github_entry()
        entry["game"] = "not-a-game"
        self.write("games/not-a-game/mods/crash-logger.json", entry)
        self.assertRefused("crash-logger.json", "not-a-game")

    def test_an_other_game_entry_outside_the_games_folder_is_refused(self):
        self.write("mods/crash-logger.json", _skyrim_github_entry())
        self.assertRefused("crash-logger.json", '"game" is', "registry/games/skyrim-se")

    def test_a_minecraft_folder_entry_may_not_carry_game_compatibility(self):
        entry = _minecraft_direct_entry()
        entry["game_compatibility"] = [{"stores": ["steam"], "game_versions": ["1.0"]}]
        self.write("mods/minecraft-pinned-mod.json", entry)
        self.assertRefused("minecraft-pinned-mod.json", '"game_compatibility"')

    def test_an_other_game_packs_folder_is_refused(self):
        self.write("games/skyrim-se/packs/crash-logger.json", _skyrim_github_entry())
        self.assertRefused("packs", "only mods entries")

    # -- compatibility ------------------------------------------------------

    def test_unknown_store_is_refused(self):
        entry = _skyrim_github_entry()
        entry["game_compatibility"][0]["stores"] = ["epic"]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "game_compatibility[0].stores", "'epic'")

    def test_unknown_framework_is_refused(self):
        entry = _skyrim_github_entry()
        entry["game_compatibility"][0]["requires"] = [{"framework": "nvse", "min_version": "1.0"}]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "requires", "'nvse'")

    def test_empty_arrays_are_refused(self):
        cases = {
            "game_compatibility": lambda e: e.update(game_compatibility=[]),
            "stores": lambda e: e["game_compatibility"][0].update(stores=[]),
            "game_versions": lambda e: e["game_compatibility"][0].update(game_versions=[]),
        }
        for field_name, break_entry in cases.items():
            with self.subTest(field=field_name):
                self.reset_registry()
                entry = _skyrim_github_entry()
                break_entry(entry)
                self.skyrim_entry_at(entry)
                self.assertRefused("crash-logger.json", field_name)

    def test_non_numeric_min_version_is_refused(self):
        entry = _skyrim_github_entry()
        entry["game_compatibility"][0]["requires"] = [{"framework": "skse", "min_version": "2.2.6b"}]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "min_version", "'2.2.6b'")

    def test_game_version_globs_must_cover_whole_components(self):
        for bad in ("1.6.11*", "1.6.1170.0-1", "1.6.1170.x", ""):
            with self.subTest(version=bad):
                self.reset_registry()
                entry = _skyrim_github_entry()
                entry["game_compatibility"][0]["game_versions"] = [bad]
                self.skyrim_entry_at(entry)
                self.assertRefused("crash-logger.json", "game_versions")

    def test_a_github_entry_needs_an_asset(self):
        entry = _skyrim_github_entry()
        del entry["game_compatibility"][0]["asset"]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "game_compatibility[0].asset", "required")

    def test_a_direct_hash_entry_may_not_name_an_asset(self):
        entry = _skyrim_direct_entry()
        entry["game_compatibility"][0]["asset"] = "Mod-*.zip"
        self.skyrim_entry_at(entry)
        self.assertRefused("skyrim-archive-mod.json", "game_compatibility[0].asset", "only for github_release")

    def test_unknown_compatibility_keys_are_refused(self):
        entry = _skyrim_github_entry()
        entry["game_compatibility"][0]["game_version"] = ["1.6.1170.0"]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "game_version")

    # -- Minecraft-only fields and strategies ---------------------------------

    def test_minecraft_only_fields_are_refused_on_other_game_entries(self):
        extras = {
            "compatible_versions": [{"mc_version": "1.21", "loader": "fabric", "mod_version": "1.0"}],
            "mod_dependencies": {"required": ["fabric-api"]},
            "package_signatures": ["com.example.mod"],
            "mod_jar_aliases": ["example"],
            "modrinth_id": "AAAAAAAA",
        }
        for field_name, value in extras.items():
            with self.subTest(field=field_name):
                self.reset_registry()
                entry = _skyrim_github_entry()
                entry[field_name] = value
                self.skyrim_entry_at(entry)
                self.assertRefused("crash-logger.json", f'"{field_name}"')

    def test_modrinth_strategy_is_refused_for_another_game(self):
        entry = _skyrim_github_entry()
        entry["download_strategy"] = "modrinth_id"
        entry["source_identifier"] = "AAAAAAAA"
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "'modrinth_id'", "github_release or direct_hash")

    def test_a_github_identifier_must_be_owner_and_repo(self):
        entry = _skyrim_github_entry()
        entry["source_identifier"] = "just-a-name"
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "owner/repo")

    def test_a_direct_hash_url_must_be_https(self):
        self.skyrim_entry_at(_skyrim_direct_entry(url="http://example.com/files/Mod-1.0.zip"))
        self.assertRefused("skyrim-archive-mod.json", "https://")

    def test_sha256_is_required_on_other_game_entries(self):
        entry = _skyrim_github_entry()
        entry["sha256"] = "not-a-hash"
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "sha256")

    # -- download hashes and pins ----------------------------------------------

    def test_a_github_entry_for_another_game_compiles_without_sha256(self):
        entry = _skyrim_github_entry()
        del entry["sha256"]
        self.skyrim_entry_at(entry)
        out = self.compile()
        self.assertIsNone(self.game_rows(out)["crash-logger"]["sha256"])

    def test_a_direct_hash_entry_for_another_game_still_requires_sha256(self):
        entry = _skyrim_direct_entry()
        del entry["sha256"]
        self.skyrim_entry_at(entry)
        self.assertRefused("skyrim-archive-mod.json", "sha256", "required")

    def test_a_valid_pin_on_another_game_entry_compiles_and_is_stored(self):
        entry = _skyrim_github_entry()
        entry["download_sources"] = [
            {
                "strategy": "github_release",
                "identifier": "example-author/crash-logger",
                "pins": [{"tag": "v1.0", "asset": "CrashLogger-1.0.7z", "sha256": "D" * 64}],
            }
        ]
        self.skyrim_entry_at(entry)
        out = self.compile()
        sources = json.loads(self.game_rows(out)["crash-logger"]["download_sources_json"])
        self.assertEqual(
            sources[0]["pins"],
            [{"tag": "v1.0", "asset": "CrashLogger-1.0.7z", "sha256": "d" * 64}],
        )

    def test_a_malformed_pin_on_another_game_entry_is_refused(self):
        entry = _skyrim_github_entry()
        entry["download_sources"] = [
            {
                "strategy": "github_release",
                "identifier": "example-author/crash-logger",
                "pins": [{"tag": "v1.0", "asset": "CrashLogger-1.0.7z", "sha256": "short"}],
            }
        ]
        self.skyrim_entry_at(entry)
        self.assertRefused("crash-logger.json", "pins[0].sha256", "64 hex")

    # -- identity -----------------------------------------------------------

    def test_an_id_used_by_two_games_is_refused_naming_both_files(self):
        self.write("mods/shared-id.json", _minecraft_direct_entry("shared-id"))
        self.skyrim_entry_at(_skyrim_github_entry("shared-id"))
        message = self.assertRefused("Duplicate catalog id 'shared-id'", "shared-id.json")
        self.assertEqual(message.count("shared-id.json"), 2)


class RealSkyrimPackageEntryTests(unittest.TestCase):
    """The brief's SKSE example compiles against the real Skyrim package, which
    declares SKSE (MASTER_SPEC §26.8). Nothing here patches the game packages."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.registry = self.tmp / "registry"
        self.registry.mkdir()
        patches = [
            mock.patch.object(_compile, "REGISTRY_DIR", self.registry),
            mock.patch.object(_compile, "_hydrate_modrinth_metadata", lambda items: None),
            mock.patch.object(_compile, "_hydrate_modrinth_versions", lambda items: None),
            mock.patch.object(_compile, "_hydrate_version_changelogs", lambda items: []),
            mock.patch.object(_compile, "_load_github_token", lambda: None),
            mock.patch.object(
                _compile, "manifest_date_added", lambda path: "2026-01-01T00:00:00+00:00"
            ),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def write_entry(self, data):
        path = self.registry / "games" / "skyrim-se" / "mods" / f"{data['id']}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data), encoding="utf-8")

    def compile_rows(self):
        out = self.tmp / "out" / "registry.db"
        _compile.compile_registry(out, skip_sign=True, no_governance_write=True, governance_mode_str="off")
        conn = sqlite3.connect(str(out))
        conn.row_factory = sqlite3.Row
        try:
            return {row["id"]: dict(row) for row in conn.execute("SELECT * FROM game_catalog_items")}
        finally:
            conn.close()

    def test_the_real_package_declares_skse(self):
        declared = _compile.load_game_declarations()
        self.assertIn("skse", declared["skyrim-se"].frameworks)

    def test_the_brief_example_compiles_against_the_real_package(self):
        self.write_entry(_skyrim_github_entry())
        rows = self.compile_rows()
        compat = json.loads(rows["crash-logger"]["game_compatibility_json"])
        self.assertEqual(compat[0]["requires"], [{"framework": "skse", "min_version": "2.2.6"}])
        self.assertEqual(compat[0]["stores"], ["steam", "gog"])

    def test_an_undeclared_framework_is_still_refused_against_the_real_package(self):
        entry = _skyrim_github_entry()
        entry["game_compatibility"][0]["requires"] = [{"framework": "nvse", "min_version": "1.0"}]
        self.write_entry(entry)
        with self.assertRaises(SystemExit) as caught:
            self.compile_rows()
        self.assertIn("'nvse'", str(caught.exception.code))


class DownloadHashPolicyTests(unittest.TestCase):
    """The manifest sha256 is required only where a pinned source needs it, and
    curator pins validate. Covers Minecraft entries; other games are in
    OtherGameEntryTests."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.manifest = self.tmp / "item.json"
        self.manifest.write_text("{}", encoding="utf-8")
        patch = mock.patch.object(
            _compile, "manifest_date_added", lambda path: "2026-01-01T00:00:00+00:00"
        )
        patch.start()
        self.addCleanup(patch.stop)
        self.conn = sqlite3.connect(":memory:")
        self.addCleanup(self.conn.close)
        _compile.create_tables(self.conn)

    def _github_item(self, **overrides):
        item = {
            "id": "xaeros-minimap",
            "name": "Xaero's Minimap",
            "content_type": "mod",
            "download_strategy": "github_release",
            "source_identifier": "owner/repo",
            "compatible_versions": [
                {"mc_version": "1.21", "loader": "fabric", "mod_version": "1.2.0"},
            ],
        }
        item.update(overrides)
        return item

    def _insert(self, item):
        _compile.insert_registry_item(self.conn, item, self.manifest)
        return self.conn.execute(
            "SELECT sha256, download_sources_json FROM registry_items WHERE id = ?",
            (item["id"],),
        ).fetchone()

    def test_a_github_release_entry_compiles_without_sha256(self):
        sha256, _ = self._insert(self._github_item())
        self.assertIsNone(sha256)

    def test_a_modrinth_entry_compiles_without_sha256(self):
        item = self._github_item(download_strategy="modrinth_id", source_identifier="AANobbMI")
        sha256, _ = self._insert(item)
        self.assertIsNone(sha256)

    def test_a_github_entry_that_states_a_sha256_keeps_it(self):
        sha256, _ = self._insert(self._github_item(sha256="a" * 64))
        self.assertEqual(sha256, "a" * 64)

    def test_a_malformed_sha256_is_refused_even_though_it_is_optional(self):
        with self.assertRaises(SystemExit):
            self._insert(self._github_item(sha256="not-a-hash"))

    def test_direct_hash_still_requires_sha256(self):
        item = _minecraft_direct_entry("direct-no-hash")
        del item["sha256"]
        with self.assertRaises(SystemExit):
            self._insert(item)

    def test_a_pinned_fallback_source_requires_sha256_even_on_a_github_primary(self):
        item = self._github_item(
            download_sources=[
                {"strategy": "github_release", "identifier": "owner/repo"},
                {"strategy": "direct_hash", "identifier": "https://example.com/files/mod-1.2.0.jar"},
            ],
        )
        with self.assertRaises(SystemExit):
            self._insert(item)
        item["sha256"] = "b" * 64
        sha256, _ = self._insert(item)
        self.assertEqual(sha256, "b" * 64)

    def test_a_valid_pin_is_kept_in_download_sources(self):
        pin = {"tag": "v1.2.0", "asset": "xaeros-1.2.0.jar", "sha256": "C" * 64}
        item = self._github_item(
            download_sources=[{"strategy": "github_release", "identifier": "owner/repo", "pins": [pin]}],
        )
        _, sources_json = self._insert(item)
        sources = json.loads(sources_json)
        self.assertEqual(
            sources[0]["pins"],
            [{"tag": "v1.2.0", "asset": "xaeros-1.2.0.jar", "sha256": "c" * 64}],
            "pins are stored in canonical (lowercase) form",
        )

    def test_a_source_without_pins_stores_no_pins_key(self):
        _, sources_json = self._insert(self._github_item())
        self.assertNotIn("pins", json.loads(sources_json)[0])

    def test_malformed_pins_are_refused(self):
        good = {"tag": "v1.2.0", "asset": "xaeros-1.2.0.jar", "sha256": "c" * 64}
        bad_pins = {
            "empty tag": {**good, "tag": " "},
            "empty asset": {**good, "asset": ""},
            "asset with a path": {**good, "asset": "dir/xaeros.jar"},
            "short sha256": {**good, "sha256": "c" * 63},
            "non-hex sha256": {**good, "sha256": "z" * 64},
            "unknown key": {**good, "size": 10},
            "missing sha256": {"tag": "v1.2.0", "asset": "xaeros-1.2.0.jar"},
        }
        for label, pin in bad_pins.items():
            with self.subTest(pin=label):
                item = self._github_item(
                    download_sources=[
                        {"strategy": "github_release", "identifier": "owner/repo", "pins": [pin]},
                    ],
                )
                with self.assertRaises(SystemExit):
                    self._insert(item)

    def test_a_pin_repeated_for_the_same_release_file_is_refused(self):
        pin = {"tag": "v1.2.0", "asset": "xaeros-1.2.0.jar", "sha256": "c" * 64}
        item = self._github_item(
            download_sources=[
                {"strategy": "github_release", "identifier": "owner/repo", "pins": [pin, dict(pin)]},
            ],
        )
        with self.assertRaises(SystemExit):
            self._insert(item)

    def test_pins_are_only_for_github_release_sources(self):
        item = _minecraft_direct_entry("direct-with-pin")
        del item["sha256"]
        item["download_sources"] = [
            {
                "strategy": "direct_hash",
                "identifier": "https://example.com/files/pinned-1.0.0.jar",
                "pins": [{"tag": "v1", "asset": "pinned-1.0.0.jar", "sha256": "c" * 64}],
            },
        ]
        with self.assertRaises(SystemExit):
            normalize_download_sources_for_test(item)

    def test_the_live_manifests_that_state_a_sha256_still_normalize(self):
        registry = Path(_compile.REGISTRY_DIR) / "mods"
        manifests = sorted(registry.glob("*.json"))
        self.assertTrue(manifests, "the live registry has mod manifests")
        for path in manifests:
            with self.subTest(manifest=path.name):
                item = json.loads(path.read_text(encoding="utf-8"))
                if item.get("archived") or item.get("status") == "archived":
                    continue
                sources = _compile.normalize_download_sources(item)
                _compile.require_manifest_sha256(item["id"], sources, item.get("sha256"))


def normalize_download_sources_for_test(item):
    return _compile.normalize_download_sources(item)


if __name__ == "__main__":
    unittest.main()
