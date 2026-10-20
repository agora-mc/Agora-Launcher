#!/usr/bin/env python3
"""Normalize desktop release assets and publish a full SHA256SUMS.

Tauri names its bundles after the product — `Agora.Launcher_<version>_<arch>.<ext>`
— which does not say desktop or CLI, mixes `x64`/`x86_64`/`amd64`, and sorts the
main Windows installer into the middle of the release's asset list (GitHub lists
assets alphabetically). This script runs once the draft release holds every
artifact and leaves it in one scheme:

    agora-<kind>-v<version>-<platform>-<arch>[-<variant>].<ext>

where `<kind>` is `desktop` or `cli` and `<arch>` is `x64` or `arm64`.

In order, it:

  1. fails before mutating anything if the release is missing an expected
     artifact, carries an unexpected one, or holds an `Agora.Launcher*` asset
     whose embedded version disagrees with the tag;
  2. deletes the updater `*.sig` sidecar assets (the signature text also lives
     inline in `latest.json`, which is what the updater actually reads);
  3. renames the Tauri bundles in place through the asset-rename API;
  4. rewrites every `latest.json` platform URL to its renamed asset and
     re-uploads it;
  5. writes `SHA256SUMS` covering every downloadable artifact except itself
     and `latest.json`, and uploads it.

Re-running is safe: a second pass finds the new names in place. A release that
somehow holds both an old and a new name for the same bundle deletes the new
name and renames the (freshly re-uploaded) old one over it.

Usage:
    python scripts/finalize_release_assets.py --repo owner/repo --tag v1.2.3

Requires an authenticated `gh` (GH_TOKEN is what Actions sets).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterable, Mapping, Sequence

TAG_PATTERN = re.compile(r"v(?P<version>\d+\.\d+\.\d+)")

# Assets that describe the release rather than being downloadable artifacts.
METADATA_ASSET_NAMES = ("latest.json", "SHA256SUMS")

# Tauri's bundle suffixes (and their one locale-carrying variant) mapped to the
# unified `<platform>-<arch>` vocabulary. Ordered so no suffix pattern can
# shadow another; each pattern captures the shared `Agora.Launcher...` stem
# unless the third field is False, which marks names Tauri does not version
# (the updater archives). Those two stay strict on purpose: a drifted name
# there means Tauri changed its naming and this table needs a matching update.
_DESKTOP_SUFFIXES: tuple[tuple[re.Pattern[str], str, bool], ...] = (
    (re.compile(r"(?P<stem>.+)_x64-setup\.exe$"), "-windows-x64-setup.exe", True),
    (re.compile(r"(?P<stem>.+)_x64_[A-Za-z]{2,3}(?:-[A-Za-z]{2,4})?\.msi$"), "-windows-x64.msi", True),
    (re.compile(r"(?P<stem>.+)_aarch64\.dmg$"), "-macos-arm64.dmg", True),
    (re.compile(r"(?P<stem>.+)_x64\.dmg$"), "-macos-x64.dmg", True),
    (re.compile(r"(?P<stem>.+)_amd64\.AppImage$"), "-linux-x64.AppImage", True),
    (re.compile(r"(?P<stem>.+)_amd64\.deb$"), "-linux-x64.deb", True),
    (re.compile(r"(?P<stem>.+)-1\.x86_64\.rpm$"), "-linux-x64.rpm", True),
    (re.compile(r"Agora\.Launcher_aarch64\.app\.tar\.gz$"), "-macos-arm64.app.tar.gz", False),
    (re.compile(r"Agora\.Launcher_x64\.app\.tar\.gz$"), "-macos-x64.app.tar.gz", False),
)

# Both `Agora.Launcher_0.9.3` and the RPM stem `Agora.Launcher-0.9.3` start here.
_TAURI_STEM = re.compile(r"Agora\.Launcher[_-](?P<version>.+)$")

_SHA256_HEX = re.compile(r"[0-9a-f]{64}")


class ReleaseAssetError(RuntimeError):
    """The release's assets do not fit the finalize plan."""


def version_from_tag(tag: str) -> str:
    """The bare version a `vX.Y.Z` release tag names."""
    match = TAG_PATTERN.fullmatch(tag)
    if not match:
        raise ReleaseAssetError(f"Tag {tag!r} is not vX.Y.Z.")
    return match.group("version")


def desktop_asset_rename(name: str, version: str) -> str:
    """The unified name for one Tauri bundle, or raise if it does not fit.

    Refusing unknown names is deliberate: a missed rename would silently leave a
    second installer in the release, which is the problem this script exists to
    fix.
    """
    for suffix_pattern, new_suffix, versioned in _DESKTOP_SUFFIXES:
        match = suffix_pattern.fullmatch(name)
        if not match:
            continue
        if versioned:
            stem_match = _TAURI_STEM.fullmatch(match.group("stem"))
            if not stem_match:
                raise ReleaseAssetError(f"Unrecognized Tauri asset name: {name!r}")
            embedded = stem_match.group("version")
            if embedded != version:
                raise ReleaseAssetError(
                    f"Asset {name!r} carries version {embedded}, expected {version}. "
                    "The release mixes artifacts from two versions; delete the stale assets and re-run."
                )
        return f"agora-desktop-v{version}{new_suffix}"
    raise ReleaseAssetError(
        f"Unrecognized Agora.Launcher asset name: {name!r}. "
        "Teach scripts/finalize_release_assets.py the new name before re-running."
    )


