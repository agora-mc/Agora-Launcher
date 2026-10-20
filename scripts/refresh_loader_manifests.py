#!/usr/bin/env python3
"""Refresh loader manifests, compile the registry database, and verify hashes.

Auto-discovers available stable Minecraft versions from Mojang's official
version manifest,
optionally limited to the latest N or versions released since a given version.
You can still pass an explicit list with --mc-versions.

Usage examples:
    python scripts/refresh_loader_manifests.py --skip-sign
    python scripts/refresh_loader_manifests.py --auto-versions --latest 5 --skip-sign
    python scripts/refresh_loader_manifests.py --auto-versions --skip-sign      # all stable MC versions (default)
    python scripts/refresh_loader_manifests.py --mc-versions 1.21 1.20.6 --skip-sign
    python scripts/refresh_loader_manifests.py --auto-versions --since 1.20 --skip-sign
"""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import re
import subprocess
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "scripts"))

import fetch_loader_manifests as fetch  # noqa: E402

COMPILE_SCRIPT = REPO_ROOT / "compiler" / "compile.py"
LOADER_MANIFESTS_PATH = REPO_ROOT / "loader-manifests" / "loader_manifests.json"

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger("refresh_loader_manifests")


def _is_standard_release(version: str) -> bool:
    """Return True for clean numeric versions like 1.21, 1.21.4, 26.1, 26.1.1.

    Ignores snapshots (snapshot/alpha/beta/combat tags, dated versions like
    26w13a, suffixed rc/pre builds). This is a secondary safety net on top of
    Fabric's ``stable`` flag — both must pass for a version to be included.
    """
    return bool(re.fullmatch(r"\d+\.\d+(?:\.\d+)?", version))


def discover_stable_mc_versions(
    since: str | None = None, latest: int | None = None
) -> list[str]:
    """Return stable Minecraft versions from Mojang's official version manifest.

    Mojang's manifest at https://piston-meta.mojang.com/mc/game/version_manifest_v2.json
    lists every version ever released (1.0 onward). We filter to type=='release'
    and apply the existing _is_standard_release() regex (drops snapshots,
    prereleases, experimentals). Versions are listed newest-first by Mojang.
    """
    url = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
    logger.info("Discovering stable Minecraft versions from Mojang: %s", url)
    try:
        data: dict[str, Any] = fetch._fetch_json(url)
    except (OSError, ValueError) as exc:
        raise fetch.UpstreamMetadataError(f"Mojang version metadata: {exc}") from exc

    if not isinstance(data, dict) or not isinstance(data.get("versions"), list):
        raise fetch.UpstreamMetadataError("Mojang version metadata was malformed")

    stable = [
        v["id"]
        for v in data.get("versions", [])
        if v.get("type") == "release" and _is_standard_release(v["id"])
    ]

    if since:
        since_key = fetch._version_key(since)
        stable = [v for v in stable if fetch._version_key(v) >= since_key]

    if latest:
        stable = stable[:latest]

    if not stable:
        raise fetch.UpstreamMetadataError(
            "Mojang version metadata contained no stable Minecraft versions"
        )

    logger.info("Discovered %d stable Minecraft versions (%s ... %s)",
                len(stable), stable[-1] if stable else "?", stable[0] if stable else "?")
    return stable


def _manifest_entries(manifest: dict[str, Any]) -> Iterable[tuple[str, dict[str, Any]]]:
    for loader, entries in manifest.get("loaders", {}).items():
        if isinstance(entries, list):
            for entry in entries:
                if isinstance(entry, dict):
                    yield loader, entry


def _entry_key(loader: str, entry: dict[str, Any]) -> str:
    return f"{loader}/{entry.get('mc_version')}/{entry.get('loader_version')}"


def _manifest_delta(
    before: dict[str, Any], after: dict[str, Any]
) -> tuple[list[tuple[str, dict[str, Any]]], list[str], list[str]]:
    before_by_key = {
        _entry_key(loader, entry): entry
        for loader, entry in _manifest_entries(before)
    }
    after_by_key = {
        _entry_key(loader, entry): entry
        for loader, entry in _manifest_entries(after)
    }
    changed = [
        (loader, entry)
        for loader, entry in _manifest_entries(after)
        if _entry_key(loader, entry) not in before_by_key
        or before_by_key[_entry_key(loader, entry)] != entry
    ]
    added = [key for key in after_by_key if key not in before_by_key]
    deleted = [key for key in before_by_key if key not in after_by_key]
    return changed, added, deleted


