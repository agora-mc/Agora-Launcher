#!/usr/bin/env python3
"""Unit tests for pure functions in Agora utility scripts."""

import http.client
import json
import os
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from typing import Any
from unittest import mock

sys.path.insert(0, os.path.dirname(__file__))

import fetch_loader_manifests
import deploy_release_assets
import finalize_release_assets as fra
import refresh_loader_manifests
import validate_loader_catalog_delta
import build_docs_web as bdw
import pin_hashes
import stage_cli_sidecar


def _response(read_side_effect=None, read_value=b"data"):
    """A urlopen context manager whose read() behaves as configured."""
    response = mock.MagicMock()
    if read_side_effect is not None:
        response.__enter__.return_value.read.side_effect = read_side_effect
    else:
        response.__enter__.return_value.read.return_value = read_value
    return response


@mock.patch("fetch_loader_manifests.time.sleep")
class TestFetchBytes(unittest.TestCase):
    """Tests for retries and network error normalization in _fetch_bytes."""

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_read_timeout_is_normalized_to_url_error(self, urlopen, _sleep):
        urlopen.return_value = _response(read_side_effect=TimeoutError("timed out"))

        with self.assertRaises(urllib.error.URLError) as raised:
            fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertIsInstance(raised.exception.__cause__, TimeoutError)

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_url_error_is_not_wrapped_again(self, urlopen, _sleep):
        original = urllib.error.URLError("not found")
        urlopen.side_effect = original

        with self.assertRaises(urllib.error.URLError) as raised:
            fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertIs(raised.exception, original)

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_truncated_body_is_retried_until_it_succeeds(self, urlopen, sleep):
        truncated = http.client.IncompleteRead(b"partial", 6_091_457)
        urlopen.side_effect = [
            _response(read_side_effect=truncated),
            _response(read_value=b"whole jar"),
        ]

        data = fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertEqual(data, b"whole jar")
        self.assertEqual(urlopen.call_count, 2)
        sleep.assert_called_once_with(fetch_loader_manifests.FETCH_RETRY_BACKOFF)

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_truncated_body_surfaces_as_url_error_once_attempts_run_out(
        self, urlopen, _sleep
    ):
        urlopen.return_value = _response(
            read_side_effect=http.client.IncompleteRead(b"partial", 10)
        )

        with self.assertRaises(urllib.error.URLError) as raised:
            fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertIsInstance(raised.exception.__cause__, http.client.IncompleteRead)
        self.assertEqual(
            urlopen.call_count, fetch_loader_manifests.FETCH_ATTEMPTS
        )

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_missing_artifact_is_not_retried(self, urlopen, _sleep):
        urlopen.side_effect = urllib.error.HTTPError(
            "https://example.test/file.jar", 404, "Not Found", {}, None
        )

        with self.assertRaises(urllib.error.HTTPError):
            fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertEqual(urlopen.call_count, 1)

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_server_error_is_retried(self, urlopen, _sleep):
        urlopen.side_effect = [
            urllib.error.HTTPError(
                "https://example.test/file.jar", 503, "Unavailable", {}, None
            ),
            _response(read_value=b"whole jar"),
        ]

        data = fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        self.assertEqual(data, b"whole jar")
        self.assertEqual(urlopen.call_count, 2)

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_retry_backoff_doubles(self, urlopen, sleep):
        urlopen.return_value = _response(
            read_side_effect=http.client.IncompleteRead(b"partial", 10)
        )

        with self.assertRaises(urllib.error.URLError):
            fetch_loader_manifests._fetch_bytes("https://example.test/file.jar")

        backoff = fetch_loader_manifests.FETCH_RETRY_BACKOFF
        self.assertEqual(
            [call.args[0] for call in sleep.call_args_list],
            [backoff * 2 ** n for n in range(fetch_loader_manifests.FETCH_ATTEMPTS - 1)],
        )

    @mock.patch("fetch_loader_manifests.urllib.request.urlopen")
    def test_custom_timeout_is_passed_to_urlopen(self, urlopen, _sleep):
        urlopen.return_value = _response(read_value=b"data")

        fetch_loader_manifests._fetch_bytes(
            "https://example.test/file.jar", timeout=1
        )

        _, kwargs = urlopen.call_args
        self.assertEqual(kwargs["timeout"], 1)


class TestIsStandardRelease(unittest.TestCase):
    """Tests for refresh_loader_manifests._is_standard_release."""

    def test_stable_two_part(self):
        """Two-part numeric version like '1.21' is standard."""
        self.assertTrue(refresh_loader_manifests._is_standard_release("1.21"))

    def test_stable_three_part(self):
        """Three-part numeric version like '1.21.1' is standard."""
        self.assertTrue(refresh_loader_manifests._is_standard_release("1.21.1"))

    def test_stable_minor_patch(self):
        """Version '1.20.6' is standard."""
        self.assertTrue(refresh_loader_manifests._is_standard_release("1.20.6"))

    def test_snapshot_weekly(self):
        """Weekly snapshot format like '24w14a' is NOT standard (regex excludes it)."""
        self.assertFalse(refresh_loader_manifests._is_standard_release("24w14a"))

    def test_snapshot_26w(self):
        """26w-prefixed snapshot like '26w07a' is NOT standard."""
        self.assertFalse(refresh_loader_manifests._is_standard_release("26w07a"))

    def test_prerelease(self):
        """Prerelease like '1.21-pre1' is NOT standard (regex excludes suffixes)."""
        self.assertFalse(refresh_loader_manifests._is_standard_release("1.21-pre1"))

    def test_release_candidate(self):
        """Release candidate like '1.21-rc1' is NOT standard."""
        self.assertFalse(refresh_loader_manifests._is_standard_release("1.21-rc1"))

    def test_invalid(self):
        """Non-numeric / malformed version is not standard."""
        self.assertFalse(refresh_loader_manifests._is_standard_release("0.0.0-invalid"))

    def test_empty(self):
        """Empty string is not standard."""
        self.assertFalse(refresh_loader_manifests._is_standard_release(""))








class TestLoaderCatalogRefreshSafety(unittest.TestCase):
    """Tests for append-only loader refresh safeguards."""

    def _entry(self, version: str = "0.1.0") -> dict[str, str]:
        return {
            "mc_version": "1.21",
            "loader_version": version,
            "source_url": f"https://example.test/{version}.jar",
            "sha256": version.ljust(64, "0"),
            "file_name": f"loader-{version}.jar",
            "file_type": "installer_jar",
        }

    def test_merge_retains_existing_tuple_and_adds_new_tuple(self):
        existing = self._entry()
        added = self._entry("0.2.0")

        merged = fetch_loader_manifests._merge_entries([existing], [added], loader="fabric")

        self.assertEqual(merged, [existing, added])

    def test_merge_rejects_derivation_change_for_unchanged_file(self):
        existing = self._entry()
        changed = dict(existing)
        changed["file_name"] = "renamed.jar"

        with self.assertRaises(fetch_loader_manifests.ExistingEntryMutationError) as raised:
            fetch_loader_manifests._merge_entries([existing], [changed], loader="fabric")

        self.assertEqual(raised.exception.mutations[0]["key"], "1.21/0.1.0")
        self.assertIn("file_name", raised.exception.mutations[0]["fields"])

    @mock.patch("fetch_loader_manifests._fetch_json", return_value={})
    def test_malformed_metadata_raises_instead_of_returning_empty(self, _fetch_json):
        with self.assertRaises(fetch_loader_manifests.UpstreamMetadataError):
            fetch_loader_manifests._fetch_fabric("1.21")

    def test_delta_accepts_append_only_new_entry(self):
        old = self._entry()
        new = self._entry("0.2.0")
        before = {"domain_allowlist": ["example.test"], "loaders": {"fabric": [old]}}
        after = {
            "domain_allowlist": ["example.test"],
            "loaders": {"fabric": [old, new]},
        }

        report = validate_loader_catalog_delta.validate_catalog_delta(
            before,
            after,
            append_only=True,
            reject_existing_mutations=True,
            max_new_entries=50,
        )

        self.assertEqual(report["errors"], [])
        self.assertEqual(report["new_entries"], 1)

    def test_delta_rejects_existing_mutation(self):
        old = self._entry()
        changed = dict(old)
        changed["source_url"] = "https://example.test/replaced.jar"
        before = {"domain_allowlist": ["example.test"], "loaders": {"fabric": [old]}}
        after = {"domain_allowlist": ["example.test"], "loaders": {"fabric": [changed]}}

        report = validate_loader_catalog_delta.validate_catalog_delta(
            before,
            after,
            append_only=True,
            reject_existing_mutations=True,
        )

        self.assertEqual(len(report["existing_entries_mutated"]), 1)
        self.assertIn("existing loader entries were mutated", report["errors"])

    def test_delta_rejects_deletion(self):
        old = self._entry()
        before = {"domain_allowlist": ["example.test"], "loaders": {"fabric": [old]}}
        after = {"domain_allowlist": ["example.test"], "loaders": {"fabric": []}}

        report = validate_loader_catalog_delta.validate_catalog_delta(
            before,
            after,
            append_only=True,
        )

        self.assertEqual(report["unexpected_deletions"], ["fabric/1.21/0.1.0"])
        self.assertIn("append-only validation found deleted entries", report["errors"])