def plan_desktop_renames(names: Iterable[str], version: str) -> dict[str, str]:
    """Old -> new name for every Tauri bundle in `names`.

    `*.sig` sidecars are left out: they are deleted, not renamed.
    """
    renames: dict[str, str] = {}
    targets: dict[str, str] = {}
    for name in names:
        if not name.startswith("Agora.Launcher") or name.endswith(".sig"):
            continue
        new_name = desktop_asset_rename(name, version)
        if new_name in targets:
            raise ReleaseAssetError(f"{name!r} and {targets[new_name]!r} would both be renamed to {new_name!r}.")
        targets[new_name] = name
        renames[name] = new_name
    return renames


def expected_asset_names(tag: str) -> list[str]:
    """Every downloadable artifact a desktop/CLI release must publish.

    `latest.json` and `SHA256SUMS` are metadata, so they are not listed here.
    Both the finalize step and the workflow's post-finalize verification compare
    the release against exactly this set.
    """
    return [
        f"agora-desktop-{tag}-windows-x64-setup.exe",
        f"agora-desktop-{tag}-windows-x64.msi",
        f"agora-desktop-{tag}-windows-x64-portable.zip",
        f"agora-desktop-{tag}-windows-x64-portable.zip.sha256",
        f"agora-desktop-{tag}-macos-arm64.dmg",
        f"agora-desktop-{tag}-macos-x64.dmg",
        f"agora-desktop-{tag}-macos-arm64.app.tar.gz",
        f"agora-desktop-{tag}-macos-x64.app.tar.gz",
        f"agora-desktop-{tag}-linux-x64.AppImage",
        f"agora-desktop-{tag}-linux-x64.deb",
        f"agora-desktop-{tag}-linux-x64.rpm",
        f"agora-cli-{tag}-windows-x64.zip",
        f"agora-cli-{tag}-linux-x64.tar.gz",
        f"agora-cli-{tag}-macos-arm64.tar.gz",
        f"agora-cli-{tag}-macos-x64.tar.gz",
    ]


def unexpected_asset_names(names: Iterable[str], tag: str) -> list[str]:
    """Assets a finished release must not carry: junk, duplicates, stale versions."""
    allowed = set(expected_asset_names(tag)) | set(METADATA_ASSET_NAMES)
    return sorted(set(names) - allowed)


def digest_hex(value: object) -> str | None:
    """Parse a GitHub asset digest (`sha256:<hex>`) into lowercase hex, or None."""
    if not isinstance(value, str):
        return None
    algorithm, separator, hexdigest = value.partition(":")
    if separator != ":" or algorithm != "sha256" or not _SHA256_HEX.fullmatch(hexdigest.lower()):
        return None
    return hexdigest.lower()


def render_sha256sums(entries: Iterable[tuple[str, str]]) -> str:
    """A `sha256sum`-compatible file: `<hex>  <name>` per artifact, sorted by name."""
    lines = []
    for name, digest in sorted(entries):
        if not _SHA256_HEX.fullmatch(digest):
            raise ReleaseAssetError(f"Not a SHA-256 digest for {name}: {digest!r}")
        lines.append(f"{digest}  {name}")
    return "\n".join(lines) + "\n"


def rewrite_latest_urls(latest: dict, renames: Mapping[str, str]) -> dict:
    """Point every `latest.json` updater URL at its renamed asset.

    Returns a new object; the input is not mutated. URLs that already name the
    unified scheme (a re-run) pass through unchanged, while an `Agora.Launcher*`
    URL missing from `renames` is an error rather than a dead updater link.
    """
    platforms = latest.get("platforms")
    if not isinstance(platforms, dict) or not platforms:
        raise ReleaseAssetError("latest.json has no platform entries.")

    rewritten = dict(latest)
    rewritten_platforms = {}
    for key, platform in platforms.items():
        if not isinstance(platform, dict):
            raise ReleaseAssetError(f"latest.json platform {key!r} is not an object.")
        platform = dict(platform)
        url = platform.get("url")
        if isinstance(url, str):
            basename = url.rsplit("/", 1)[-1]
            if basename in renames:
                platform["url"] = url[: len(url) - len(basename)] + renames[basename]
            elif basename.startswith("Agora.Launcher"):
                raise ReleaseAssetError(
                    f"latest.json points {key!r} at {basename!r}, which no rename covers."
                )
        rewritten_platforms[key] = platform
    rewritten["platforms"] = rewritten_platforms
    return rewritten


