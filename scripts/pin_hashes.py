#!/usr/bin/env python3
"""Pin SHA-256 hashes for curated registry entries.

Two modes:

1. Plain manifest pinning (default):
   python scripts/pin_hashes.py registry/mods/foo.json

   Reads a curated manifest, validates it satisfies the compiler's
   ``direct_hash`` contract (so it cannot emit a manifest the compiler would
   reject), downloads ``source_identifier``, computes SHA-256, and rewrites
   the manifest's ``sha256`` in place. Idempotent: a manifest whose hash is
   already current is left untouched.

2. Technic pack scaffolding (promotes Tier S/Z -> Tier C):
   python scripts/pin_hashes.py --technic <slug> [--output out.json]

   Resolves ``/modpack/<slug>?build=stable4`` -> the pack's Solder endpoint ->
   the recommended build, downloads every mod entry, computes SHA-256 for each
   (Technic's MD5 never enters Agora's trust model), and emits a ready-to-
   review pin report with each mod's name, version, URL, MD5, and SHA-256.

Technic notes (measured 2026-08): the Solder ``forge`` field is None on every
pack, so the loader version must be recovered from the rehosted ``forge`` zip
mod entry and mapped onto Agora's pinned loader manifests at install time.
``--technic`` reports that entry (e.g. ``14.23.5.2860``) so a curator can
resolve the official loader rather than Technic's rehosted zip.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import re
import sys
from pathlib import Path
from typing import Any

try:
    import requests
except ImportError:  # pragma: no cover
    print("pin_hashes.py requires `requests` (pip install requests)", file=sys.stderr)
    sys.exit(2)

REPO_ROOT = Path(__file__).resolve().parent.parent
TECHNIC_API = "https://api.technicpack.net"

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger("pin_hashes")


def _validate_sha256(raw: Any) -> str:
    """Mirror compiler.compile.validate_sha256 (rejected manifests fail the build)."""
    if raw is None or raw == "" or not isinstance(raw, str):
        raise SystemExit(
            "sha256 is required, non-empty, and must be a string"
        )
    if not re.fullmatch(r"[0-9a-fA-F]{64}", raw):
        raise SystemExit("sha256 must be exactly 64 hex characters")
    return raw.lower()


def direct_hash_url(item: dict[str, Any]) -> str:
    """The pinned URL of *item*'s ``direct_hash`` source.

    An item states its sources either as a ``download_sources`` list or as the
    legacy ``download_strategy``/``source_identifier`` pair. Exactly one
    ``direct_hash`` source must be present: this script rewrites a single
    ``sha256`` field, so an item with two pinned URLs has no single hash to pin.
    """
    item_id = item.get("id", "<unknown>")
    sources = item.get("download_sources")
    if isinstance(sources, list) and sources:
        pinned = [
            str(source.get("identifier", "")).strip()
            for source in sources
            if isinstance(source, dict) and source.get("strategy") == "direct_hash"
        ]
        if len(pinned) != 1:
            raise SystemExit(
                f"{item_id}: pin_hashes.py needs exactly one direct_hash source, found {len(pinned)}"
            )
        return pinned[0]
    if item.get("download_strategy") != "direct_hash":
        raise SystemExit(
            f"{item_id}: pin_hashes.py pins direct_hash entries only; "
            f"got download_strategy={item.get('download_strategy')!r}"
        )
    return str(item.get("source_identifier", "")).strip()


def _validate_direct_hash_contract(item: dict[str, Any]) -> str:
    """Fail *before* writing a manifest the compiler would reject.

    Mirrors compiler.compile.validate_pinned_source. Returns the pinned URL.
    """
    item_id = item.get("id", "<unknown>")
    source = direct_hash_url(item)
    if not source.startswith("https://"):
        raise SystemExit(f"{item_id}: direct_hash source_identifier must be an https:// URL")
    filename = source.split("#")[0].split("?")[0].rsplit("/", 1)[-1]
    if not filename or filename.startswith(".") or ".." in filename or "." not in filename:
        raise SystemExit(
            f"{item_id}: direct_hash source_identifier must end in a filename "
            f"(e.g. https://example.com/files/my-mod-1.2.3.jar)"
        )
    declared = item.get("compatible_versions")
    if not declared:
        raise SystemExit(
            f"{item_id}: direct_hash requires an explicit compatible_versions list"
        )
    for entry in declared:
        if not isinstance(entry, dict):
            raise SystemExit(f"{item_id}: each compatible_versions entry must be an object")
        missing = [
            key
            for key in ("mc_version", "loader", "mod_version")
            if not str(entry.get(key, "")).strip()
        ]
        if missing:
            raise SystemExit(
                f"{item_id}: compatible_versions entry missing {', '.join(missing)}"
            )
        if str(entry["mod_version"]).strip() == "latest":
            raise SystemExit(
                f"{item_id}: compatible_versions needs a real mod_version, not 'latest'"
            )
    return source


def _sha256_of(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def pin_manifest(path: Path) -> str:
    """Download a direct_hash entry's file and pin its SHA-256 in place."""
    manifest = json.loads(path.read_text(encoding="utf-8"))
    url = _validate_direct_hash_contract(manifest)
    item_id = manifest.get("id", "<unknown>")

    logger.info("downloading %s from %s", item_id, url)
    response = requests.get(url, timeout=120)
    response.raise_for_status()
    sha256 = _sha256_of(response.content)

    old = manifest.get("sha256")
    if old is not None and old.strip().lower() == sha256:
        logger.info("%s: sha256 already current (%s)", item_id, sha256)
        return sha256

    manifest["sha256"] = sha256
    path.write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    if old is not None:
        logger.warning(
            "%s: sha256 CHANGED\n  old: %s\n  new: %s",
            item_id,
            old.strip(),
            sha256,
        )
    else:
        logger.info("%s: wrote sha256 %s", item_id, sha256)
    return sha256