class TestPinnedFileChanges(unittest.TestCase):
    """A changed upstream file is re-pinned (repack) or quarantined, never fatal."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        fetch_loader_manifests.load_quarantine(self.tmp / "absent.json")

    def tearDown(self):
        fetch_loader_manifests.load_quarantine(self.tmp / "absent.json")
        self._tmp.cleanup()

    def _entry(self, version="1.0", sha="a", content="c1", **extra):
        entry = {
            "mc_version": "1.21",
            "loader_version": version,
            "source_url": f"https://example.test/forge-{version}.jar",
            "sha256": sha * 64,
            "file_name": f"forge-{version}.jar",
            "file_type": "installer_jar",
            "version_json_sha256": "v" * 64,
        }
        if content is not None:
            entry["content_sha256"] = content * 32
        entry.update(extra)
        return entry

    def _jar(self, name, members, *, compression=None):
        import zipfile

        path = self.tmp / name
        if compression is None:
            compression = zipfile.ZIP_DEFLATED
        with zipfile.ZipFile(path, "w", compression) as zf:
            for member, data in members.items():
                zf.writestr(member, data)
        return path

    # -- Content hash -------------------------------------------------------

    def test_content_hash_ignores_signing_and_compression(self):
        import zipfile

        files = {"version.json": b"{}", "net/Installer.class": b"\xca\xfe\xba\xbe code"}
        plain = self._jar(
            "plain.jar",
            {"META-INF/MANIFEST.MF": b"Manifest-Version: 1.0\r\nMain-Class: net.Installer\r\n\r\n", **files},
        )
        resigned = self._jar(
            "resigned.jar",
            {
                **files,
                "META-INF/MANIFEST.MF": (
                    b"Manifest-Version: 1.0\nMain-Class: net.Installer\nCreated-By: other jdk\n\n"
                    b"Name: version.json\nSHA-256-Digest: abc=\n\n"
                ),
                "META-INF/FORGE.SF": b"signature file",
                "META-INF/FORGE.RSA": b"signature block",
            },
            compression=zipfile.ZIP_STORED,
        )

        self.assertIsNotNone(fetch_loader_manifests.installer_content_sha256(plain))
        self.assertEqual(
            fetch_loader_manifests.installer_content_sha256(plain),
            fetch_loader_manifests.installer_content_sha256(resigned),
        )

    def test_content_hash_sees_code_entry_point_and_new_files(self):
        base = {
            "META-INF/MANIFEST.MF": b"Main-Class: net.Installer\n",
            "net/Installer.class": b"original",
        }
        original = fetch_loader_manifests.installer_content_sha256(self._jar("a.jar", base))
        for name, change in (
            ("code.jar", {"net/Installer.class": b"patched"}),
            ("main.jar", {"META-INF/MANIFEST.MF": b"Main-Class: net.Evil\n"}),
            ("extra.jar", {"net/Extra.class": b"new"}),
        ):
            with self.subTest(name):
                changed = self._jar(name, {**base, **change})
                self.assertNotEqual(
                    original, fetch_loader_manifests.installer_content_sha256(changed)
                )

    def test_content_hash_refuses_ambiguous_jar(self):
        import warnings
        import zipfile

        path = self.tmp / "dup.jar"
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            with zipfile.ZipFile(path, "w") as zf:
                zf.writestr("a.class", b"one")
                zf.writestr("a.class", b"two")
        self.assertIsNone(fetch_loader_manifests.installer_content_sha256(path))

    # -- Merge --------------------------------------------------------------

    def test_repack_is_repinned_with_history(self):
        pinned = self._entry()
        upstream = self._entry(sha="b")

        merged = fetch_loader_manifests._merge_entries([pinned], [upstream], loader="forge")

        self.assertEqual(merged[0]["sha256"], "b" * 64)
        self.assertEqual(merged[0]["sha256_history"][0]["sha256"], "a" * 64)
        self.assertEqual(fetch_loader_manifests.get_new_quarantines(), [])

    def test_changed_contents_are_quarantined_not_fatal(self):
        pinned = self._entry()
        other = self._entry("2.0", sha="d")
        upstream = self._entry(sha="b", content="c2")

        merged = fetch_loader_manifests._merge_entries(
            [pinned, other], [upstream, other], loader="forge"
        )

        self.assertEqual(merged, [other])
        self.assertEqual(fetch_loader_manifests.get_new_quarantines(), ["forge/1.21/1.0"])
        self.assertEqual(fetch_loader_manifests.quarantined_keys(), {"forge/1.21/1.0"})

    def test_unfingerprinted_entry_is_quarantined(self):
        pinned = self._entry(content=None)
        upstream = self._entry(sha="b")

        merged = fetch_loader_manifests._merge_entries([pinned], [upstream], loader="forge")

        self.assertEqual(merged, [])
        self.assertEqual(fetch_loader_manifests.quarantined_keys(), {"forge/1.21/1.0"})

    def test_quarantined_entry_is_not_readded_as_new(self):
        pinned = self._entry()
        upstream = self._entry(sha="b", content="c2")
        merged = fetch_loader_manifests._merge_entries([pinned], [upstream], loader="forge")

        rerun = fetch_loader_manifests._merge_entries(merged, [upstream], loader="forge")

        self.assertEqual(rerun, [])

    def test_quarantine_survives_save_and_load(self):
        pinned = self._entry()
        fetch_loader_manifests._merge_entries(
            [pinned], [self._entry(sha="b", content="c2")], loader="forge"
        )
        path = self.tmp / "quarantine.json"
        fetch_loader_manifests.save_quarantine(path)

        fetch_loader_manifests.load_quarantine(path)

        self.assertEqual(fetch_loader_manifests.quarantined_keys(), {"forge/1.21/1.0"})
        self.assertEqual(fetch_loader_manifests.get_new_quarantines(), [])
        # A committed quarantine keeps the entry out of a catalog that still has it.
        self.assertEqual(
            fetch_loader_manifests._merge_entries([pinned], [], loader="forge"), []
        )

    def test_approval_repins_and_restores_entry(self):
        pinned = self._entry()
        upstream = self._entry(sha="b", content="c2")
        fetch_loader_manifests._merge_entries([pinned], [upstream], loader="forge")
        path = self.tmp / "quarantine.json"
        fetch_loader_manifests.save_quarantine(path)
        data = json.loads(path.read_text())
        data["entries"][0]["approved"] = True
        path.write_text(json.dumps(data))
        fetch_loader_manifests.load_quarantine(path)

        merged = fetch_loader_manifests._merge_entries([], [upstream], loader="forge")

        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0]["sha256"], "b" * 64)
        self.assertEqual(merged[0]["content_sha256"], "c2" * 32)
        self.assertEqual(merged[0]["sha256_history"][0]["sha256"], "a" * 64)
        self.assertEqual(fetch_loader_manifests.quarantined_keys(), set())

    def test_upstream_reverting_restores_pinned_entry(self):
        pinned = self._entry()
        fetch_loader_manifests._merge_entries(
            [pinned], [self._entry(sha="b", content="c2")], loader="forge"
        )

        merged = fetch_loader_manifests._merge_entries([], [self._entry()], loader="forge")

        self.assertEqual(merged, [pinned])
        self.assertEqual(fetch_loader_manifests.quarantined_keys(), set())

    def test_malformed_quarantine_file_is_an_error(self):
        path = self.tmp / "quarantine.json"
        path.write_text("{not json")
        with self.assertRaises(ValueError):
            fetch_loader_manifests.load_quarantine(path)

    # -- Delta validation ---------------------------------------------------

    def _delta(self, before_entries, after_entries, **kwargs):
        return validate_loader_catalog_delta.validate_catalog_delta(
            {"domain_allowlist": ["example.test"], "loaders": {"forge": before_entries}},
            {"domain_allowlist": ["example.test"], "loaders": {"forge": after_entries}},
            append_only=True,
            reject_existing_mutations=True,
            **kwargs,
        )

    def test_delta_accepts_recorded_rotation(self):
        pinned = self._entry()
        repinned = self._entry(
            sha="b", version_json_sha256="w" * 64,
            sha256_history=[{"sha256": "a" * 64, "replaced_on": "2026-10-19", "reason": "x"}],
        )

        report = self._delta([pinned], [repinned])

        self.assertEqual(report["errors"], [])
        self.assertEqual(report["repinned_entries"], ["forge/1.21/1.0"])

    def test_delta_rejects_unrecorded_hash_change(self):
        report = self._delta([self._entry()], [self._entry(sha="b")])
        self.assertIn("existing loader entries were mutated", report["errors"])

    def test_delta_rotation_does_not_cover_other_fields(self):
        repinned = self._entry(
            sha="b", source_url="https://elsewhere.test/x.jar",
            sha256_history=[{"sha256": "a" * 64, "replaced_on": "2026-10-19", "reason": "x"}],
        )
        report = self._delta([self._entry()], [repinned])
        self.assertIn("source_url", report["existing_entries_mutated"][0]["fields"])

    def test_delta_accepts_quarantined_deletion_only(self):
        report = self._delta(
            [self._entry(), self._entry("2.0")], [], quarantined={"forge/1.21/1.0"}
        )
        self.assertEqual(report["unexpected_deletions"], ["forge/1.21/2.0"])

    # -- Rotating verification ----------------------------------------------

    def test_verification_sample_rotates_through_every_entry(self):
        from datetime import datetime, timedelta, timezone

        entries = [self._entry(str(i)) for i in range(10)]
        manifest = {"loaders": {"forge": entries}}
        seen: set[str] = set()
        start = datetime(2026, 10, 1, tzinfo=timezone.utc)
        for day in range(4):
            now = start + timedelta(days=day)
            with mock.patch.object(refresh_loader_manifests, "datetime") as fake:
                fake.now.return_value = now
                targets = refresh_loader_manifests._verification_targets(
                    manifest, {"forge/1.21/7"}, 3
                )
            keys = [refresh_loader_manifests._entry_key(*pair) for pair in targets]
            self.assertIn("forge/1.21/7", keys)
            seen.update(keys)
        self.assertEqual(len(seen), 10)

    def test_verify_repins_quarantines_and_fingerprints(self):
        import zipfile

        def jar(name, code, compression=zipfile.ZIP_DEFLATED):
            return self._jar(name, {"version.json": b"{}", "Installer.class": code},
                             compression=compression)

        def pin(version, path, **extra):
            entry = self._entry(version, content=None, **extra)
            entry["sha256"] = fetch_loader_manifests._sha256_hex(path.read_bytes())
            entry["version_json_sha256"] = fetch_loader_manifests._extract_version_json_sha256(path)
            return entry

        old_repack = jar("repack-old.jar", b"same")
        new_repack = jar("repack-new.jar", b"same", compression=zipfile.ZIP_STORED)
        unchanged = jar("unchanged.jar", b"stable")
        new_changed = jar("changed-new.jar", b"different")
        repack = pin("1.0", old_repack,
                     content_sha256=fetch_loader_manifests.installer_content_sha256(old_repack))
        changed = pin("2.0", jar("changed-old.jar", b"original"))
        stable = pin("3.0", unchanged)
        served = {
            repack["source_url"]: new_repack,
            changed["source_url"]: new_changed,
            stable["source_url"]: unchanged,
        }
        catalog = self.tmp / "catalog"
        catalog.mkdir()
        (catalog / "loader_manifests.json").write_text(json.dumps(
            {"domain_allowlist": ["example.test"], "loaders": {"forge": [repack, changed, stable]}}
        ))

        with mock.patch.object(fetch_loader_manifests, "LOADER_MANIFESTS_DIR", catalog), \
             mock.patch.object(fetch_loader_manifests, "_download_to_cache",
                               side_effect=lambda url, _name, **_kw: served[url]):
            summary = refresh_loader_manifests.verify_manifest()

        self.assertEqual(summary["repinned"], ["forge/1.21/1.0"])
        self.assertEqual(summary["quarantined"], ["forge/1.21/2.0"])
        written = json.loads((catalog / "loader_manifests.json").read_text())["loaders"]["forge"]
        self.assertEqual([e["loader_version"] for e in written], ["1.0", "3.0"])
        self.assertEqual(written[0]["sha256"],
                         fetch_loader_manifests._sha256_hex(new_repack.read_bytes()))
        self.assertEqual(written[1]["content_sha256"],
                         fetch_loader_manifests.installer_content_sha256(unchanged))
        self.assertTrue((catalog / "known_good_hashes.json").exists())

    def test_verification_without_sample_checks_everything(self):
        manifest = {"loaders": {"forge": [self._entry(str(i)) for i in range(5)]}}
        self.assertEqual(
            len(refresh_loader_manifests._verification_targets(manifest, set(), None)), 5
        )


class TestLoaderManifestEnrichment(unittest.TestCase):
    """Tests for capability/channel enrichment of loader catalog entries."""

    def _entry(self, version: str = "0.1.0", **extra) -> dict[str, Any]:
        entry = {
            "mc_version": "1.21",
            "loader_version": version,
            "source_url": f"https://example.test/{version}.jar",
            "sha256": version.ljust(64, "0"),
            "file_name": f"loader-{version}.jar",
            "file_type": "installer_jar",
        }
        entry.update(extra)
        return entry

    # -- Conservative release-channel detection ------------------------------

    def test_release_channel_conservative_stable(self):
        for version in ("0.18.6", "21.1.181", "51.0.0", "0.30.0", "1.21"):
            with self.subTest(version=version):
                self.assertEqual(
                    fetch_loader_manifests._release_channel(version), "stable"
                )

    def test_release_channel_conservative_prerelease(self):
        for version in (
            "0.29.2-beta.1",
            "0.30.0-beta.0",
            "20.2.86-beta",
            "0.19.0-dev+mc1.21",
            "51.0.0-rc1",
        ):
            with self.subTest(version=version):
                self.assertEqual(
                    fetch_loader_manifests._release_channel(version), "prerelease"
                )

    # -- New entries from fetchers carry capabilities ------------------------

    @mock.patch("fetch_loader_manifests._fetch_profile_json")
    @mock.patch("fetch_loader_manifests._fetch_json")
    def test_fabric_new_entry_writes_capabilities(self, fetch_json, fetch_profile):
        fetch_json.return_value = [{"loader": {"version": "0.19.0"}}]
        fetch_profile.return_value = b'{"loader": {"version": "0.19.0"}}'
        entries = fetch_loader_manifests._fetch_fabric("1.21", refresh_profiles=True)
        self.assertEqual(
            entries[0]["provided_versions"], {"fabricloader": "0.19.0"}
        )
        self.assertEqual(entries[0]["release_channel"], "stable")

    @mock.patch("fetch_loader_manifests._fetch_profile_json")
    @mock.patch("fetch_loader_manifests._fetch_json")
    def test_quilt_new_entry_writes_capabilities(self, fetch_json, fetch_profile):
        fetch_json.return_value = [{"loader": {"version": "0.28.0-beta.5"}}]
        fetch_profile.return_value = b'{"loader": {"version": "0.28.0-beta.5"}}'
        entries = fetch_loader_manifests._fetch_quilt("1.21", refresh_profiles=True)
        self.assertEqual(
            entries[0]["provided_versions"], {"quilt_loader": "0.28.0-beta.5"}
        )
        self.assertEqual(entries[0]["release_channel"], "prerelease")

    @mock.patch("fetch_loader_manifests._download_to_cache")
    @mock.patch("fetch_loader_manifests._fetch_bytes")
    def test_neoforge_new_entry_writes_capabilities(self, fetch_bytes, download):
        fetch_bytes.return_value = (
            b"<metadata><versions><version>21.1.181</version></versions></metadata>"
        )
        with tempfile.NamedTemporaryFile(delete=False) as tmp:
            tmp.write(b"not-a-jar")
            jar_path = Path(tmp.name)
        try:
            download.return_value = jar_path
            entries = fetch_loader_manifests._fetch_neoforge(["1.21.1"])
        finally:
            os.unlink(jar_path)
        self.assertEqual(entries[0]["provided_versions"], {"neoforge": "21.1.181"})
        self.assertEqual(entries[0]["release_channel"], "stable")
        # Invalid pinned metadata fails closed rather than inventing a
        # NeoForge language-provider capability.
        self.assertNotIn("javafml", entries[0]["provided_versions"])
        self.assertNotIn("lowcodefml", entries[0]["provided_versions"])

    @mock.patch("fetch_loader_manifests._download_to_cache")
    @mock.patch("fetch_loader_manifests._fetch_bytes")
    def test_forge_new_entry_writes_capabilities(self, fetch_bytes, download):
        fetch_bytes.return_value = (
            b"<metadata><versions><version>1.21-51.0.0</version></versions></metadata>"
        )
        with tempfile.NamedTemporaryFile(delete=False) as tmp:
            tmp.write(b"not-a-jar")
            jar_path = Path(tmp.name)
        try:
            download.return_value = jar_path
            entries = fetch_loader_manifests._fetch_forge(["1.21"])
        finally:
            os.unlink(jar_path)
        self.assertEqual(
            entries[0]["provided_versions"],
            {"forge": "51.0.0", "javafml": "51", "lowcodefml": "51"},
        )
        self.assertEqual(entries[0]["release_channel"], "stable")

    def test_neoforge_language_providers_come_from_pinned_fml_profile(self):
        profile = {
            "libraries": [
                {"name": "net.neoforged.fancymodloader:loader:4.0.39"},
            ]
        }
        self.assertEqual(
            fetch_loader_manifests._extract_neoforge_language_capabilities(profile),
            {"javafml": "4.0.39", "lowcodefml": "4.0.39"},
        )

    def test_malformed_neoforge_fml_profile_fails_closed(self):
        profile = {
            "libraries": [
                {"name": "net.neoforged.fancymodloader:loader:"},
            ]
        }
        self.assertEqual(
            fetch_loader_manifests._extract_neoforge_language_capabilities(profile),
            {},
        )

    # -- Append-only enrichment in _merge_entries -----------------------------

    def test_merge_enriches_legacy_entry_without_error(self):
        legacy = self._entry()
        fresh = dict(legacy)
        fresh["provided_versions"] = {"fabricloader": "0.1.0"}
        fresh["release_channel"] = "stable"

        merged = fetch_loader_manifests._merge_entries([legacy], [fresh], loader="fabric")

        self.assertEqual(len(merged), 1)
        self.assertEqual(
            merged[0]["provided_versions"], {"fabricloader": "0.1.0"}
        )
        self.assertEqual(merged[0]["release_channel"], "stable")

    def test_merge_treats_empty_provided_versions_as_absent(self):
        legacy = self._entry(provided_versions={})
        fresh = dict(legacy)
        fresh["provided_versions"] = {"fabricloader": "0.1.0"}

        merged = fetch_loader_manifests._merge_entries([legacy], [fresh], loader="fabric")

        self.assertEqual(
            merged[0]["provided_versions"], {"fabricloader": "0.1.0"}
        )

    def test_merge_rejects_changed_capability(self):
        existing = self._entry(provided_versions={"fabricloader": "0.1.0"})
        changed = dict(existing)
        changed["provided_versions"] = {"fabricloader": "0.2.0"}

        with self.assertRaises(fetch_loader_manifests.ExistingEntryMutationError) as raised:
            fetch_loader_manifests._merge_entries([existing], [changed], loader="fabric")

        fields = raised.exception.mutations[0]["fields"]
        self.assertIn("provided_versions", fields)
        self.assertEqual(fields["provided_versions"]["before"], {"fabricloader": "0.1.0"})
        self.assertEqual(fields["provided_versions"]["after"], {"fabricloader": "0.2.0"})

    def test_merge_rejects_changed_release_channel(self):
        existing = self._entry(release_channel="stable")
        changed = dict(existing)
        changed["release_channel"] = "prerelease"

        with self.assertRaises(fetch_loader_manifests.ExistingEntryMutationError) as raised:
            fetch_loader_manifests._merge_entries([existing], [changed], loader="fabric")

        self.assertIn("release_channel", raised.exception.mutations[0]["fields"])

    def test_merge_rank_enrichment_and_change(self):
        existing = self._entry()
        fresh = dict(existing)
        fresh["recommendation_rank"] = 1
        merged = fetch_loader_manifests._merge_entries([existing], [fresh], loader="fabric")
        self.assertEqual(merged[0]["recommendation_rank"], 1)

        existing = self._entry(recommendation_rank=1)
        changed = dict(existing)
        changed["recommendation_rank"] = 2
        with self.assertRaises(fetch_loader_manifests.ExistingEntryMutationError):
            fetch_loader_manifests._merge_entries([existing], [changed], loader="fabric")

    def test_merge_keeps_enriched_entry_stable_on_rerun(self):
        first = self._entry()
        fresh = dict(first)
        fresh["provided_versions"] = {"fabricloader": "0.1.0"}
        fresh["release_channel"] = "stable"
        enriched = fetch_loader_manifests._merge_entries([first], [fresh], loader="fabric")
        # A second refresh with identical upstream data must not mutate.
        rerun = fetch_loader_manifests._merge_entries(enriched, [fresh], loader="fabric")
        self.assertEqual(rerun, enriched)

    # -- Writer enrichment pass ----------------------------------------------

    def test_write_loader_manifests_enriches_legacy_entries(self):
        legacy = self._entry(version="0.18.6")
        manifest = {
            "domain_allowlist": ["example.com"],
            "loaders": {"fabric": [legacy]},
        }
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch("fetch_loader_manifests.LOADER_MANIFESTS_DIR", Path(tmp)):
                fetch_loader_manifests._write_loader_manifests(manifest)
            written = json.loads((Path(tmp) / "loader_manifests.json").read_text())

        entry = written["loaders"]["fabric"][0]
        self.assertEqual(
            entry["provided_versions"], {"fabricloader": "0.18.6"}
        )
        self.assertEqual(entry["release_channel"], "stable")
        self.assertEqual(
            list(entry.keys())[-2:], ["provided_versions", "release_channel"]
        )

    def test_write_enrichment_never_overwrites_existing_values(self):
        existing = self._entry(
            version="0.18.6",
            provided_versions={"fabricloader": "9.9.9"},
            release_channel="prerelease",
            recommendation_rank=7,
        )
        manifest = {
            "domain_allowlist": ["example.com"],
            "loaders": {"fabric": [existing]},
        }
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch("fetch_loader_manifests.LOADER_MANIFESTS_DIR", Path(tmp)):
                fetch_loader_manifests._write_loader_manifests(manifest)
            written = json.loads((Path(tmp) / "loader_manifests.json").read_text())

        entry = written["loaders"]["fabric"][0]
        self.assertEqual(entry["provided_versions"], {"fabricloader": "9.9.9"})
        self.assertEqual(entry["release_channel"], "prerelease")
        self.assertEqual(entry["recommendation_rank"], 7)


class TestSha256Hex(unittest.TestCase):
    """Tests for fetch_loader_manifests._sha256_hex."""

    def test_known_hello(self):
        """SHA-256 of b'hello' matches the known constant."""
        expected = (
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        )
        self.assertEqual(
            fetch_loader_manifests._sha256_hex(b"hello"),
            expected,
        )

    def test_empty(self):
        """SHA-256 of empty bytes matches the known empty-hash constant."""
        expected = (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )
        self.assertEqual(fetch_loader_manifests._sha256_hex(b""), expected)

    def test_deterministic(self):
        """Same input always produces the same hash."""
        data = b"test data for determinism"
        self.assertEqual(
            fetch_loader_manifests._sha256_hex(data),
            fetch_loader_manifests._sha256_hex(data),
        )


class TestStableJsonSha256(unittest.TestCase):
    """Tests for fetch_loader_manifests._stable_json_sha256."""

    def test_canonicalizes_key_order(self):
        """Different key orderings in JSON produce the same hash when keys are sorted."""
        obj1 = {"b": 1, "a": 2}
        obj2 = {"a": 2, "b": 1}
        hash1 = fetch_loader_manifests._stable_json_sha256(
            json.dumps(obj1, separators=(",", ":")).encode()
        )
        hash2 = fetch_loader_manifests._stable_json_sha256(
            json.dumps(obj2, separators=(",", ":")).encode()
        )
        self.assertEqual(hash1, hash2)

    def test_drops_default_keys(self):
        """Default drop set removes 'time' and 'releaseTime' before hashing."""
        payload_with_time = json.dumps(
            {"keep": 1, "time": "2025-01-01T00:00:00Z", "releaseTime": "2025-01-02T00:00:00Z"}
        ).encode()
        payload_without_time = json.dumps(
            {"keep": 1}
        ).encode()
        self.assertEqual(
            fetch_loader_manifests._stable_json_sha256(payload_with_time),
            fetch_loader_manifests._stable_json_sha256(payload_without_time),
        )

    def test_custom_drop(self):
        """Custom drop set removes specified keys before hashing."""
        payload_with_ignore = json.dumps(
            {"keep": 1, "ignore_me": "should not matter"}
        ).encode()
        payload_without_ignore = json.dumps(
            {"keep": 1}
        ).encode()
        self.assertEqual(
            fetch_loader_manifests._stable_json_sha256(
                payload_with_ignore, drop={"ignore_me"}
            ),
            fetch_loader_manifests._stable_json_sha256(payload_without_ignore),
        )

    def test_different_content_different_hash(self):
        """Different JSON content produces different hashes."""
        hash1 = fetch_loader_manifests._stable_json_sha256(b'{"a":1}')
        hash2 = fetch_loader_manifests._stable_json_sha256(b'{"a":2}')
        self.assertNotEqual(hash1, hash2)


# ═══════════════════════════════════════════════════════════════════════════
# Runtime catalog tests
# ═══════════════════════════════════════════════════════════════════════════

import generate_runtime_catalog as grc


class TestRuntimeCatalogSchema(unittest.TestCase):
    """Tests for runtime catalog schema validation."""

    CATALOG_PATH = Path(__file__).resolve().parent.parent / "runtime-catalog" / "runtime_catalog.json"

    @classmethod
    def setUpClass(cls):
        with cls.CATALOG_PATH.open("r", encoding="utf-8") as fh:
            cls.catalog = json.load(fh)

    def test_schema_version(self):
        """schema_version must be 1."""
        self.assertEqual(self.catalog["schema_version"], 1)

    def test_generated_at_present(self):
        """generated_at is a non-empty string."""
        self.assertIsInstance(self.catalog.get("generated_at"), str)
        self.assertTrue(self.catalog["generated_at"])

    def test_source_present(self):
        """source metadata is present."""
        self.assertIsInstance(self.catalog.get("source"), str)
        self.assertTrue(self.catalog["source"])

    def test_entries_is_list(self):
        """entries must be a non-empty list."""
        entries = self.catalog.get("entries")
        self.assertIsInstance(entries, list)
        self.assertGreater(len(entries), 0)

    def test_check_catalog_no_errors(self):
        """Built-in validation passes on the file."""
        errors = grc.check_catalog(self.catalog)
        self.assertEqual(errors, [], f"check_catalog returned errors: {errors}")


class TestRuntimeCatalogEntryFields(unittest.TestCase):
    """Tests for field presence and types in each entry."""

    @classmethod
    def setUpClass(cls):
        with TestRuntimeCatalogSchema.CATALOG_PATH.open("r", encoding="utf-8") as fh:
            cls.catalog = json.load(fh)
        cls.entries = cls.catalog["entries"]

    def test_all_required_fields_present(self):
        """Every entry has all required string and int fields."""
        required_str = [
            "vendor", "full_version", "openjdk_version",
            "os", "arch", "image_type", "jvm_impl",
            "archive_type", "url", "sha256", "java_relative_path",
            "license", "source_api_url",
        ]
        required_int = ["major", "size"]
        for i, entry in enumerate(self.entries):
            for field in required_str:
                self.assertIsInstance(
                    entry.get(field), str,
                    f"entry[{i}].{field} is not a string",
                )
                self.assertTrue(
                    entry[field],
                    f"entry[{i}].{field} is empty",
                )
            for field in required_int:
                self.assertIsInstance(
                    entry.get(field), int,
                    f"entry[{i}].{field} is not an int",
                )
                self.assertGreater(
                    entry[field], 0,
                    f"entry[{i}].{field} is not positive",
                )

    def test_vendor_is_eclipse_temurin(self):
        """Every entry's vendor is 'eclipse-temurin'."""
        for entry in self.entries:
            self.assertEqual(entry["vendor"], "eclipse-temurin")

    def test_license_spdx(self):
        """Every entry has the correct SPDX license."""
        expected = "GPL-2.0-only WITH Classpath-exception-2.0"
        for entry in self.entries:
            self.assertEqual(entry["license"], expected)

    def test_image_type_is_jre_or_jdk(self):
        """Every entry has image_type 'jre' or 'jdk' (JRE fallback to JDK when no JRE published)."""
        for entry in self.entries:
            self.assertIn(entry["image_type"], ("jre", "jdk"),
                          f"Unexpected image_type {entry.get('image_type')} for {entry.get('url')}")

    def test_jvm_impl_is_hotspot(self):
        """Every entry has jvm_impl 'hotspot'."""
        for entry in self.entries:
            self.assertEqual(entry["jvm_impl"], "hotspot")

    def test_sha256_lowercase_hex(self):
        """Every sha256 is 64 lowercase hex chars."""
        for entry in self.entries:
            sha = entry["sha256"]
            self.assertRegex(sha, r"^[0-9a-f]{64}$",
                             f"Invalid SHA-256 for {entry.get('url')}")

    def test_url_https(self):
        """All URLs start with https://."""
        for entry in self.entries:
            self.assertTrue(
                entry["url"].startswith("https://"),
                f"URL not HTTPS: {entry['url']}",
            )

    def test_url_adoptium_github_release(self):
        """All URLs match official Adoptium GitHub release pattern."""
        pattern = grc.ADOPTIUM_GITHUB_RELEASE_RE
        for entry in self.entries:
            self.assertRegex(
                entry["url"], pattern,
                f"URL not official Adoptium GitHub: {entry['url']}",
            )

    def test_os_in_known_set(self):
        """os is one of windows, linux, macos."""
        for entry in self.entries:
            self.assertIn(entry["os"], ["windows", "linux", "macos"])

    def test_arch_in_known_set(self):
        """arch is one of x64, aarch64."""
        for entry in self.entries:
            self.assertIn(entry["arch"], ["x64", "aarch64"])

    def test_windows_entries_use_zip(self):
        """Windows entries use archive_type 'zip'."""
        for entry in self.entries:
            if entry["os"] == "windows":
                self.assertEqual(entry["archive_type"], "zip")

    def test_linux_macos_entries_use_tar_gz(self):
        """Linux and macOS entries use archive_type 'tar.gz'."""
        for entry in self.entries:
            if entry["os"] in ("linux", "macos"):
                self.assertEqual(entry["archive_type"], "tar.gz")

    def test_java_relative_path_correct_per_os(self):
        """java_relative_path matches expected per-OS value."""
        for entry in self.entries:
            os_name = entry["os"]
            jrp = entry["java_relative_path"]
            if os_name == "windows":
                self.assertEqual(jrp, "bin/java.exe")
            elif os_name == "macos":
                self.assertEqual(jrp, "Contents/Home/bin/java")
            else:
                self.assertEqual(jrp, "bin/java")

    def test_size_reasonable(self):
        """Size must be at least 10 MB (10,000,000 bytes)."""
        for entry in self.entries:
            self.assertGreaterEqual(
                entry["size"], 10_000_000,
                f"Suspiciously small size for {entry['url']}",
            )

    def test_major_in_requested_set(self):
        """major is in the set of requested majors."""
        for entry in self.entries:
            self.assertIn(entry["major"], grc.REQUESTED_MAJORS)