def run_gh(*args: str) -> str:
    """Run a `gh` command, returning stdout; raises with stderr on failure."""
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit code {result.returncode}"
        raise ReleaseAssetError(f"gh {' '.join(args)} failed: {detail}")
    return result.stdout


def fetch_assets(repo: str, tag: str) -> list[dict]:
    payload = json.loads(run_gh("release", "view", tag, "--repo", repo, "--json", "assets"))
    assets = payload.get("assets")
    if not isinstance(assets, list):
        raise ReleaseAssetError(f"gh release view returned no asset list for {tag}.")
    for asset in assets:
        if not isinstance(asset, dict) or "id" not in asset or not isinstance(asset.get("name"), str):
            raise ReleaseAssetError(f"Release {tag} has an asset without an id/name: {asset!r}")
    return assets


def delete_asset(repo: str, asset: dict) -> None:
    run_gh("api", "--method", "DELETE", f"/repos/{repo}/releases/assets/{asset['id']}")


def rename_asset(repo: str, asset: dict, new_name: str) -> None:
    # `-f` encodes the request body as JSON, which is all this endpoint accepts.
    run_gh(
        "api",
        "--method",
        "PATCH",
        f"/repos/{repo}/releases/assets/{asset['id']}",
        "-f",
        f"name={new_name}",
    )


def download_asset(repo: str, tag: str, name: str, directory: Path) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    run_gh("release", "download", tag, "--repo", repo, "--pattern", name, "--dir", str(directory), "--clobber")
    path = directory / name
    if not path.is_file():
        raise ReleaseAssetError(f"Release {tag} has no asset named {name}.")
    return path


def upload_asset(repo: str, tag: str, path: Path) -> None:
    run_gh("release", "upload", tag, str(path), "--repo", repo, "--clobber")


def asset_digest(repo: str, tag: str, asset: dict, directory: Path) -> str:
    """An artifact's SHA-256, from the release API when it has one, else the bytes."""
    digest = digest_hex(asset.get("digest"))
    if digest:
        return digest
    detail = json.loads(run_gh("api", f"/repos/{repo}/releases/assets/{asset['id']}"))
    digest = digest_hex(detail.get("digest"))
    if digest:
        return digest
    hasher = hashlib.sha256()
    with download_asset(repo, tag, asset["name"], directory).open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def plan_and_apply(repo: str, tag: str, version: str) -> None:
    assets = fetch_assets(repo, tag)
    names = [asset["name"] for asset in assets]
    renames = plan_desktop_renames(names, version)
    sigs = sorted(name for name in names if name.startswith("Agora.Launcher") and name.endswith(".sig"))

    final = (set(names) - set(renames) - set(sigs)) | set(renames.values())
    missing = sorted((set(expected_asset_names(tag)) | {"latest.json"}) - final)
    if missing:
        raise ReleaseAssetError("Release is missing expected assets: " + ", ".join(missing))
    unexpected = unexpected_asset_names(final, tag)
    if unexpected:
        raise ReleaseAssetError("Release carries unexpected assets: " + ", ".join(unexpected))

    by_name = {asset["name"]: asset for asset in assets}
    for name in sigs:
        print(f"Deleting signature sidecar {name}")
        delete_asset(repo, by_name[name])
    for old, new in sorted(renames.items()):
        # A leftover target is stale by definition: the old name is what a
        # re-run just uploaded. Delete it so the rename has somewhere to land.
        if new in by_name:
            print(f"Deleting stale asset {new}")
            delete_asset(repo, by_name[new])
    for old, new in sorted(renames.items()):
        print(f"Renaming {old} -> {new}")
        rename_asset(repo, by_name[old], new)

    with tempfile.TemporaryDirectory() as workdir_name:
        workdir = Path(workdir_name)

        latest_path = download_asset(repo, tag, "latest.json", workdir)
        latest = json.loads(latest_path.read_text(encoding="utf-8"))
        latest_path.write_text(json.dumps(rewrite_latest_urls(latest, renames), indent=2) + "\n", encoding="utf-8")
        upload_asset(repo, tag, latest_path)

        # Re-fetch so digests and ids describe the renamed assets.
        current = {asset["name"]: asset for asset in fetch_assets(repo, tag)}
        sums = [
            (name, asset_digest(repo, tag, current[name], workdir))
            for name in expected_asset_names(tag)
        ]
        sums_path = workdir / "SHA256SUMS"
        sums_path.write_text(render_sha256sums(sums), encoding="ascii")
        upload_asset(repo, tag, sums_path)
    print(f"Finalized {len(sums)} artifacts; SHA256SUMS uploaded.")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--repo", required=True, help="GitHub repository, owner/name")
    parser.add_argument("--tag", required=True, help="Release tag, vX.Y.Z")
    args = parser.parse_args(argv)

    try:
        plan_and_apply(args.repo, args.tag, version_from_tag(args.tag))
    except ReleaseAssetError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