# ---------------------------------------------------------------------------
# Technic scaffolding
# ---------------------------------------------------------------------------


def _technic_json(url: str) -> dict[str, Any]:
    response = requests.get(url, timeout=30)
    if response.status_code == 401:
        raise SystemExit(
            f"Technic API returned 401 for {url}; "
            "does the request include the mandatory '?build=stable4' parameter?"
        )
    response.raise_for_status()
    return response.json()


def technic_pin_report(slug: str) -> dict[str, Any]:
    """Resolve a Technic pack's recommended build and pin every mod's SHA-256.

    Returns a ready-to-review report; the caller decides how to shape it into
    a registry manifest.
    """
    pack = _technic_json(f"{TECHNIC_API}/modpack/{slug}?build=stable4")
    solder = pack.get("solder")
    if not solder:
        raise SystemExit(f"{slug}: Technic reports no Solder endpoint for this pack")
    solder = solder.rstrip("/")

    api = _technic_json(f"{solder}/api/modpack/{slug}")
    recommended = api.get("recommended") or pack.get("recommended")
    if not recommended:
        raise SystemExit(f"{slug}: no recommended build reported by Solder")

    build = _technic_json(f"{solder}/api/modpack/{slug}/{recommended}")
    mines = build.get("minecraft")
    loader = None
    entry = None
    mods: list[dict[str, Any]] = []
    for mod in build.get("mods", []):
        name = str(mod.get("name", "")).strip()
        url = str(mod.get("url", "")).strip()
        version = str(mod.get("version", "")).strip()
        md5 = str(mod.get("md5", "")).strip()

        # The loader ships as a rehosted zip mod entry (forge: None everywhere);
        # recover the version so install can map it onto Agora's pinned loader
        # manifests instead of Technic's rehosted file.
        if name.lower() == "forge" and loader is None:
            loader = version

        if not url:
            logger.warning("%s: mod entry %r has no url; skipping", slug, name)
            continue
        logger.info("downloading %s (%s) for hashing", name, version or "?")
        response = requests.get(url, timeout=120)
        response.raise_for_status()
        entry = {
            "name": name,
            "version": version,
            "md5": md5,
            "sha256": _sha256_of(response.content),
            "size": len(response.content),
            "url": url,
        }
        mods.append(entry)

    return {
        "technic_slug": slug,
        "display_name": pack.get("display_name") or pack.get("name") or slug,
        "solder": solder,
        "recommended_build": recommended,
        "minecraft": mines,
        "loader": {"forge": loader} if loader else {},
        "mods": mods,
        "page_url": pack.get("link") or f"https://www.technicpack.net/modpack/{slug}",
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Pin SHA-256 hashes for curated registry entries."
    )
    parser.add_argument(
        "manifests",
        nargs="*",
        help="Paths to registry manifests (direct_hash entries) to pin in place.",
    )
    parser.add_argument(
        "--technic",
        metavar="SLUG",
        help="Resolve a Technic pack's recommended build and pin every mod hash.",
    )
    parser.add_argument(
        "--output",
        metavar="PATH",
        help="For --technic, write the pin report JSON to PATH instead of stdout.",
    )
    args = parser.parse_args()

    if args.technic:
        report = technic_pin_report(args.technic)
        rendered = json.dumps(report, indent=2, ensure_ascii=False) + "\n"
        if args.output:
            Path(args.output).write_text(rendered, encoding="utf-8")
            logger.info("wrote pin report to %s", args.output)
        else:
            sys.stdout.write(rendered)
        return

    if not args.manifests:
        parser.error("provide at least one manifest path, or --technic <slug>")
    for raw in args.manifests:
        pin_manifest(Path(raw))


if __name__ == "__main__":
    main()