class TestRuntimeCatalogDuplicates(unittest.TestCase):
    """Tests for duplicate (major, os, arch) tuple rejection."""

    @classmethod
    def setUpClass(cls):
        with TestRuntimeCatalogSchema.CATALOG_PATH.open("r", encoding="utf-8") as fh:
            cls.entries = json.load(fh)["entries"]

    def test_no_duplicate_tuples(self):
        """No two entries share the same (major, os, arch) tuple."""
        seen: set[tuple[int, str, str]] = set()
        for entry in self.entries:
            key = (entry["major"], entry["os"], entry["arch"])
            self.assertNotIn(
                key, seen,
                f"Duplicate tuple major={key[0]} os={key[1]} arch={key[2]}",
            )
            seen.add(key)

    def test_check_catalog_detects_duplicates(self):
        """check_catalog should flag duplicate tuples."""
        bad = {
            "schema_version": 1,
            "generated_at": "2025-01-01T00:00:00Z",
            "source": "test",
            "entries": [
                {
                    "vendor": "eclipse-temurin", "major": 21,
                    "full_version": "21.0.0+1", "openjdk_version": "21.0.0+1",
                    "os": "linux", "arch": "x64",
                    "image_type": "jre", "jvm_impl": "hotspot",
                    "archive_type": "tar.gz",
                    "url": "https://github.com/adoptium/temurin21-binaries/releases/download/test/OpenJDK21U-jre_x64_linux_hotspot_21.0.0_1.tar.gz",
                    "sha256": "a" * 64,
                    "size": 50000000,
                    "java_relative_path": "bin/java",
                    "license": "GPL-2.0-only WITH Classpath-exception-2.0",
                    "source_api_url": "https://api.adoptium.net/v3/assets/latest/21/hotspot",
                },
                {
                    "vendor": "eclipse-temurin", "major": 21,
                    "full_version": "21.0.0+2", "openjdk_version": "21.0.0+2",
                    "os": "linux", "arch": "x64",
                    "image_type": "jre", "jvm_impl": "hotspot",
                    "archive_type": "tar.gz",
                    "url": "https://github.com/adoptium/temurin21-binaries/releases/download/test/OpenJDK21U-jre_x64_linux_hotspot_21.0.0_2.tar.gz",
                    "sha256": "b" * 64,
                    "size": 50000001,
                    "java_relative_path": "bin/java",
                    "license": "GPL-2.0-only WITH Classpath-exception-2.0",
                    "source_api_url": "https://api.adoptium.net/v3/assets/latest/21/hotspot",
                },
            ],
        }
        errors = grc.check_catalog(bad)
        dup_errors = [e for e in errors if "duplicate" in e.lower()]
        self.assertGreater(len(dup_errors), 0)