def _write_report(path: Path | None, report: dict[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _verification_targets(
    manifest: dict[str, Any], changed_keys: set[str], sample_size: int | None
) -> list[tuple[str, dict[str, Any]]]:
    """Changed entries plus a slice of the rest that rotates by date.

    The fetch step only re-reads the newest versions of each loader, so the
    rotating slice is what notices upstream changing an older pinned file;
    every entry is reached once per len(entries) / sample_size days.
    """
    entries = sorted(_manifest_entries(manifest), key=lambda pair: _entry_key(*pair))
    if sample_size is None or sample_size >= len(entries):
        return entries
    targets = [pair for pair in entries if _entry_key(*pair) in changed_keys]
    if sample_size > 0 and entries:
        day = datetime.now(timezone.utc).date().toordinal()
        start = (day * sample_size) % len(entries)
        rotated = entries[start:] + entries[:start]
        targets += [
            pair for pair in rotated[:sample_size]
            if _entry_key(*pair) not in changed_keys
        ]
    return targets


def _cache_name(source_url: str) -> str:
    cache_name = source_url.split("/")[-1]
    # Profile JSON URLs all end in /profile/json; give them unique cache names
    # so we don't reuse the same cached file for every entry.
    if not cache_name.endswith(".jar"):
        ext = os.path.splitext(cache_name)[1] or ".bin"
        cache_name = hashlib.sha256(source_url.encode("utf-8")).hexdigest()[:32] + ext
    return cache_name


def _file_fields(entry: dict[str, Any], path: Path) -> dict[str, Any]:
    """The file-derived fields of an entry, recomputed from a downloaded file."""
    data = path.read_bytes()
    if entry.get("file_type") == "profile_json":
        # Profile/version JSON is hashed with volatile keys stripped.
        return {"sha256": fetch._stable_json_sha256(data)}
    fields: dict[str, Any] = {"sha256": fetch._sha256_hex(data)}
    if entry.get("file_type") == "installer_jar":
        fields["content_sha256"] = fetch.installer_content_sha256(path)
        fields["version_json_sha256"] = fetch._extract_version_json_sha256(path)
        profile = fetch._extract_install_profile(path) or {}
        if isinstance(profile.get("spec"), int):
            fields["installer_spec"] = profile["spec"]
    return fields


def verify_manifest(
    changed_keys: Iterable[str] = (),
    sample_size: int | None = None,
) -> dict[str, Any]:
    """Re-download pinned loader files and reconcile them with their pins.

    Checks the entries this run added or changed plus a rotating sample of the
    rest (sample_size=None checks every entry). A file upstream now serves
    differently is re-pinned when it is a repack and quarantined otherwise,
    exactly as during a refresh, so one changed file never blocks the catalog.
    A file that cannot be downloaded is reported and checked again later.
    """
    manifest = fetch._load_existing_manifest()
    targets = _verification_targets(manifest, set(changed_keys), sample_size)
    logger.info("Verifying %d pinned loader file(s)", len(targets))
    summary: dict[str, Any] = {
        "checked": len(targets), "ok": 0, "repinned": [], "quarantined": [], "errors": [],
    }
    modified = False

    for loader, entry in targets:
        key = _entry_key(loader, entry)
        try:
            cache_path = fetch._download_to_cache(entry["source_url"], _cache_name(entry["source_url"]))
            upstream = _file_fields(entry, cache_path)
        except Exception as exc:  # noqa: BLE001
            logger.warning("Could not verify %s: %s", key, exc)
            summary["errors"].append(key)
            continue

        if upstream["sha256"] == entry["sha256"]:
            summary["ok"] += 1
            # Fingerprint older jars as they come round, so a later repack of
            # them is recognised without review too.
            if upstream.get("content_sha256") and entry.get("content_sha256") is None:
                entry["content_sha256"] = upstream["content_sha256"]
                modified = True
            continue

        modified = True
        if fetch._is_repack(entry, upstream):
            fetch._rotate_file_fields(entry, upstream, "repacked upstream; contents unchanged")
            summary["repinned"].append(key)
        else:
            fetch.quarantine_entry(
                loader, entry, upstream,
                "upstream file changed and its contents could not be shown unchanged",
            )
            manifest["loaders"][loader].remove(entry)
            summary["quarantined"].append(key)

    if modified:
        fetch._write_loader_manifests(manifest)
        fetch._write_known_good_hashes(manifest)
    logger.info(
        "Verification complete: %d checked, %d OK, %d re-pinned, %d quarantined, %d unreachable",
        summary["checked"], summary["ok"], len(summary["repinned"]),
        len(summary["quarantined"]), len(summary["errors"]),
    )
    return summary


def fetch_loader_manifests(
    mc_versions: list[str], per_mc_limit: int | None, keep_others: bool = False,
) -> None:
    """Run the same logic as fetch_loader_manifests.py from this process."""
    logger.info("Fetching loader manifests for Minecraft versions: %s", mc_versions)

    manifest = fetch._load_existing_manifest()
    manifest["domain_allowlist"] = sorted(
        set(manifest.get("domain_allowlist", []) + fetch.DOMAIN_ALLOWLIST)
    )
    loaders = manifest.setdefault("loaders", {})
    for loader in ("fabric", "quilt", "neoforge", "forge"):
        loaders.setdefault(loader, [])

    if keep_others:
        logger.info(
            "Refreshes are append-only; retaining existing entries outside the target list"
        )
    else:
        logger.warning(
            "Refreshes are append-only; --keep-others is retained only for CLI compatibility"
        )

    for mc_version in mc_versions:
        logger.info("Fetching Fabric versions for %s", mc_version)
        loaders["fabric"] = fetch._merge_entries(
            loaders["fabric"],
            fetch._fetch_fabric(
                mc_version,
                per_mc_limit,
                refresh_profiles=True,
            ),
            loader="fabric",
        )

        logger.info("Fetching Quilt versions for %s", mc_version)
        loaders["quilt"] = fetch._merge_entries(
            loaders["quilt"],
            fetch._fetch_quilt(
                mc_version,
                per_mc_limit,
                refresh_profiles=True,
            ),
            loader="quilt",
        )

    logger.info("Fetching NeoForge versions for %s", mc_versions)
    loaders["neoforge"] = fetch._merge_entries(
        loaders["neoforge"],
        fetch._fetch_neoforge(mc_versions, per_mc_limit),
        loader="neoforge",
    )

    logger.info("Fetching Forge versions for %s", mc_versions)
    loaders["forge"] = fetch._merge_entries(
        loaders["forge"],
        fetch._fetch_forge(mc_versions, per_mc_limit),
        loader="forge",
    )

    fetch._write_loader_manifests(manifest)
    fetch._write_known_good_hashes(manifest)

    total = sum(len(entries) for entries in loaders.values())
    logger.info("Done. %d loader entries written", total)


def compile_registry(
    out: str, skip_sign: bool, no_governance_write: bool = False,
    governance_mode: str | None = None,
    governance_policy: str | None = None,
    governance_repo: str | None = None,
    governance_state_in: Path | None = None,
    governance_state_out: Path | None = None,
) -> None:
    """Run compiler/compile.py as a subprocess."""
    cmd = [sys.executable, str(COMPILE_SCRIPT), "--out", out]
    if skip_sign:
        cmd.append("--skip-sign")
    if governance_mode:
        cmd.extend(["--governance-mode", governance_mode])
    if no_governance_write:
        cmd.append("--no-governance-write")
    if governance_policy:
        cmd.extend(["--governance-policy", governance_policy])
    if governance_repo:
        cmd.extend(["--governance-repo", governance_repo])
    if governance_state_in is not None:
        cmd.extend(["--governance-state-in", str(governance_state_in)])
    if governance_state_out is not None:
        cmd.extend(["--governance-state-out", str(governance_state_out)])

    logger.info("Running compiler: %s", " ".join(cmd))
    subprocess.run(cmd, cwd=REPO_ROOT, check=True)


def _notify_quarantine(keys: list[str]) -> None:
    """Tell maintainers on Discord about newly quarantined loader files.

    Best effort: the run has already done the safe thing by holding the files
    back, so a failed notification is logged and nothing more.
    """
    webhook_url = os.environ.get("DISCORD_WEBHOOK_URL")
    if not keys or not webhook_url:
        return
    lines = "\n".join(f"- `{key}`" for key in keys[:20])
    more = f"\n…and {len(keys) - 20} more" if len(keys) > 20 else ""
    message = (
        "**Loader files quarantined.** Upstream changed these pinned files and "
        "Agora could not show the contents unchanged, so they are hidden from "
        "users until reviewed. Approve them in `loader-manifests/quarantine.json`.\n"
        f"{lines}{more}"
    )
    request = urllib.request.Request(
        webhook_url,
        data=json.dumps({"content": message[:1900]}).encode("utf-8"),
        headers={"Content-Type": "application/json", "User-Agent": "AgoraLoaderManifestBot/1.0"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=15):
            pass
    except Exception as exc:  # noqa: BLE001
        logger.warning("Could not post the quarantine notice to Discord: %s", type(exc).__name__)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Refresh loader manifests, compile the registry, and verify hashes."
    )
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--mc-versions",
        nargs="+",
        help="Explicit Minecraft versions to refresh (default: 1.21)",
    )
    source.add_argument(
        "--auto-versions",
        action="store_true",
        help="Auto-discover stable Minecraft versions from Mojang's official version manifest",
    )
    parser.add_argument(
        "--latest",
        type=int,
        help="With --auto-versions, keep only the N latest stable versions (default: all)",
    )
    parser.add_argument(
        "--since",
        help="With --auto-versions, keep only versions >= this (e.g. 1.20)",
    )
    parser.add_argument(
        "--latest-per-mc",
        type=int,
        default=5,
        help="Query at most N new loader versions per Minecraft version (default: 5); existing entries are retained",
    )
    parser.add_argument(
        "--keep-others",
        action="store_true",
        help="Compatibility flag; unattended refreshes are always append-only",
    )
    parser.add_argument(
        "--out",
        default="registry.db",
        help="Output path for the compiled registry database (default: registry.db)",
    )
    parser.add_argument(
        "--skip-sign",
        action="store_true",
        help="Write a placeholder signature instead of a real Ed25519 signature",
    )
    parser.add_argument(
        "--no-verify",
        action="store_true",
        help="Skip loader artifact hash verification",
    )
    parser.add_argument(
        "--verify-changed-only",
        action="store_true",
        help="Verify only newly added or changed entries instead of the full catalog",
    )
    parser.add_argument(
        "--verify-sample",
        type=int,
        default=None,
        metavar="N",
        help="Verify changed entries plus N others, rotating daily through the catalog",
    )
    parser.add_argument(
        "--no-governance-write",
        action="store_true",
        help="Do not append or rotate the legacy registry/governance/audit_log.json",
    )
    parser.add_argument(
        "--governance-mode",
        type=str,
        choices=["off", "read-only", "monitor"],
        default=None,
        help="Governance mode for the compiler subprocess (default: unset preserves legacy behavior; use 'off' to disable)",
    )
    parser.add_argument(
        "--governance-policy",
        type=str,
        choices=["production", "sandbox"],
        default=None,
        help="Governance policy for the compiler subprocess (default: unset uses compiler default 'production')",
    )
    parser.add_argument(
        "--governance-repo",
        type=str,
        default=None,
        help="GitHub owner/repo for governance issues (default: unset uses compiler logic)",
    )
    parser.add_argument(
        "--governance-state-in",
        type=Path,
        default=None,
        help="Path to input governance-state.json (default: unset = fresh state in compiler)",
    )
    parser.add_argument(
        "--governance-state-out",
        type=Path,
        default=None,
        help="Path to output governance-state.json (default: unset uses compiler default)",
    )
    parser.add_argument(
        "--report",
        type=Path,
        help="Write a machine-readable refresh report to this path",
    )
    args = parser.parse_args()

    report: dict[str, Any] = {
        "metadata_sources_failed": [],
        "candidate_download_failures": [],
        "existing_entries_mutated": [],
        "unexpected_deletions": [],
        "new_entries": 0,
        "retired_candidates": [],
    }
    try:
        # Load the persisted three-strike tally so retired candidates are
        # skipped without a request; it is rewritten at the end of the run.
        fetch.load_failed_candidates()
        retired_at_start = fetch.get_retired_candidates()
        if retired_at_start:
            logger.info(
                "Skipping %d retired loader candidate(s): %s",
                len(retired_at_start),
                ", ".join(retired_at_start[:5])
                + (" …" if len(retired_at_start) > 5 else ""),
            )
        fetch.load_quarantine()

        before = fetch._load_existing_manifest()

        if args.auto_versions:
            # Default to ALL stable versions; user can pass --latest N to slice
            # or --since X to keep only versions newer than X.
            mc_versions = discover_stable_mc_versions(
                since=args.since, latest=args.latest
            )
        elif args.mc_versions:
            mc_versions = args.mc_versions
        else:
            mc_versions = ["1.21"]

        logger.info("Target Minecraft versions: %s", mc_versions)

        per_mc_limit: int | None = (
            None if args.latest_per_mc <= 0 else args.latest_per_mc
        )
        fetch_loader_manifests(
            mc_versions, per_mc_limit, keep_others=args.keep_others
        )

        after = json.loads(LOADER_MANIFESTS_PATH.read_text(encoding="utf-8"))
        changed_entries, new_entries, deleted_entries = _manifest_delta(before, after)
        # Quarantined entries leave the catalog on purpose; anything else that
        # disappeared is a bug in the refresh.
        deleted_entries = [
            key for key in deleted_entries if key not in fetch.quarantined_keys()
        ]
        report["new_entries"] = len(new_entries)
        report["unexpected_deletions"] = deleted_entries
        report["candidate_download_failures"] = fetch.get_download_failures()

        if deleted_entries:
            logger.error("Refresh deleted existing loader entries: %s", deleted_entries)
            _write_report(args.report, report)
            return 1

        if not args.no_verify:
            sample_size = 0 if args.verify_changed_only else args.verify_sample
            report["verification"] = verify_manifest(
                (_entry_key(loader, entry) for loader, entry in changed_entries),
                sample_size,
            )

        fetch.save_quarantine()
        report["newly_quarantined"] = fetch.get_new_quarantines()
        report["quarantined"] = sorted(fetch.quarantined_keys())
        _notify_quarantine(report["newly_quarantined"])

        compile_registry(
            args.out,
            args.skip_sign,
            no_governance_write=args.no_governance_write,
            governance_mode=args.governance_mode,
            governance_policy=args.governance_policy,
            governance_repo=args.governance_repo,
            governance_state_in=args.governance_state_in,
            governance_state_out=args.governance_state_out,
        )

        if args.auto_versions:
            # Write this only after all upstream and artifact checks pass so a
            # failed refresh does not leave an unrelated tracked-file change.
            mc_versions_path = (
                Path(__file__).resolve().parent.parent
                / "loader-manifests"
                / "minecraft_versions.json"
            )
            mc_versions_path.write_text(
                json.dumps(mc_versions, indent=2) + "\n", encoding="utf-8"
            )
            logger.info("Wrote %d Minecraft versions to %s", len(mc_versions), mc_versions_path)

        failed_path = fetch.save_failed_candidates()
        retired_now = fetch.get_retired_candidates()
        logger.info(
            "Wrote %d failing candidate(s) (%d retired) to %s",
            len(fetch.get_failed_candidates()),
            len(retired_now),
            failed_path,
        )
        report["retired_candidates"] = retired_now

        _write_report(args.report, report)
        return 0
    except fetch.ExistingEntryMutationError as exc:
        report["existing_entries_mutated"] = exc.mutations
        logger.error("%s", exc)
        _write_report(args.report, report)
        return 1
    except fetch.UpstreamMetadataError as exc:
        report["metadata_sources_failed"] = [str(exc)]
        logger.error("%s", exc)
        _write_report(args.report, report)
        return 1
    except Exception as exc:  # noqa: BLE001
        report["refresh_error"] = str(exc)
        logger.exception("Loader refresh failed")
        _write_report(args.report, report)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
