#!/usr/bin/env python3
"""Generate loader manifests and pinned hashes from official upstream APIs.

Each refresh queries at most the requested number of new versions per
Minecraft version and merges them append-only into the existing catalog.

Usage:
    python scripts/fetch_loader_manifests.py [--mc-versions 1.21 1.20.1]
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import logging
import re
import time
import urllib.error
import urllib.request
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from xml.etree import ElementTree as ET

DEFAULT_MC_VERSIONS = ["1.21"]

REPO_ROOT = Path(__file__).resolve().parent.parent
LOADER_MANIFESTS_DIR = REPO_ROOT / "loader-manifests"
CACHE_DIR = REPO_ROOT / ".cache" / "loader-manifests"

DOMAIN_ALLOWLIST = [
    "meta.fabricmc.net",
    "maven.fabricmc.net",
    "maven.minecraftforge.net",
    "neoforged.net",
    "repo1.maven.org",
    "maven.neoforged.net",
    "meta.quiltmc.org",
    "maven.quiltmc.org",
    "minecraftforge.net",
    "files.minecraftforge.net",
]

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger("fetch_loader_manifests")


class UpstreamMetadataError(RuntimeError):
    """Raised when a loader metadata source is unavailable or malformed."""


class ExistingEntryMutationError(ValueError):
    """Raised when upstream changes an already-published loader tuple."""

    def __init__(self, mutations: list[dict[str, Any]]) -> None:
        self.mutations = mutations
        super().__init__(
            "Existing loader entries changed; manual review is required: "
            + json.dumps(mutations, sort_keys=True)
        )


IMMUTABLE_ENTRY_FIELDS = (
    "mc_version",
    "loader_version",
    "source_url",
    "sha256",
    "file_name",
    "file_type",
    "version_json_sha256",
    "installer_spec",
)

# Optional enrichment fields. A legacy entry missing one of these fields may
# be enriched by the refresh; once a value exists, changing it is a mutation.
ENRICHABLE_ENTRY_FIELDS = (
    "provided_versions",
    "release_channel",
    "recommendation_rank",
)

# The capability each loader family's distribution provides under its own id.
# This is the identity written for new entries.
DISTRIBUTION_CAPABILITY = {
    "fabric": "fabricloader",
    "quilt": "quilt_loader",
    "forge": "forge",
    "neoforge": "neoforge",
}

FORGE_LANGUAGE_PROVIDERS = ("javafml", "lowcodefml")


# ---------------------------------------------------------------------------
# Utility helpers
# ---------------------------------------------------------------------------


def _version_key(v: str):
    """Return a sortable key for a dotted version string."""
    parts = re.split(r"[.\-+]", v)
    out: list[Any] = []
    for part in parts:
        try:
            out.append(int(part))
        except ValueError:
            out.append(part.lower())
    return out


def _release_channel(version: str) -> str:
    """Conservatively classify a loader version's release channel.

    A dotted-numeric version with optional build metadata (e.g. ``0.18.6``,
    ``0.4.0+build.112``) is ``stable``. A hyphenated prerelease component or a
    non-numeric component is ``prerelease``. Build metadata alone does not
    lower SemVer precedence and is not a prerelease marker.
    """
    semantic_core = version.split("+", 1)[0]
    if re.fullmatch(r"\d+(\.\d+)+", semantic_core):
        return "stable"
    return "prerelease"


def _enrich_entry(loader: str, entry: dict[str, Any]) -> dict[str, Any]:
    """Add optional enrichment fields to an entry without changing values.

    Adds ``provided_versions`` (the distribution identity) and a conservative
    ``release_channel`` when absent. Existing values are never overwritten.
    """
    capability = DISTRIBUTION_CAPABILITY.get(loader)
    if capability is not None:
        provided = entry.get("provided_versions")
        if provided is None:
            provided = {}
            entry["provided_versions"] = provided
        if (
            isinstance(provided, dict)
            and capability not in provided
            and entry.get("loader_version")
        ):
            provided[capability] = entry["loader_version"]
    if "release_channel" not in entry and entry.get("loader_version"):
        entry["release_channel"] = _release_channel(entry["loader_version"])
    return entry


def _enrichment_absent(entry: dict[str, Any], field: str) -> bool:
    """Whether an enrichment field is effectively absent (may be enriched)."""
    value = entry.get(field)
    if field == "provided_versions":
        return value in (None, {})
    return value is None


def _sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _extract_install_profile(jar_path: Path) -> dict[str, Any] | None:
    """Extract and parse install_profile.json from a Forge/NeoForge installer JAR."""
    try:
        with zipfile.ZipFile(jar_path, "r") as zf:
            if "install_profile.json" in zf.namelist():
                data = zf.read("install_profile.json")
                return json.loads(data.decode("utf-8"))
    except (zipfile.BadZipFile, OSError, json.JSONDecodeError) as exc:
        logger.warning("Could not read install_profile.json from %s: %s", jar_path.name, exc)
    return None


def _extract_version_json(jar_path: Path) -> dict[str, Any] | None:
    """Extract and parse version.json from a Forge/NeoForge installer JAR."""
    try:
        with zipfile.ZipFile(jar_path, "r") as zf:
            if "version.json" in zf.namelist():
                data = zf.read("version.json")
                return json.loads(data.decode("utf-8"))
    except (zipfile.BadZipFile, OSError, json.JSONDecodeError) as exc:
        logger.warning("Could not read version.json from %s: %s", jar_path.name, exc)
    return None


def _forge_language_capabilities(loader_version: str) -> dict[str, str]:
    """Return Forge's documented built-in language-provider versions.

    Forge defines both javafml and lowcodefml as the major Forge version. This
    derives only the major component, never the full distribution version.
    """
    major = loader_version.split(".", 1)[0]
    if not major.isascii() or not major.isdigit():
        return {}
    return {provider: major for provider in FORGE_LANGUAGE_PROVIDERS}


def _extract_neoforge_language_capabilities(
    version_json: dict[str, Any] | None,
) -> dict[str, str]:
    """Extract NeoForge provider capabilities from its pinned version profile.

    Newer profiles may publish a direct ``languageProviders`` map. The 1.21.1
    profile instead pins an FML loader Maven coordinate. FML's built-in
    providers report their containing FML JAR version, so that exact pinned
    coordinate is the authoritative version for both javafml and lowcodefml.
    Unknown shapes fail closed and leave the capability absent.
    """
    if not isinstance(version_json, dict):
        return {}
    direct = version_json.get("languageProviders")
    if isinstance(direct, dict):
        capabilities: dict[str, str] = {}
        for provider in FORGE_LANGUAGE_PROVIDERS:
            value = direct.get(provider)
            if isinstance(value, str) and value.strip():
                capabilities[provider] = value.strip()
            elif isinstance(value, dict):
                version = value.get("version")
                if isinstance(version, str) and version.strip():
                    capabilities[provider] = version.strip()
        if capabilities:
            return capabilities

    libraries = version_json.get("libraries")
    if not isinstance(libraries, list):
        return {}
    for library in libraries:
        if not isinstance(library, dict):
            continue
        coordinate = library.get("name")
        if not isinstance(coordinate, str):
            continue
        parts = coordinate.split(":")
        if len(parts) < 3 or parts[0] != "net.neoforged.fancymodloader" or parts[1] != "loader":
            continue
        fml_version = parts[2].strip()
        if fml_version:
            return {provider: fml_version for provider in FORGE_LANGUAGE_PROVIDERS}
    return {}


def _stable_json_sha256(data: bytes, drop: set[str] | None = None) -> str:
    """Return a deterministic SHA-256 of a JSON payload after stripping volatile keys.

    Fabric dynamically rewrites `time`/`releaseTime` on every request, so pinning
    the raw response is unstable. This normalizes the payload for verification.
    """
    drop = drop or {"time", "releaseTime"}
    obj = json.loads(data.decode("utf-8"))
    if isinstance(obj, dict):
        obj = {k: v for k, v in obj.items() if k not in drop}
    canonical = json.dumps(
        obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )
    return _sha256_hex(canonical.encode("utf-8"))


# ---------------------------------------------------------------------------
# Candidate download failures: a persisted three-strike tally
#
# A transient upstream outage must not permanently blacklist a published loader
# version, so a single failure is never enough to skip anything. But retrying a
# version that has never once resolved wastes a request on every run forever,
# so failures accumulate strikes in a file committed alongside the other loader
# manifests (the runner is ephemeral; the repository is not).
#
#   strike 1-2  -> still retried next run, recorded as "failing"
#   strike 3+   -> "retired": skipped without a request until it succeeds again
#
# Any successful download clears the entry outright, so a version that starts
# resolving again is immediately back in rotation. That is what makes retiring
# safe: it is reversible the moment upstream recovers.
# ---------------------------------------------------------------------------

FAILED_CANDIDATES_PATH = LOADER_MANIFESTS_DIR / "failed_candidates.json"

#: Strikes at which a candidate stops being requested.
RETIREMENT_STRIKES = 3

_DOWNLOAD_FAILURES: list[dict[str, str]] = []
#: key -> {"strikes": int, "first_failed": iso, "last_failed": iso, "status": str}
_FAILED_STATE: dict[str, dict[str, Any]] = {}
#: Keys that resolved this run, so their entries are dropped when saving.
_RECOVERED_KEYS: set[str] = set()


def load_failed_candidates(path: Path | None = None) -> dict[str, dict[str, Any]]:
    """Load the persisted strike tally. A missing or malformed file is empty."""
    global _FAILED_STATE, _RECOVERED_KEYS
    target = path or FAILED_CANDIDATES_PATH
    _RECOVERED_KEYS = set()
    try:
        raw = json.loads(target.read_text(encoding="utf-8"))
        candidates = raw.get("candidates")
        _FAILED_STATE = candidates if isinstance(candidates, dict) else {}
    except (OSError, ValueError):
        _FAILED_STATE = {}
    return _FAILED_STATE


def save_failed_candidates(path: Path | None = None) -> Path:
    """Write the updated tally, dropping anything that recovered this run."""
    target = path or FAILED_CANDIDATES_PATH
    surviving = {
        key: value
        for key, value in _FAILED_STATE.items()
        if key not in _RECOVERED_KEYS
    }
    target.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema_version": 1,
        "retirement_strikes": RETIREMENT_STRIKES,
        "candidates": dict(sorted(surviving.items())),
    }
    target.write_text(
        json.dumps(payload, indent=2, sort_keys=False) + "\n", encoding="utf-8"
    )
    return target


def get_failed_candidates() -> dict[str, dict[str, Any]]:
    """Current tally, including entries that recovered this run."""
    return dict(_FAILED_STATE)


def get_retired_candidates() -> list[str]:
    """Keys currently skipped without a request."""
    return sorted(
        key
        for key, value in _FAILED_STATE.items()
        if key not in _RECOVERED_KEYS
        and int(value.get("strikes", 0)) >= RETIREMENT_STRIKES
    )


def get_download_failures() -> list[dict[str, str]]:
    """Return candidate download failures observed during this process."""
    return list(_DOWNLOAD_FAILURES)


def _failed_key(loader: str, mc_version: str, loader_version: str) -> str:
    return f"{loader}/{mc_version}/{loader_version}"


def _failed_should_skip(key: str) -> bool:
    entry = _FAILED_STATE.get(key)
    if not entry:
        return False
    return int(entry.get("strikes", 0)) >= RETIREMENT_STRIKES


def _failed_record_success(key: str) -> None:
    # Recovery clears the tally, so a version that comes back is retried
    # normally from the next run onward.
    if key in _FAILED_STATE:
        _RECOVERED_KEYS.add(key)


def _failed_record_failure(key: str) -> None:
    if not any(failure["key"] == key for failure in _DOWNLOAD_FAILURES):
        _DOWNLOAD_FAILURES.append({"key": key})
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    entry = _FAILED_STATE.get(key)
    # A failure after a success in the same run means it is not recovered.
    _RECOVERED_KEYS.discard(key)
    if entry is None:
        entry = {"strikes": 0, "first_failed": now}
    entry["strikes"] = int(entry.get("strikes", 0)) + 1
    entry["last_failed"] = now
    entry["status"] = (
        "retired" if entry["strikes"] >= RETIREMENT_STRIKES else "failing"
    )
    _FAILED_STATE[key] = entry
    logger.warning("Could not download candidate %s; it will be retried next run", key)


#: Attempts per request before a failure is taken at face value.
FETCH_ATTEMPTS = 3
#: Seconds before the first retry; doubled for every attempt after that.
FETCH_RETRY_BACKOFF = 1.0
#: Statuses where the server is reachable but momentarily unwilling to answer.
RETRYABLE_STATUSES = frozenset({408, 425, 429, 500, 502, 503, 504})


def _is_transient(exc: BaseException) -> bool:
    """Return True when repeating the request could plausibly succeed.

    A truncated body, dropped connection or read timeout says nothing about
    whether the artifact exists upstream, so one of them must not be able to
    fail a nightly run on its own. A 404 does say something definitive, and
    retrying those would only cost every run the thousands of probes for Forge
    versions that were never published.
    """
    if isinstance(exc, urllib.error.HTTPError):
        return exc.code in RETRYABLE_STATUSES
    if isinstance(exc, urllib.error.URLError):
        reason = exc.reason
        return not isinstance(reason, BaseException) or _is_transient(reason)
    # IncompleteRead and RemoteDisconnected arrive as HTTPException; timeouts,
    # connection resets, TLS errors and DNS blips arrive as OSError.
    return isinstance(exc, (http.client.HTTPException, OSError))


def _fetch_bytes(url: str, timeout: float = 60) -> bytes:
    headers = {
        "User-Agent": (
            "AgoraLoaderManifestBot/1.0 "
            "(repository configured by AGORA_REGISTRY_REPO)"
        ),
    }
    req = urllib.request.Request(url, headers=headers)

    for attempt in range(1, FETCH_ATTEMPTS + 1):
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.read()
        except (urllib.error.URLError, http.client.HTTPException, OSError) as exc:
            if attempt < FETCH_ATTEMPTS and _is_transient(exc):
                delay = FETCH_RETRY_BACKOFF * 2 ** (attempt - 1)
                logger.warning(
                    "Attempt %d/%d for %s failed (%s); retrying in %.1fs",
                    attempt,
                    FETCH_ATTEMPTS,
                    url,
                    exc,
                    delay,
                )
                time.sleep(delay)
                continue
            if isinstance(exc, urllib.error.URLError):
                raise
            # urllib wraps connection-phase failures in URLError, but a timeout
            # or a truncated body raised while reading the response escapes as
            # TimeoutError/IncompleteRead. Normalize every phase so
            # loader-specific skip logic can handle them.
            raise urllib.error.URLError(f"failed reading {url}: {exc}") from exc

    raise AssertionError("unreachable: the retry loop always returns or raises")


def _fetch_json(url: str) -> Any:
    return json.loads(_fetch_bytes(url).decode("utf-8"))


def _download_to_cache(
    url: str, cache_name: str, timeout: float = 60
) -> Path:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cache_path = CACHE_DIR / cache_name
    if cache_path.exists():
        logger.debug("Using cached %s", cache_name)
        return cache_path

    logger.info("Downloading %s", url)
    data = _fetch_bytes(url, timeout=timeout)
    cache_path.write_bytes(data)
    return cache_path


def _fetch_profile_json(url: str, cache_name: str, *, refresh: bool = False) -> bytes:
    """Fetch a profile JSON, caching it in ``.cache/profile-json/``."""
    cache_dir = CACHE_DIR / "profile-json"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_path = cache_dir / cache_name
    if cache_path.exists() and not refresh:
        logger.debug("Using cached profile JSON %s", cache_name)
        return cache_path.read_bytes()
    logger.info("Downloading profile JSON %s", url)
    try:
        data = _fetch_bytes(url)
    except urllib.error.URLError:
        if cache_path.exists():
            logger.warning("Profile refresh failed; using cached %s", cache_name)
            return cache_path.read_bytes()
        raise
    cache_path.write_bytes(data)
    return data


def _extract_version_json_sha256(jar_path: Path) -> str | None:
    """Extract version.json from an installer jar and return its stable SHA-256."""
    try:
        with zipfile.ZipFile(jar_path, "r") as zf:
            if "version.json" in zf.namelist():
                return _stable_json_sha256(zf.read("version.json"))
    except (zipfile.BadZipFile, OSError) as exc:
        logger.warning("Could not read %s: %s", jar_path.name, exc)
    return None


def _neoforge_version_to_mc(version: str) -> str | None:
    """Map NeoForge version to Minecraft version heuristically."""
    parts = version.split(".")
    if not parts or not parts[0].isdigit():
        return None
    major = parts[0]
    minor = parts[1] if len(parts) > 1 else "0"
    if minor == "0":
        return f"1.{major}"
    return f"1.{major}.{minor}"


# ---------------------------------------------------------------------------
# Fetchers per loader
# ---------------------------------------------------------------------------


def _fetch_fabric(
    mc_version: str,
    per_mc_limit: int | None = None,
    refresh_profiles: bool = False,
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    url = f"https://meta.fabricmc.net/v2/versions/loader/{mc_version}"
    try:
        versions = _fetch_json(url)
    except urllib.error.HTTPError as exc:
        logger.warning("Fabric has no loader versions for MC %s (%s)", mc_version, exc)
        return []
    except (urllib.error.URLError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise UpstreamMetadataError(
            f"Fabric loader list for {mc_version}: {exc}"
        ) from exc
    if not isinstance(versions, list):
        raise UpstreamMetadataError(
            f"Fabric loader list for {mc_version} was not a JSON array"
        )
    if any(
        not isinstance(info, dict)
        or not isinstance(info.get("loader"), dict)
        or not isinstance(info["loader"].get("version"), str)
        for info in versions
    ):
        raise UpstreamMetadataError(
            f"Fabric loader list for {mc_version} contained a malformed entry"
        )

    versions = sorted(
        versions,
        key=lambda info: _version_key(info.get("loader", {}).get("version", "")),
        reverse=True,
    )
    if per_mc_limit:
        versions = versions[:per_mc_limit]

    for info in versions:
        loader_info = info.get("loader") if isinstance(info, dict) else None
        loader_version = loader_info.get("version") if isinstance(loader_info, dict) else None
        if not loader_version:
            continue

        key = _failed_key("fabric", mc_version, loader_version)
        if _failed_should_skip(key):
            logger.debug("Skipping Fabric %s/%s (previously failed)", mc_version, loader_version)
            continue

        profile_url = (
            f"https://meta.fabricmc.net/v2/versions/loader/{mc_version}"
            f"/{loader_version}/profile/json"
        )
        cache_name = re.sub(r'[^a-zA-Z0-9._-]', '_', f"fabric-{mc_version}-{loader_version}.json")
        try:
            data = _fetch_profile_json(profile_url, cache_name, refresh=refresh_profiles)
        except urllib.error.URLError as exc:
            _failed_record_failure(key)
            logger.error(
                "Failed to fetch Fabric profile %s/%s: %s",
                mc_version,
                loader_version,
                exc,
            )
            continue

        _failed_record_success(key)
        file_name = f"fabric-loader-{loader_version}-{mc_version}.json"
        try:
            sha = _stable_json_sha256(data)
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise UpstreamMetadataError(
                f"Fabric profile {mc_version}/{loader_version}: {exc}"
            ) from exc

        entries.append({
            "mc_version": mc_version,
            "loader_version": loader_version,
            "source_url": profile_url,
            "sha256": sha,
            "file_name": file_name,
            "file_type": "profile_json",
            "provided_versions": {"fabricloader": loader_version},
            "release_channel": _release_channel(loader_version),
        })
        logger.info(
            "Added Fabric loader %s for MC %s (%s sha256=%s...)",
            loader_version,
            mc_version,
            _release_channel(loader_version),
            sha[:16],
        )

    return entries


def _fetch_quilt(
    mc_version: str,
    per_mc_limit: int | None = None,
    refresh_profiles: bool = False,
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    url = f"https://meta.quiltmc.org/v3/versions/loader/{mc_version}"
    try:
        versions = _fetch_json(url)
    except urllib.error.HTTPError as exc:
        logger.warning("Quilt has no loader versions for MC %s (%s)", mc_version, exc)
        return []
    except (urllib.error.URLError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise UpstreamMetadataError(
            f"Quilt loader list for {mc_version}: {exc}"
        ) from exc
    if not isinstance(versions, list):
        raise UpstreamMetadataError(
            f"Quilt loader list for {mc_version} was not a JSON array"
        )
    if any(
        not isinstance(info, dict)
        or not isinstance(info.get("loader"), dict)
        or not isinstance(info["loader"].get("version"), str)
        for info in versions
    ):
        raise UpstreamMetadataError(
            f"Quilt loader list for {mc_version} contained a malformed entry"
        )

    versions = sorted(
        versions,
        key=lambda info: _version_key(info.get("loader", {}).get("version", "")),
        reverse=True,
    )
    if per_mc_limit:
        versions = versions[:per_mc_limit]

    for info in versions:
        loader_info = info.get("loader") if isinstance(info, dict) else None
        loader_version = loader_info.get("version") if isinstance(loader_info, dict) else None
        if not loader_version:
            continue

        key = _failed_key("quilt", mc_version, loader_version)
        if _failed_should_skip(key):
            logger.debug("Skipping Quilt %s/%s (previously failed)", mc_version, loader_version)
            continue

        # Quilt's profile URL order matches Fabric: mc_version then loader_version.
        # If that 404s, fall back to the swapped order before giving up.
        profile_url_mc_first = (
            f"https://meta.quiltmc.org/v3/versions/loader/{mc_version}"
            f"/{loader_version}/profile/json"
        )
        profile_url_loader_first = (
            f"https://meta.quiltmc.org/v3/versions/loader/{loader_version}"
            f"/{mc_version}/profile/json"
        )
        cache_name = re.sub(r'[^a-zA-Z0-9._-]', '_', f"quilt-{mc_version}-{loader_version}.json")
        data: bytes | None = None
        profile_url = profile_url_mc_first
        try:
            data = _fetch_profile_json(
                profile_url_mc_first, cache_name, refresh=refresh_profiles
            )
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                logger.info(
                    "Quilt profile path %s 404, trying swapped order",
                    profile_url_mc_first,
                )
                profile_url = profile_url_loader_first
                cache_name = re.sub(r'[^a-zA-Z0-9._-]', '_', f"quilt-{mc_version}-{loader_version}-alt.json")
                try:
                    data = _fetch_profile_json(
                        profile_url_loader_first, cache_name, refresh=refresh_profiles
                    )
                except urllib.error.URLError as exc2:
                    _failed_record_failure(key)
                    logger.error(
                        "Failed to fetch Quilt profile %s/%s: %s",
                        mc_version,
                        loader_version,
                        exc2,
                    )
                    continue
            else:
                _failed_record_failure(key)
                logger.error(
                    "Failed to fetch Quilt profile %s/%s: %s",
                    mc_version,
                    loader_version,
                    exc,
                )
                continue
        except urllib.error.URLError as exc:
            _failed_record_failure(key)
            logger.error(
                "Failed to fetch Quilt profile %s/%s: %s",
                mc_version,
                loader_version,
                exc,
            )
            continue

        if data is None:
            continue

        _failed_record_success(key)
        file_name = f"quilt-loader-{loader_version}-{mc_version}.json"
        try:
            sha = _stable_json_sha256(data)
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise UpstreamMetadataError(
                f"Quilt profile {mc_version}/{loader_version}: {exc}"
            ) from exc

        entries.append({
            "mc_version": mc_version,
            "loader_version": loader_version,
            "source_url": profile_url,
            "sha256": sha,
            "file_name": file_name,
            "file_type": "profile_json",
            "provided_versions": {"quilt_loader": loader_version},
            "release_channel": _release_channel(loader_version),
        })
        logger.info(
            "Added Quilt loader %s for MC %s (%s sha256=%s...)",
            loader_version,
            mc_version,
            _release_channel(loader_version),
            sha[:16],
        )

    return entries


def _fetch_neoforge(
    mc_versions: list[str],
    per_mc_limit: int | None = None,
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    metadata_url = (
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml"
    )
    try:
        xml = _fetch_bytes(metadata_url)
        root = ET.fromstring(xml)
    except (urllib.error.URLError, ET.ParseError) as exc:
        raise UpstreamMetadataError(f"NeoForge Maven metadata: {exc}") from exc

    if root.find(".//versions") is None:
        raise UpstreamMetadataError("NeoForge Maven metadata was malformed")
    versions = [v.text for v in root.findall(".//versions/version") if v.text]
    candidates_by_mc: dict[str, list[str]] = {}
    for version in versions:
        mc_version = _neoforge_version_to_mc(version)
        if mc_version is None or mc_version not in mc_versions:
            continue
        candidates_by_mc.setdefault(mc_version, []).append(version)

    selected_versions: list[tuple[str, str]] = []
    for mc_version, group in candidates_by_mc.items():
        sorted_group = sorted(group, key=_version_key, reverse=True)
        chosen = sorted_group[:per_mc_limit] if per_mc_limit else sorted_group
        selected_versions.extend((v, mc_version) for v in chosen)

    for version, mc_version in selected_versions:
        key = _failed_key("neoforge", version, version)
        if _failed_should_skip(key):
            logger.debug("Skipping NeoForge %s (previously failed)", version)
            continue

        source_url = (
            f"https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}"
            f"/neoforge-{version}-installer.jar"
        )
        cache_name = f"neoforge-{version}-installer.jar"
        try:
            jar_path = _download_to_cache(source_url, cache_name)
        except urllib.error.URLError as exc:
            _failed_record_failure(key)
            logger.error("Failed to download NeoForge installer %s: %s", version, exc)
            continue

        _failed_record_success(key)

        jar_sha = _sha256_hex(jar_path.read_bytes())
        version_json_sha = _extract_version_json_sha256(jar_path)
        version_json = _extract_version_json(jar_path)

        install = _extract_install_profile(jar_path)
        file_name = f"neoforge-{version}-installer.jar"

        installer_spec = None
        if install is not None:
            spec_val = install.get("spec")
            if isinstance(spec_val, int):
                installer_spec = spec_val

        entry: dict[str, Any] = {
            "mc_version": mc_version,
            "loader_version": version,
            "source_url": source_url,
            "sha256": jar_sha,
            "file_name": file_name,
            "file_type": "installer_jar",
            "provided_versions": {
                "neoforge": version,
                **_extract_neoforge_language_capabilities(version_json),
            },
            "release_channel": _release_channel(version),
        }
        if version_json_sha:
            entry["version_json_sha256"] = version_json_sha
        if installer_spec is not None:
            entry["installer_spec"] = installer_spec

        logger.info(
            "Added NeoForge %s for MC %s (jar=%s..., version.json=%s..., spec=%s)",
            version,
            mc_version,
            jar_sha[:16],
            (version_json_sha or "N/A")[:16],
            installer_spec,
        )
        entries.append(entry)

    return entries


def _fetch_forge(
    mc_versions: list[str],
    per_mc_limit: int | None = None,
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    metadata_url = (
        "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml"
    )
    try:
        xml = _fetch_bytes(metadata_url)
        root = ET.fromstring(xml)
    except (urllib.error.URLError, ET.ParseError) as exc:
        raise UpstreamMetadataError(f"Forge Maven metadata: {exc}") from exc

    if root.find(".//versions") is None:
        raise UpstreamMetadataError("Forge Maven metadata was malformed")
    versions = [v.text for v in root.findall(".//versions/version") if v.text]
    candidates_by_mc: dict[str, list[tuple[str, str]]] = {}
    for version in versions:
        # Forge versions look like "1.21-51.0.0" (mc_version-build).
        if "-" not in version:
            continue
        mc_version, loader_version = version.split("-", 1)
        if mc_version not in mc_versions:
            continue
        candidates_by_mc.setdefault(mc_version, []).append((version, loader_version))

    selected_versions: list[tuple[str, str, str]] = []
    for mc_version, group in candidates_by_mc.items():
        sorted_group = sorted(group, key=lambda pair: _version_key(pair[1]), reverse=True)
        chosen = sorted_group[:per_mc_limit] if per_mc_limit else sorted_group
        selected_versions.extend((version, loader_version, mc_version) for version, loader_version in chosen)

    for version, loader_version, mc_version in selected_versions:
        key = _failed_key("forge", mc_version, version)
        if _failed_should_skip(key):
            logger.debug("Skipping Forge %s (previously failed)", version)
            continue

        source_url = (
            f"https://maven.minecraftforge.net/net/minecraftforge/forge/{version}"
            f"/forge-{version}-installer.jar"
        )
        cache_name = f"forge-{version}-installer.jar"
        try:
            jar_path = _download_to_cache(source_url, cache_name, timeout=1)
        except urllib.error.URLError as exc:
            _failed_record_failure(key)
            logger.error("Failed to download Forge installer %s: %s", version, exc)
            continue

        _failed_record_success(key)

        jar_sha = _sha256_hex(jar_path.read_bytes())
        version_json_sha = _extract_version_json_sha256(jar_path)

        install = _extract_install_profile(jar_path)
        file_name = f"forge-{version}-installer.jar"

        installer_spec = None
        if install is not None:
            spec_val = install.get("spec")
            if isinstance(spec_val, int):
                installer_spec = spec_val

        entry: dict[str, Any] = {
            "mc_version": mc_version,
            "loader_version": loader_version,
            "source_url": source_url,
            "sha256": jar_sha,
            "file_name": file_name,
            "file_type": "installer_jar",
            "provided_versions": {
                "forge": loader_version,
                **_forge_language_capabilities(loader_version),
            },
            "release_channel": _release_channel(loader_version),
        }
        if version_json_sha:
            entry["version_json_sha256"] = version_json_sha
        if installer_spec is not None:
            entry["installer_spec"] = installer_spec

        logger.info(
            "Added Forge %s for MC %s (jar=%s..., version.json=%s..., spec=%s)",
            version,
            mc_version,
            jar_sha[:16],
            (version_json_sha or "N/A")[:16],
            installer_spec,
        )
        entries.append(entry)

    return entries


# ---------------------------------------------------------------------------
# Manifest persistence
# ---------------------------------------------------------------------------


def _load_existing_manifest() -> dict[str, Any]:
    path = LOADER_MANIFESTS_DIR / "loader_manifests.json"
    if path.exists():
        with path.open("r", encoding="utf-8") as fh:
            return json.load(fh)
    return {
        "domain_allowlist": sorted(DOMAIN_ALLOWLIST),
        "loaders": {"fabric": [], "quilt": [], "neoforge": [], "forge": []},
    }


def _merge_entries(existing: list[dict[str, Any]], new_entries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: dict[tuple[str, str], dict[str, Any]] = {
        (e["mc_version"], e["loader_version"]): e for e in existing
    }
    mutations: list[dict[str, Any]] = []
    for entry in new_entries:
        key = (entry["mc_version"], entry["loader_version"])
        previous = seen.get(key)
        if previous is None:
            seen[key] = entry
            continue

        changed_fields = {
            field: {"before": previous.get(field), "after": entry.get(field)}
            for field in IMMUTABLE_ENTRY_FIELDS
            if previous.get(field) != entry.get(field)
        }
        # Enrichable fields: absent -> the new value is adopted (enrichment);
        # present -> any change is a mutation and must fail closed.
        for field in ENRICHABLE_ENTRY_FIELDS:
            if _enrichment_absent(previous, field):
                if not _enrichment_absent(entry, field):
                    previous[field] = entry[field]
            elif previous.get(field) != entry.get(field):
                changed_fields[field] = {
                    "before": previous.get(field),
                    "after": entry.get(field),
                }
        if changed_fields:
            mutations.append({
                "key": f"{key[0]}/{key[1]}",
                "fields": changed_fields,
            })

    if mutations:
        raise ExistingEntryMutationError(mutations)

    return list(seen.values())


def _sort_entries(entries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return sorted(
        entries,
        key=lambda e: (_version_key(e["mc_version"]), _version_key(e["loader_version"])),
    )


# Canonical key order for written entries, so enriched legacy entries and
# freshly fetched entries serialize identically.
_CANONICAL_ENTRY_KEYS = (
    "mc_version",
    "loader_version",
    "source_url",
    "sha256",
    "file_name",
    "file_type",
    "version_json_sha256",
    "installer_spec",
    "provided_versions",
    "release_channel",
    "recommendation_rank",
)


def _canonicalize_entry(loader: str, entry: dict[str, Any]) -> dict[str, Any]:
    """Enrich a legacy entry and re-emit its keys in canonical order."""
    entry = _enrich_entry(loader, entry)
    return {key: entry[key] for key in _CANONICAL_ENTRY_KEYS if key in entry}


def _write_loader_manifests(manifest: dict[str, Any]) -> None:
    LOADER_MANIFESTS_DIR.mkdir(parents=True, exist_ok=True)
    path = LOADER_MANIFESTS_DIR / "loader_manifests.json"
    for loader, entries in manifest["loaders"].items():
        manifest["loaders"][loader] = [
            _canonicalize_entry(loader, entry) for entry in entries
        ]
        manifest["loaders"][loader] = _sort_entries(manifest["loaders"][loader])
    manifest["domain_allowlist"] = sorted(set(manifest["domain_allowlist"]))

    with path.open("w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=False)
        fh.write("\n")
    logger.info("Wrote %s", path)


def _write_known_good_hashes(manifest: dict[str, Any]) -> None:
    loader_hashes: dict[str, dict[str, str | None]] = {}
    for loader, entries in manifest["loaders"].items():
        loader_hashes[loader] = {}
        for entry in entries:
            sha = entry.get("sha256")
            loader_hashes[loader][entry["file_name"]] = (
                f"sha256:{sha}" if sha else None
            )

    data = {
        "domain_allowlist": manifest["domain_allowlist"],
        "loader_hashes": loader_hashes,
        "_source": (
            "Generated from loader_manifests.json by scripts/fetch_loader_manifests.py. "
            "Do not edit manually."
        ),
    }
    path = LOADER_MANIFESTS_DIR / "known_good_hashes.json"
    with path.open("w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=False)
        fh.write("\n")
    logger.info("Wrote %s", path)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fetch official modloader manifests and pin their SHA-256 hashes."
    )
    parser.add_argument(
        "--mc-versions",
        nargs="+",
        default=DEFAULT_MC_VERSIONS,
        help="Minecraft versions to query (default: 1.21)",
    )
    parser.add_argument(
        "--latest-per-mc",
        type=int,
        default=5,
        help="Query at most N new loader versions per Minecraft version (default: 5, 0 = unlimited); existing entries are retained",
    )
    parser.add_argument(
        "--all-versions",
        action="store_true",
        help="Disable the per-Minecraft-version limit and keep every available loader version",
    )
    args = parser.parse_args()

    mc_versions = sorted(set(args.mc_versions), key=_version_key)
    per_mc_limit: int | None = None if args.all_versions else args.latest_per_mc
    logger.info("Querying loaders for Minecraft versions: %s", mc_versions)
    logger.info("Per-MC version limit: %s", "unlimited" if per_mc_limit is None else per_mc_limit)

    manifest = _load_existing_manifest()
    # Ensure the manifest always has the canonical domain allowlist.
    manifest["domain_allowlist"] = sorted(
        set(manifest.get("domain_allowlist", []) + DOMAIN_ALLOWLIST)
    )
    loaders = manifest.setdefault("loaders", {})
    for loader in ("fabric", "quilt", "neoforge", "forge"):
        loaders.setdefault(loader, [])

    for mc_version in mc_versions:
        logger.info("Fetching Fabric versions for %s", mc_version)
        loaders["fabric"] = _merge_entries(
            loaders["fabric"],
            _fetch_fabric(mc_version, per_mc_limit),
        )

        logger.info("Fetching Quilt versions for %s", mc_version)
        loaders["quilt"] = _merge_entries(
            loaders["quilt"],
            _fetch_quilt(mc_version, per_mc_limit),
        )

    logger.info("Fetching NeoForge versions for %s", mc_versions)
    loaders["neoforge"] = _merge_entries(
        loaders["neoforge"],
        _fetch_neoforge(mc_versions, per_mc_limit),
    )

    logger.info("Fetching Forge versions for %s", mc_versions)
    loaders["forge"] = _merge_entries(
        loaders["forge"],
        _fetch_forge(mc_versions, per_mc_limit),
    )

    _write_loader_manifests(manifest)
    _write_known_good_hashes(manifest)

    total = sum(len(entries) for entries in loaders.values())
    logger.info("Done. %d loader entries in loader_manifests.json", total)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