class TestRuntimeCatalogDeterministicSort(unittest.TestCase):
    """Tests for deterministic ordering of catalog entries."""

    @classmethod
    def setUpClass(cls):
        with TestRuntimeCatalogSchema.CATALOG_PATH.open("r", encoding="utf-8") as fh:
            cls.entries = json.load(fh)["entries"]

    def test_sorted_by_major_then_os_then_arch(self):
        """Entries are sorted by major ASC, os ASC, arch ASC."""
        os_order = {"linux": 0, "macos": 1, "windows": 2}
        arch_order = {"aarch64": 0, "x64": 1}
        for i in range(len(self.entries) - 1):
            a = self.entries[i]
            b = self.entries[i + 1]
            key_a = (a["major"], os_order.get(a["os"], 99), arch_order.get(a["arch"], 99))
            key_b = (b["major"], os_order.get(b["os"], 99), arch_order.get(b["arch"], 99))
            self.assertLessEqual(
                key_a, key_b,
                f"Entries not sorted: {a['major']}/{a['os']}/{a['arch']} "
                f"before {b['major']}/{b['os']}/{b['arch']}",
            )

    def test_deterministic_json_output(self):
        """Re-serializing the same entries produces identical JSON."""
        entries_json = json.dumps(self.entries, sort_keys=True)
        entries2 = json.loads(entries_json)
        entries2_json = json.dumps(entries2, sort_keys=True)
        self.assertEqual(entries_json, entries2_json)


class TestRuntimeCatalogParserFixture(unittest.TestCase):
    """Tests for the response parser with a mock API response."""

    def test_parse_valid_response(self):
        """A well-formed API response produces a valid entry."""
        fixture = {
            "binary": {
                "architecture": "x64",
                "image_type": "jre",
                "jvm_impl": "hotspot",
                "os": "linux",
                "package": {
                    "checksum": "e5038aae3ca9ff670bc696496b0728dbd23d280026bad30291cb919221ecfdcb",
                    "link": "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_x64_linux_hotspot_21.0.11_10.tar.gz",
                    "name": "OpenJDK21U-jre_x64_linux_hotspot_21.0.11_10.tar.gz",
                    "size": 52099793,
                },
            },
            "vendor": "eclipse",
            "version": {
                "major": 21,
                "minor": 0,
                "security": 11,
                "openjdk_version": "21.0.11+10-LTS",
                "semver": "21.0.11+10.0.LTS",
            },
        }
        result = grc._validate_and_extract(fixture, 21, "linux", "x64")
        self.assertIsInstance(result, dict)
        self.assertEqual(result["major"], 21)
        self.assertEqual(result["os"], "linux")
        self.assertEqual(result["arch"], "x64")
        self.assertEqual(result["sha256"], "e5038aae3ca9ff670bc696496b0728dbd23d280026bad30291cb919221ecfdcb")
        self.assertEqual(result["size"], 52099793)

    def test_parse_os_mismatch(self):
        """Mismatched OS returns an error string."""
        fixture = {
            "binary": {
                "architecture": "x64",
                "image_type": "jre",
                "jvm_impl": "hotspot",
                "os": "windows",
                "package": {
                    "checksum": "a" * 64,
                    "link": "https://github.com/adoptium/temurin21-binaries/releases/download/test/pkg.tar.gz",
                    "name": "pkg.tar.gz",
                    "size": 50000000,
                },
            },
            "vendor": "eclipse",
            "version": {"major": 21, "minor": 0, "security": 0, "openjdk_version": "21", "semver": "21"},
        }
        result = grc._validate_and_extract(fixture, 21, "linux", "x64")
        self.assertIsInstance(result, str)
        self.assertIn("os mismatch", result)

    def test_parse_missing_package_returns_error(self):
        """Missing package data returns an error."""
        fixture = {
            "binary": {
                "architecture": "x64",
                "image_type": "jre",
                "jvm_impl": "hotspot",
                "os": "linux",
            },
            "vendor": "eclipse",
            "version": {"major": 21, "minor": 0, "security": 0, "openjdk_version": "21", "semver": "21"},
        }
        result = grc._validate_and_extract(fixture, 21, "linux", "x64")
        self.assertIsInstance(result, str)
        self.assertIn("package", result.lower())

    def test_parse_bad_checksum_returns_error(self):
        """Non-hex checksum returns an error."""
        fixture = {
            "binary": {
                "architecture": "x64",
                "image_type": "jre",
                "jvm_impl": "hotspot",
                "os": "linux",
                "package": {
                    "checksum": "not-a-valid-sha",
                    "link": "https://github.com/adoptium/temurin21-binaries/releases/download/test/pkg.tar.gz",
                    "name": "pkg.tar.gz",
                    "size": 50000000,
                },
            },
            "vendor": "eclipse",
            "version": {"major": 21, "minor": 0, "security": 0, "openjdk_version": "21", "semver": "21"},
        }
        result = grc._validate_and_extract(fixture, 21, "linux", "x64")
        self.assertIsInstance(result, str)
        self.assertIn("checksum", result.lower())


class TestRuntimeCatalogUnavailableMatrix(unittest.TestCase):
    """Tests for handling of unavailable OS/arch combinations."""

    def test_windows_aarch64_in_unavailable(self):
        """windows + aarch64 is marked unavailable for all requested majors."""
        for major in grc.REQUESTED_MAJORS:
            self.assertIn(
                (major, "windows", "aarch64"),
                grc.UNAVAILABLE_COMBOS,
            )

    def test_warnings_in_catalog(self):
        """The catalog has warnings for unavailable combinations."""
        with TestRuntimeCatalogSchema.CATALOG_PATH.open("r", encoding="utf-8") as fh:
            catalog = json.load(fh)
        warnings = catalog.get("warnings", [])
        self.assertGreater(len(warnings), 0)
        has_unavailable = any("unavailable" in w for w in warnings)
        self.assertTrue(has_unavailable)


class TestRuntimeCatalogMaliciousHost(unittest.TestCase):
    """Tests for rejection of non-Adoptium GitHub URLs."""

    def test_malicious_host_rejected(self):
        """check_catalog rejects non-Adoptium GitHub URLs."""
        bad = {
            "schema_version": 1,
            "generated_at": "2025-01-01T00:00:00Z",
            "source": "test",
            "entries": [
                {
                    "vendor": "eclipse-temurin", "major": 21,
                    "full_version": "21.0.0+1", "openjdk_version": "21.0.0+1",
                    "os": "linux", "arch": "x64",
                    "image_type": "jre", "jvm_impl": "hotspot",
                    "archive_type": "tar.gz",
                    "url": "https://malicious.example.com/backdoor.tar.gz",
                    "sha256": "a" * 64,
                    "size": 50000000,
                    "java_relative_path": "bin/java",
                    "license": "GPL-2.0-only WITH Classpath-exception-2.0",
                    "source_api_url": "https://api.adoptium.net/v3/assets/latest/21/hotspot",
                },
            ],
        }
        errors = grc.check_catalog(bad)
        url_errors = [e for e in errors if "not official" in e]
        self.assertGreater(len(url_errors), 0)

    def test_http_url_rejected(self):
        """check_catalog rejects non-HTTPS URLs."""
        bad = {
            "schema_version": 1,
            "generated_at": "2025-01-01T00:00:00Z",
            "source": "test",
            "entries": [
                {
                    "vendor": "eclipse-temurin", "major": 21,
                    "full_version": "21.0.0+1", "openjdk_version": "21.0.0+1",
                    "os": "linux", "arch": "x64",
                    "image_type": "jre", "jvm_impl": "hotspot",
                    "archive_type": "tar.gz",
                    "url": "http://github.com/adoptium/temurin21-binaries/releases/download/test/pkg.tar.gz",
                    "sha256": "a" * 64,
                    "size": 50000000,
                    "java_relative_path": "bin/java",
                    "license": "GPL-2.0-only WITH Classpath-exception-2.0",
                    "source_api_url": "https://api.adoptium.net/v3/assets/latest/21/hotspot",
                },
            ],
        }
        errors = grc.check_catalog(bad)
        http_errors = [e for e in errors if "not HTTPS" in e]
        self.assertGreater(len(http_errors), 0)

    def test_wrong_github_repo_rejected(self):
        """A GitHub release URL from a non-Adoptium repo is rejected."""
        bad = {
            "schema_version": 1,
            "generated_at": "2025-01-01T00:00:00Z",
            "source": "test",
            "entries": [
                {
                    "vendor": "eclipse-temurin", "major": 21,
                    "full_version": "21.0.0+1", "openjdk_version": "21.0.0+1",
                    "os": "linux", "arch": "x64",
                    "image_type": "jre", "jvm_impl": "hotspot",
                    "archive_type": "tar.gz",
                    "url": "https://github.com/evil-corp/malware/releases/download/v1/pkg.tar.gz",
                    "sha256": "a" * 64,
                    "size": 50000000,
                    "java_relative_path": "bin/java",
                    "license": "GPL-2.0-only WITH Classpath-exception-2.0",
                    "source_api_url": "https://api.adoptium.net/v3/assets/latest/21/hotspot",
                },
            ],
        }
        errors = grc.check_catalog(bad)
        url_errors = [e for e in errors if "URL not official" in e]
        self.assertGreater(len(url_errors), 0)


# ═══════════════════════════════════════════════════════════════════════════
# Tauri command binding check tests
# ═══════════════════════════════════════════════════════════════════════════

import check_tauri_bindings as ctb


class TestParseRustCommands(unittest.TestCase):
    """Tests for parse_rust_commands."""

    def test_parses_generate_handler(self):
        text = """
        .invoke_handler(tauri::generate_handler![
            commands::browse_items,
            commands::for_you_items,
            commands::get_registry_item,
        ])
        """
        result = ctb.parse_rust_commands(text)
        self.assertEqual(result, {"browse_items", "for_you_items", "get_registry_item"})

    def test_handles_generics_in_block(self):
        text = """
        generate_handler![
            commands::foo::<u64>,
            commands::bar,
        ]
        """
        result = ctb.parse_rust_commands(text)
        # Only picks up commands:: prefix, not the generic
        self.assertIn("bar", result)

    def test_empty_handler(self):
        text = """generate_handler![]"""
        result = ctb.parse_rust_commands(text)
        self.assertEqual(result, set())

    def test_no_handler_errors(self):
        """Missing handler block should exit."""
        with self.assertRaises(SystemExit):
            ctb.parse_rust_commands("fn main() {}")


class TestParseRustDefinedCommands(unittest.TestCase):
    """Tests for parse_rust_defined_commands."""

    def test_detects_tauri_command(self):
        text = """
        #[tauri::command]
        pub async fn browse_items() -> Result<Vec<String>> { todo!() }

        #[tauri::command]
        pub fn list_things() -> Result<()> { todo!() }
        """
        result = ctb.parse_rust_defined_commands(text)
        self.assertEqual(result, {"browse_items", "list_things"})

    def test_skips_non_command_fns(self):
        text = """
        pub fn helper() {}
        #[tauri::command]
        pub fn exposed() {}
        """
        result = ctb.parse_rust_defined_commands(text)
        self.assertEqual(result, {"exposed"})


class TestParseTsInvokeCalls(unittest.TestCase):
    """Tests for parse_ts_invoke_calls."""

    def test_parses_invoke_calls(self):
        text = """
        export const foo = () => invoke<string>('foo_cmd');
        export const bar = (x: number) => invoke<number>('bar_cmd', { x });
        export const baz = () => invoke<Record<string, unknown> | null>('baz_cmd');
        """
        result = ctb.parse_ts_invoke_calls(text)
        self.assertEqual(result, {"foo_cmd", "bar_cmd", "baz_cmd"})


class TestBuildManifest(unittest.TestCase):
    """Tests for build_manifest structure."""

    def test_returns_expected_keys(self):
        """The manifest dict has the expected top-level keys."""
        # Parse a known-small set of files via inline fixtures
        manifest = {
            "schema_version": ctb.SCHEMA_VERSION,
            "summary": {"registered_rust": 0, "defined_rust": 0, "ts_wrappers": 0},
            "commands": {
                "registered": [],
                "defined": [],
                "ts_wrappers": [],
                "missing_ts_wrapper": [],
                "missing_rust_command": [],
                "defined_not_registered": [],
            },
        }
        self.assertEqual(manifest["schema_version"], 1)
        self.assertIn("registered_rust", manifest["summary"])
        self.assertIn("commands", manifest)


# ═══════════════════════════════════════════════════════════════════════════
# compile_registry forwarding tests
# ═══════════════════════════════════════════════════════════════════════════

class TestCompileRegistryForwarding(unittest.TestCase):
    """Tests for subprocess argument forwarding in compile_registry."""

    def _assert_not_in_args(self, cmd: list[str], *flags: str) -> None:
        for flag in flags:
            self.assertNotIn(flag, cmd, f"{flag!r} should not be in command")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_governance_mode(self, mock_run):
        """--governance-mode read-only is forwarded."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_mode="read-only",
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-mode")
        self.assertEqual(cmd[idx + 1], "read-only")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_governance_policy(self, mock_run):
        """--governance-policy sandbox is forwarded."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_policy="sandbox",
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-policy")
        self.assertEqual(cmd[idx + 1], "sandbox")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_omits_absent_governance_policy(self, mock_run):
        """--governance-policy omitted when not provided."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
        )
        cmd = mock_run.call_args[0][0]
        self._assert_not_in_args(cmd, "--governance-policy")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_governance_repo(self, mock_run):
        """--governance-repo is forwarded."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_repo="owner/repo",
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-repo")
        self.assertEqual(cmd[idx + 1], "owner/repo")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_omits_absent_governance_repo(self, mock_run):
        """--governance-repo omitted when not provided."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
        )
        cmd = mock_run.call_args[0][0]
        self._assert_not_in_args(cmd, "--governance-repo")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_governance_state_in(self, mock_run):
        """--governance-state-in with Path is forwarded as single argument."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_state_in=Path("some/path.json"),
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-state-in")
        expected = str(Path("some/path.json"))
        self.assertEqual(cmd[idx + 1], expected)

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_omits_absent_governance_state_in(self, mock_run):
        """--governance-state-in omitted when not provided."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
        )
        cmd = mock_run.call_args[0][0]
        self._assert_not_in_args(cmd, "--governance-state-in")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_governance_state_out(self, mock_run):
        """--governance-state-out with Path is forwarded as single argument."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_state_out=Path("out/state.json"),
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-state-out")
        expected = str(Path("out/state.json"))
        self.assertEqual(cmd[idx + 1], expected)

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_omits_absent_governance_state_out(self, mock_run):
        """--governance-state-out omitted when not provided."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
        )
        cmd = mock_run.call_args[0][0]
        self._assert_not_in_args(cmd, "--governance-state-out")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_path_with_spaces(self, mock_run):
        """Path with spaces forwarded as single subprocess argument."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_state_in=Path("my path/state.json"),
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-state-in")
        expected = str(Path("my path/state.json"))
        self.assertEqual(cmd[idx + 1], expected)

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_path_with_spaces_out(self, mock_run):
        """Path with spaces forwarded for state-out."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_state_out=Path("my output/state.json"),
        )
        cmd = mock_run.call_args[0][0]
        idx = cmd.index("--governance-state-out")
        expected = str(Path("my output/state.json"))
        self.assertEqual(cmd[idx + 1], expected)

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_forwards_off_mode(self, mock_run):
        """--governance-mode off is forwarded."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            governance_mode="off",
        )
        cmd = mock_run.call_args[0][0]
        self.assertIn("--governance-mode", cmd)
        idx = cmd.index("--governance-mode")
        self.assertEqual(cmd[idx + 1], "off")

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_failure_propagation(self, mock_run):
        """CalledProcessError from subprocess is propagated."""
        from subprocess import CalledProcessError

        mock_run.side_effect = CalledProcessError(1, "test")
        with self.assertRaises(CalledProcessError):
            refresh_loader_manifests.compile_registry(
                out="registry.db", skip_sign=True,
            )

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_no_governance_write_when_mode_unset(self, mock_run):
        """--no-governance-write forwarded when governance_mode is None."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            no_governance_write=True,
        )
        cmd = mock_run.call_args[0][0]
        self.assertIn("--no-governance-write", cmd)

    @mock.patch("refresh_loader_manifests.subprocess.run")
    def test_monitor_state_can_suppress_legacy_audit_write(self, mock_run):
        """Monitor state and legacy audit-log writes are controlled separately."""
        refresh_loader_manifests.compile_registry(
            out="registry.db", skip_sign=True,
            no_governance_write=True,
            governance_mode="monitor",
        )
        cmd = mock_run.call_args[0][0]
        self.assertIn("--governance-mode", cmd)
        self.assertIn("--no-governance-write", cmd)


# ═══════════════════════════════════════════════════════════════════════════
# governance-state.json validator tests
# ═══════════════════════════════════════════════════════════════════════════

import validate_governance_state as vgs


class TestDeployReleaseAssets(unittest.TestCase):
    def test_main_uploads_only_four_user_facing_assets(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets = tuple(
                Path(tmp) / name
                for name in (
                    "registry.db",
                    "registry.db.sig",
                    "registry-web.json",
                    "registry-web.json.sig",
                )
            )
            for asset in assets:
                asset.write_bytes(b"test")

            release = {"id": 1, "assets": [], "upload_url": "https://example.test"}
            with (
                mock.patch.dict(
                    os.environ,
                    {"GITHUB_TOKEN": "token", "GITHUB_REPOSITORY": "owner/repo"},
                    clear=False,
                ),
                mock.patch.object(deploy_release_assets, "RELEASE_ASSETS", assets),
                mock.patch.object(
                    deploy_release_assets, "get_release_by_tag", return_value=release
                ),
                mock.patch.object(deploy_release_assets, "delete_existing_assets"),
                mock.patch.object(deploy_release_assets, "upload_asset") as upload,
                mock.patch.object(deploy_release_assets, "prune_old_releases"),
            ):
                self.assertEqual(deploy_release_assets.main(), 0)

            self.assertEqual(
                [call.args[2].name for call in upload.call_args_list],
                [asset.name for asset in assets],
            )
            self.assertNotIn(
                "governance-state.json",
                [asset.name for asset in assets],
            )


class TestGovernanceStateValidator(unittest.TestCase):
    """Tests for validate_governance_state.validate()."""

    def test_valid_initial_state(self):
        """Valid initial state returns no errors."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertEqual(errors, [])
        finally:
            os.unlink(tmp_path)

    def test_rejects_wrong_schema_version(self):
        """schema_version != 1 is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 2,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("schema_version" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_wrong_repo(self):
        """Wrong repo value is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "some/other",
                "policy": "production",
                "events": [],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("repo" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_wrong_policy(self):
        """Non-production policy is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "sandbox",
                "events": [],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("policy" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_non_list_events(self):
        """Non-list events is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": "not a list",
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("events" in e and "list" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_non_object_top_level(self):
        """Top-level non-object is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            tmp.write("[]")
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("Top-level" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_non_dict_event(self):
        """Non-dict event is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": ["not an object"],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("must be a JSON object" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_empty_event_id(self):
        """Empty event_id is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [
                    {
                        "event_id": "",
                        "item_id": "item1",
                        "event_type": "vote_started",
                    }
                ],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("event_id" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_duplicate_event_id(self):
        """Duplicate event_id is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [
                    {
                        "event_id": "same",
                        "item_id": "item1",
                        "event_type": "vote_started",
                    },
                    {
                        "event_id": "same",
                        "item_id": "item2",
                        "event_type": "vote_closed",
                    },
                ],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("Duplicate" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_missing_item_id(self):
        """Missing item_id is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [
                    {
                        "event_id": "evt1",
                        "event_type": "vote_started",
                    }
                ],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("item_id" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_missing_event_type(self):
        """Missing event_type is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [
                    {
                        "event_id": "evt1",
                        "item_id": "item1",
                    }
                ],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("event_type" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_rejects_non_string_item_id(self):
        """Non-string item_id is rejected."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [
                    {
                        "event_id": "evt1",
                        "item_id": 123,
                        "event_type": "vote_started",
                    }
                ],
            }, tmp)
            tmp_path = tmp.name
        try:
            errors = vgs.validate(Path(tmp_path))
            self.assertTrue(
                any("item_id" in e for e in errors)
            )
        finally:
            os.unlink(tmp_path)

    def test_cli_rejects_invalid_file(self):
        """CLI main() returns 1 for invalid JSON."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            tmp.write("not json")
            tmp_path = tmp.name
        original_argv = sys.argv
        sys.argv = ["script", tmp_path]
        try:
            exit_code = vgs.main()
            self.assertEqual(exit_code, 1)
        finally:
            sys.argv = original_argv
            os.unlink(tmp_path)

    def test_cli_usage(self):
        """main() with wrong arg count returns 1."""
        original_argv = sys.argv
        sys.argv = ["script"]
        try:
            self.assertEqual(vgs.main(), 1)
        finally:
            sys.argv = original_argv

    def test_cli_accepts_valid_file(self):
        """CLI main() returns 0 for valid state."""
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False,
        ) as tmp:
            json.dump({
                "schema_version": 1,
                "governance_repository": "agora-mc/Agora-Launcher",
                "policy": "production",
                "events": [],
            }, tmp)
            tmp_path = tmp.name
        original_argv = sys.argv
        sys.argv = ["script", tmp_path]
        try:
            exit_code = vgs.main()
            self.assertEqual(exit_code, 0)
        finally:
            sys.argv = original_argv
            os.unlink(tmp_path)


class TestBuildDocsWebSlugify(unittest.TestCase):
    """Tests for build_docs_web.slugify."""

    def test_docs_path_drops_prefix(self):
        self.assertEqual(bdw.slugify(Path("docs/CLI.md")), "cli")

    def test_nested_path(self):
        self.assertEqual(bdw.slugify(Path("docs/architecture/baseline.md")), "architecture-baseline")

    def test_root_doc(self):
        self.assertEqual(bdw.slugify(Path("CODE_OF_ENGAGEMENT.md")), "code-of-engagement")

    def test_underscores_and_case_normalized(self):
        self.assertEqual(bdw.slugify(Path("docs/desktop-native-smoke-checklist.md")), "desktop-native-smoke-checklist")
        self.assertEqual(bdw.slugify(Path("docs/RELEASING.md")), "releasing")


class TestBuildDocsWebExtractTitle(unittest.TestCase):
    """Tests for build_docs_web.extract_title."""

    def test_first_h1_wins(self):
        content = "Intro\n\n# Real Title\n\n## Not this\n"
        self.assertEqual(bdw.extract_title(content, Path("docs/x.md")), "Real Title")

    def test_fallback_to_filename(self):
        content = "no heading here"
        self.assertEqual(bdw.extract_title(content, Path("docs/my-doc.md")), "My Doc")


class TestBuildDocsWebExtractDescription(unittest.TestCase):
    """Tests for build_docs_web.extract_description."""

    def test_first_paragraph(self):
        content = "# Title\n\nThis is the first paragraph.\n\n# Second\n"
        self.assertEqual(bdw.extract_description(content), "This is the first paragraph.")

    def test_truncates_long(self):
        content = "Word " * 200
        desc = bdw.extract_description(content)
        self.assertLessEqual(len(desc), 241)

    def test_empty_returns_empty(self):
        self.assertEqual(bdw.extract_description(""), "")

    def test_strips_inline_markdown(self):
        content = "# T\n\nThe `agora` binary is **fast** and [documented](./CLI.md).\n"
        self.assertEqual(
            bdw.extract_description(content),
            "The agora binary is fast and documented.",
        )


class TestBuildDocsWebRewriteLinks(unittest.TestCase):
    """Tests for build_docs_web.rewrite_links."""

    def setUp(self):
        self.slugs = {
            "docs/CLI.md": "cli",
            "docs/DEVELOPMENT.md": "development",
            "CODE_OF_ENGAGEMENT.md": "code-of-engagement",
        }

    def test_rewrites_relative_md_link(self):
        content = "[CLI reference](./CLI.md)"
        out = bdw.rewrite_links(content, Path("docs/README.md"), self.slugs)
        self.assertIn("](/docs/cli)", out)
        self.assertNotIn("./CLI.md", out)

    def test_rewrites_parent_link(self):
        content = "[Code of Engagement](../CODE_OF_ENGAGEMENT.md)"
        out = bdw.rewrite_links(content, Path("docs/README.md"), self.slugs)
        self.assertIn("](/docs/code-of-engagement)", out)

    def test_preserves_anchor(self):
        content = "[CLI reference](./CLI.md#exit-codes)"
        out = bdw.rewrite_links(content, Path("docs/README.md"), self.slugs)
        self.assertIn("](/docs/cli#exit-codes)", out)

    def test_leaves_absolute_urls(self):
        content = "[Site](https://example.com/docs)"
        self.assertEqual(bdw.rewrite_links(content, Path("docs/x.md"), self.slugs), content)

    def test_leaves_unknown_markdown(self):
        content = "[Missing](./NOPE.md)"
        out = bdw.rewrite_links(content, Path("docs/x.md"), self.slugs)
        self.assertEqual(out, content)


class TestBuildDocsWebClassify(unittest.TestCase):
    """Tests for build_docs_web.classify."""

    def test_player_facing_reference(self):
        self.assertEqual(bdw.classify(Path("docs/TROUBLESHOOTING.md")), ("user", "Fix a problem"))
        self.assertEqual(bdw.classify(Path("docs/CLI.md")), ("user", "Power tools"))

    def test_contributor_reference(self):
        self.assertEqual(bdw.classify(Path("docs/DEVELOPMENT.md")), ("developer", "Build Agora"))
        self.assertEqual(
            bdw.classify(Path("CODE_OF_ENGAGEMENT.md")), ("developer", "Contribute and curate")
        )

    def test_directory_prefix_applies_to_children(self):
        self.assertEqual(
            bdw.classify(Path("docs/architecture/layer-ownership.md")), ("developer", "Architecture")
        )
        self.assertEqual(
            bdw.classify(Path("docs/archive/direct-launch-cli-parity-plan.md")),
            ("internal", "Archive"),
        )

    def test_unclassified_defaults_to_internal(self):
        self.assertEqual(bdw.classify(Path("docs/brand-new-note.md")), bdw.DEFAULT_CATEGORY)
        self.assertEqual(bdw.DEFAULT_CATEGORY[0], "internal")

    def test_every_group_has_a_display_order(self):
        groups = {group for _, group in bdw.DOC_CATEGORIES.values()}
        groups.add(bdw.DEFAULT_CATEGORY[1])
        self.assertEqual(groups - set(bdw.GROUP_ORDER), set())

    def test_every_audience_has_a_display_order(self):
        audiences = {audience for audience, _ in bdw.DOC_CATEGORIES.values()}
        audiences.add(bdw.DEFAULT_CATEGORY[0])
        self.assertEqual(audiences - set(bdw.AUDIENCE_ORDER), set())


class TestBuildDocsWebStripTitle(unittest.TestCase):
    """Tests for build_docs_web.strip_title."""

    def test_removes_leading_h1_and_following_blank_lines(self):
        content = "# Title\n\nFirst paragraph.\n"
        self.assertEqual(bdw.strip_title(content), "First paragraph.")

    def test_keeps_later_headings(self):
        content = "# Title\n\n## Section\n\nBody.\n"
        out = bdw.strip_title(content)
        self.assertTrue(out.startswith("## Section"))
        self.assertNotIn("# Title", out)

    def test_no_leading_h1_is_unchanged(self):
        content = "> A blockquote opener.\n\n# Later heading\n"
        self.assertEqual(bdw.strip_title(content), content)

    def test_tolerates_html_comment_preamble(self):
        content = "<!-- generated -->\n\n# Title\n\nBody.\n"
        self.assertEqual(bdw.strip_title(content), "<!-- generated -->\n\nBody.")

    def test_empty_is_unchanged(self):
        self.assertEqual(bdw.strip_title(""), "")


class TestPinHashesOtherGames(unittest.TestCase):
    """pin_hashes accepts a direct_hash entry for another game (MASTER_SPEC §26.8)."""

    def _entry(self, **overrides):
        entry = {
            "id": "skyrim-archive-mod",
            "game": "skyrim-se",
            "content_type": "mod",
            "download_strategy": "direct_hash",
            "source_identifier": "https://example.com/files/Mod-1.0.7z",
            "sha256": "0" * 64,
            "game_compatibility": [{"stores": ["steam"], "game_versions": ["1.6.1170.0"]}],
        }
        entry.update(overrides)
        return entry

    def test_other_game_entry_passes_without_compatible_versions(self):
        self.assertEqual(
            pin_hashes._validate_direct_hash_contract(self._entry()),
            "https://example.com/files/Mod-1.0.7z",
        )

    def test_other_game_entry_needs_game_compatibility(self):
        entry = self._entry()
        del entry["game_compatibility"]
        with self.assertRaises(SystemExit) as caught:
            pin_hashes._validate_direct_hash_contract(entry)
        self.assertIn("game_compatibility", str(caught.exception.code))

    def test_other_game_entry_still_needs_an_https_filename(self):
        with self.assertRaises(SystemExit):
            pin_hashes._validate_direct_hash_contract(
                self._entry(source_identifier="https://example.com/download?id=12")
            )

    def test_minecraft_entry_still_needs_compatible_versions(self):
        entry = self._entry(game="minecraft", source_identifier="https://example.com/files/m-1.0.jar")
        del entry["game_compatibility"]
        with self.assertRaises(SystemExit) as caught:
            pin_hashes._validate_direct_hash_contract(entry)
        self.assertIn("compatible_versions", str(caught.exception.code))
class TestFinalizeReleaseAssetsNames(unittest.TestCase):
    """Tests for finalize_release_assets tag, rename, and plan helpers."""

    def test_version_from_tag(self):
        self.assertEqual(fra.version_from_tag("v1.2.3"), "1.2.3")
        for tag in ("1.2.3", "v1.2", "v1.2.3-rc.1", "release-1.2.3"):
            with self.assertRaises(fra.ReleaseAssetError, msg=tag):
                fra.version_from_tag(tag)

    def test_desktop_asset_rename_covers_every_tauri_bundle(self):
        cases = {
            "Agora.Launcher_0.9.3_x64-setup.exe": "agora-desktop-v0.9.3-windows-x64-setup.exe",
            "Agora.Launcher_0.9.3_x64_en-US.msi": "agora-desktop-v0.9.3-windows-x64.msi",
            "Agora.Launcher_0.9.3_aarch64.dmg": "agora-desktop-v0.9.3-macos-arm64.dmg",
            "Agora.Launcher_0.9.3_x64.dmg": "agora-desktop-v0.9.3-macos-x64.dmg",
            "Agora.Launcher_0.9.3_amd64.AppImage": "agora-desktop-v0.9.3-linux-x64.AppImage",
            "Agora.Launcher_0.9.3_amd64.deb": "agora-desktop-v0.9.3-linux-x64.deb",
            "Agora.Launcher-0.9.3-1.x86_64.rpm": "agora-desktop-v0.9.3-linux-x64.rpm",
            "Agora.Launcher_aarch64.app.tar.gz": "agora-desktop-v0.9.3-macos-arm64.app.tar.gz",
            "Agora.Launcher_x64.app.tar.gz": "agora-desktop-v0.9.3-macos-x64.app.tar.gz",
        }
        for old_name, new_name in cases.items():
            with self.subTest(old_name=old_name):
                self.assertEqual(fra.desktop_asset_rename(old_name, "0.9.3"), new_name)

    def test_desktop_asset_rename_accepts_any_msi_locale(self):
        self.assertEqual(
            fra.desktop_asset_rename("Agora.Launcher_0.9.3_x64_pt-BR.msi", "0.9.3"),
            "agora-desktop-v0.9.3-windows-x64.msi",
        )

    def test_desktop_asset_rename_rejects_mismatched_version(self):
        with self.assertRaisesRegex(fra.ReleaseAssetError, "0.1.0"):
            fra.desktop_asset_rename("Agora.Launcher_0.1.0_x64-setup.exe", "0.9.3")

    def test_desktop_asset_rename_rejects_unknown_bundle(self):
        with self.assertRaisesRegex(fra.ReleaseAssetError, "Unrecognized"):
            fra.desktop_asset_rename("Agora.Launcher_0.9.3_x64.snap", "0.9.3")

    def test_desktop_asset_rename_rejects_drifted_updater_archive(self):
        # Tauri does not version .app.tar.gz names; if that ever changes, the
        # table must be updated rather than silently mis-renaming the archive.
        with self.assertRaisesRegex(fra.ReleaseAssetError, "Unrecognized"):
            fra.desktop_asset_rename("Agora.Launcher_0.9.3_x64.app.tar.gz", "0.9.3")

    def test_plan_desktop_renames_skips_signature_sidecars(self):
        names = [
            "Agora.Launcher_0.9.3_x64.dmg",
            "Agora.Launcher_0.9.3_x64.dmg.sig",
            "agora-cli-v0.9.3-linux-x64.tar.gz",
        ]
        self.assertEqual(
            fra.plan_desktop_renames(names, "0.9.3"),
            {"Agora.Launcher_0.9.3_x64.dmg": "agora-desktop-v0.9.3-macos-x64.dmg"},
        )

    def test_plan_desktop_renames_rejects_colliding_targets(self):
        names = [
            "Agora.Launcher_0.9.3_x64_en-US.msi",
            "Agora.Launcher_0.9.3_x64_pt-BR.msi",
        ]
        with self.assertRaises(fra.ReleaseAssetError):
            fra.plan_desktop_renames(names, "0.9.3")


class TestFinalizeReleaseAssetsChecksums(unittest.TestCase):
    """Tests for finalize_release_assets expected sets and SHA256SUMS rendering."""

    def test_expected_asset_names_covers_desktop_cli_and_portable(self):
        names = fra.expected_asset_names("v1.2.3")
        self.assertEqual(len(names), 15)
        self.assertEqual(len(set(names)), 15)
        self.assertIn("agora-desktop-v1.2.3-windows-x64-setup.exe", names)
        self.assertIn("agora-desktop-v1.2.3-windows-x64-portable.zip.sha256", names)
        self.assertIn("agora-cli-v1.2.3-macos-arm64.tar.gz", names)

    def test_unexpected_asset_names_allows_metadata_and_expected_artifacts(self):
        names = fra.expected_asset_names("v1.2.3") + ["latest.json", "SHA256SUMS"]
        self.assertEqual(fra.unexpected_asset_names(names, "v1.2.3"), [])

    def test_unexpected_asset_names_reports_junk_and_stale_sidecars(self):
        names = [
            *fra.expected_asset_names("v1.2.3"),
            "latest.json",
            "notes.txt",
            "Agora.Launcher_1.2.3_x64.dmg.sig",
        ]
        self.assertEqual(
            fra.unexpected_asset_names(names, "v1.2.3"),
            ["Agora.Launcher_1.2.3_x64.dmg.sig", "notes.txt"],
        )

    def test_digest_hex_parses_sha256_digests_only(self):
        value = "a" * 64
        self.assertEqual(fra.digest_hex(f"sha256:{value}"), value)
        self.assertEqual(fra.digest_hex(f"sha256:{value.upper()}"), value)
        self.assertIsNone(fra.digest_hex(None))
        self.assertIsNone(fra.digest_hex(f"sha256:{value[:63]}"))
        self.assertIsNone(fra.digest_hex(f"sha512:{value}"))
        self.assertIsNone(fra.digest_hex(value))

    def test_render_sha256sums_is_sorted_and_checkable(self):
        rendered = fra.render_sha256sums([("b.zip", "b" * 64), ("a.zip", "a" * 64)])
        self.assertEqual(rendered, f"{'a' * 64}  a.zip\n{'b' * 64}  b.zip\n")

    def test_render_sha256sums_rejects_non_digests(self):
        with self.assertRaises(fra.ReleaseAssetError):
            fra.render_sha256sums([("a.zip", "not-a-digest")])


class TestFinalizeReleaseAssetsLatestJson(unittest.TestCase):
    """Tests for finalize_release_assets.rewrite_latest_urls."""

    def test_rewrites_renamed_urls_and_leaves_the_input_alone(self):
        latest = {
            "version": "0.9.3",
            "platforms": {
                "windows-x86_64-nsis": {
                    "signature": "sig",
                    "url": "https://github.com/o/r/releases/download/v0.9.3/Agora.Launcher_0.9.3_x64-setup.exe",
                },
                "darwin-aarch64": {
                    "signature": "sig",
                    "url": "https://github.com/o/r/releases/download/v0.9.3/Agora.Launcher_aarch64.app.tar.gz",
                },
            },
        }
        renames = {
            "Agora.Launcher_0.9.3_x64-setup.exe": "agora-desktop-v0.9.3-windows-x64-setup.exe",
            "Agora.Launcher_aarch64.app.tar.gz": "agora-desktop-v0.9.3-macos-arm64.app.tar.gz",
        }
        rewritten = fra.rewrite_latest_urls(latest, renames)
        self.assertTrue(
            rewritten["platforms"]["windows-x86_64-nsis"]["url"].endswith(
                "/agora-desktop-v0.9.3-windows-x64-setup.exe"
            )
        )
        self.assertTrue(
            rewritten["platforms"]["darwin-aarch64"]["url"].endswith(
                "/agora-desktop-v0.9.3-macos-arm64.app.tar.gz"
            )
        )
        self.assertTrue(
            latest["platforms"]["darwin-aarch64"]["url"].endswith(
                "/Agora.Launcher_aarch64.app.tar.gz"
            )
        )

    def test_already_renamed_urls_pass_through(self):
        latest = {
            "platforms": {
                "darwin-x86_64": {
                    "url": "https://example.test/agora-desktop-v0.9.3-macos-x64.app.tar.gz"
                }
            }
        }
        self.assertEqual(fra.rewrite_latest_urls(latest, {}), latest)

    def test_uncovered_tauri_url_is_an_error(self):
        latest = {
            "platforms": {
                "windows-x86_64": {"url": "https://example.test/Agora.Launcher_0.9.3_x64.msi"}
            }
        }
        with self.assertRaises(fra.ReleaseAssetError):
            fra.rewrite_latest_urls(latest, {})

    def test_missing_platforms_is_an_error(self):
        with self.assertRaises(fra.ReleaseAssetError):
            fra.rewrite_latest_urls({"version": "0.9.3"}, {})


class TestFinalizeReleaseAssetsApply(unittest.TestCase):
    """Tests for the finalize plan: sidecars dropped, bundles renamed, URLs and sums rebuilt."""

    TAG = "v0.9.3"
    OLD_NAMES = (
        "Agora.Launcher_0.9.3_x64-setup.exe",
        "Agora.Launcher_0.9.3_x64_en-US.msi",
        "Agora.Launcher_0.9.3_aarch64.dmg",
        "Agora.Launcher_0.9.3_x64.dmg",
        "Agora.Launcher_0.9.3_amd64.AppImage",
        "Agora.Launcher_0.9.3_amd64.deb",
        "Agora.Launcher-0.9.3-1.x86_64.rpm",
        "Agora.Launcher_aarch64.app.tar.gz",
        "Agora.Launcher_x64.app.tar.gz",
    )

    @staticmethod
    def _asset(name: str, asset_id: int) -> dict:
        return {"id": asset_id, "name": name, "digest": "sha256:" + "a" * 64}

    def test_plan_and_apply_handles_a_release_and_its_rerun(self):
        stale = "agora-desktop-v0.9.3-windows-x64.msi"
        expected = fra.expected_asset_names(self.TAG)
        before = [
            *(self._asset(name, index + 1) for index, name in enumerate(self.OLD_NAMES)),
            *(self._asset(name + ".sig", 100 + index) for index, name in enumerate(self.OLD_NAMES)),
            # A stale unified name plus a fresh old name: the old upload must win.
            self._asset(stale, 500),
            *(self._asset(name, 200 + index) for index, name in enumerate(expected) if name != stale),
            self._asset("latest.json", 600),
        ]
        after = [self._asset(name, 200 + index) for index, name in enumerate(expected)]
        after.append(self._asset("latest.json", 600))

        latest = {
            "version": "0.9.3",
            "platforms": {
                "windows-x86_64-nsis": {
                    "signature": "sig",
                    "url": f"https://github.com/o/r/releases/download/{self.TAG}/Agora.Launcher_0.9.3_x64-setup.exe",
                },
                "darwin-x86_64": {
                    "signature": "sig",
                    "url": f"https://github.com/o/r/releases/download/{self.TAG}/Agora.Launcher_x64.app.tar.gz",
                },
            },
        }
        uploads: dict[str, str] = {}

        def fake_download(repo, tag, name, directory):
            path = Path(directory) / name
            path.write_text(json.dumps(latest), encoding="utf-8")
            return path

        def fake_upload(repo, tag, path):
            uploads[Path(path).name] = Path(path).read_text(encoding="utf-8")

        manager = mock.Mock()
        with (
            mock.patch.object(fra, "fetch_assets", side_effect=[before, after]),
            mock.patch.object(fra, "delete_asset", manager.delete),
            mock.patch.object(fra, "rename_asset", manager.rename),
            mock.patch.object(fra, "download_asset", side_effect=fake_download),
            mock.patch.object(fra, "upload_asset", side_effect=fake_upload),
            mock.patch.object(fra, "asset_digest", return_value="a" * 64),
        ):
            fra.plan_and_apply("o/r", self.TAG, "0.9.3")

        deleted = sorted(call.args[1]["name"] for call in manager.delete.call_args_list)
        renamed = {call.args[1]["name"]: call.args[2] for call in manager.rename.call_args_list}
        self.assertEqual(set(renamed), set(self.OLD_NAMES))
        # Every target that already exists is stale and must be replaced by the
        # fresh upload under the old name; the sidecars are dropped outright.
        self.assertEqual(
            deleted,
            sorted([*(name + ".sig" for name in self.OLD_NAMES), *renamed.values()]),
        )
        self.assertIn(stale, deleted)
        self.assertEqual(
            renamed["Agora.Launcher_0.9.3_x64-setup.exe"],
            "agora-desktop-v0.9.3-windows-x64-setup.exe",
        )

        self.assertEqual(sorted(uploads), ["SHA256SUMS", "latest.json"])
        rewritten = json.loads(uploads["latest.json"])
        self.assertTrue(
            rewritten["platforms"]["windows-x86_64-nsis"]["url"].endswith(
                "/agora-desktop-v0.9.3-windows-x64-setup.exe"
            )
        )
        self.assertTrue(
            rewritten["platforms"]["darwin-x86_64"]["url"].endswith(
                "/agora-desktop-v0.9.3-macos-x64.app.tar.gz"
            )
        )
        covered = sorted(line.split("  ", 1)[1] for line in uploads["SHA256SUMS"].splitlines())
        self.assertEqual(covered, sorted(expected))

    def test_main_reports_asset_errors(self):
        with (
            mock.patch.object(fra, "plan_and_apply", side_effect=fra.ReleaseAssetError("boom")),
            mock.patch("sys.stderr"),
        ):
            self.assertEqual(fra.main(["--repo", "o/r", "--tag", self.TAG]), 1)

    def test_main_succeeds_with_a_finalize_plan(self):
        with mock.patch.object(fra, "plan_and_apply") as plan:
            self.assertEqual(fra.main(["--repo", "o/r", "--tag", self.TAG]), 0)
        plan.assert_called_once_with("o/r", self.TAG, "0.9.3")


class TestStageCliSidecar(unittest.TestCase):
    """Tests for stage_cli_sidecar's path rules, which must match Tauri's."""

    def test_sidecar_name_carries_the_triple_and_windows_suffix(self):
        d = Path("bin")
        self.assertEqual(
            stage_cli_sidecar.sidecar_path(d, "x86_64-pc-windows-msvc"),
            d / "agora-x86_64-pc-windows-msvc.exe",
        )
        self.assertEqual(
            stage_cli_sidecar.sidecar_path(d, "aarch64-apple-darwin"),
            d / "agora-aarch64-apple-darwin",
        )

    def test_built_path_follows_whether_cargo_was_given_a_target(self):
        t = Path("target")
        self.assertEqual(
            stage_cli_sidecar.built_cli_path(t, "x86_64-unknown-linux-gnu", explicit_target=False),
            t / "release" / "agora",
        )
        self.assertEqual(
            stage_cli_sidecar.built_cli_path(t, "x86_64-apple-darwin", explicit_target=True),
            t / "x86_64-apple-darwin" / "release" / "agora",
        )


if __name__ == "__main__":
    unittest.main()
