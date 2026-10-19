#!/usr/bin/env python3
"""Validate an unattended loader-catalog refresh before publication."""

from __future__ import annotations

import argparse
import difflib
import json
import sys
from pathlib import Path
from typing import Any

from fetch_loader_manifests import (
    FILE_DERIVED_FIELDS,
    IMMUTABLE_ENTRY_FIELDS,
    QUARANTINE_PATH,
)


def _entry_key(loader: str, entry: dict[str, Any]) -> str:
    return f"{loader}/{entry.get('mc_version')}/{entry.get('loader_version')}"


def _catalog_entries(
    catalog: dict[str, Any], label: str, errors: list[str]
) -> dict[str, dict[str, Any]]:
    loaders = catalog.get("loaders")
    if not isinstance(loaders, dict):
        errors.append(f"{label}: loaders must be an object")
        return {}

    result: dict[str, dict[str, Any]] = {}
    for loader, entries in loaders.items():
        if not isinstance(entries, list):
            errors.append(f"{label}: loader {loader!r} entries must be an array")
            continue
        for entry in entries:
            if not isinstance(entry, dict):
                errors.append(f"{label}: loader {loader!r} contains a non-object entry")
                continue
            key = _entry_key(loader, entry)
            if key in result:
                errors.append(f"{label}: duplicate entry {key}")
            result[key] = entry
    return result


def _changed_json_lines(before_path: Path, after_path: Path) -> int:
    before = before_path.read_text(encoding="utf-8").splitlines()
    after = after_path.read_text(encoding="utf-8").splitlines()
    return sum(
        1
        for line in difflib.unified_diff(before, after)
        if (line.startswith("+") or line.startswith("-"))
        and not line.startswith(("+++", "---"))
    )


def _domain_set(
    catalog: dict[str, Any], label: str, errors: list[str]
) -> set[str]:
    domains = catalog.get("domain_allowlist")
    if not isinstance(domains, list) or not all(
        isinstance(domain, str) for domain in domains
    ):
        errors.append(f"{label}: domain_allowlist must be an array of strings")
        return set()
    return set(domains)


def _is_recorded_rotation(old_entry: dict[str, Any], new_entry: dict[str, Any]) -> bool:
    """Whether the new entry records re-pinning away from the old file."""
    history = new_entry.get("sha256_history")
    return (
        isinstance(history, list)
        and bool(history)
        and isinstance(history[-1], dict)
        and history[-1].get("sha256") == old_entry.get("sha256")
    )


def validate_catalog_delta(
    before: dict[str, Any],
    after: dict[str, Any],
    *,
    append_only: bool = False,
    reject_existing_mutations: bool = False,
    max_new_entries: int | None = None,
    max_changed_lines: int | None = None,
    changed_json_lines: int | None = None,
    refresh_report: dict[str, Any] | None = None,
    quarantined: set[str] | None = None,
) -> dict[str, Any]:
    errors: list[str] = []
    before_entries = _catalog_entries(before, "before", errors)
    after_entries = _catalog_entries(after, "after", errors)

    # Entries held back in quarantine leave the catalog deliberately.
    deleted = sorted(set(before_entries) - set(after_entries) - (quarantined or set()))
    new = sorted(set(after_entries) - set(before_entries))
    mutations: list[dict[str, Any]] = []
    rotations: list[str] = []
    for key in sorted(set(before_entries) & set(after_entries)):
        old_entry = before_entries[key]
        new_entry = after_entries[key]
        # A re-pinned file may change what was derived from the file, and only
        # that; the rest of the entry still has to match.
        rotated = old_entry.get("sha256") != new_entry.get("sha256") and _is_recorded_rotation(
            old_entry, new_entry
        )
        if rotated:
            rotations.append(key)
        changed_fields = {
            field: {"before": old_entry.get(field), "after": new_entry.get(field)}
            for field in IMMUTABLE_ENTRY_FIELDS
            if old_entry.get(field) != new_entry.get(field)
            and not (rotated and field in FILE_DERIVED_FIELDS)
        }
        if changed_fields:
            mutations.append({"key": key, "fields": changed_fields})

    before_domains = _domain_set(before, "before", errors)
    after_domains = _domain_set(after, "after", errors)
    added_domains = sorted(after_domains - before_domains)
    removed_domains = sorted(before_domains - after_domains)

    report: dict[str, Any] = {
        "metadata_sources_failed": list(
            (refresh_report or {}).get("metadata_sources_failed", [])
        ),
        "candidate_download_failures": list(
            (refresh_report or {}).get("candidate_download_failures", [])
        ),
        "existing_entries_mutated": mutations,
        "repinned_entries": rotations,
        "quarantined_entries": sorted(quarantined or set()),
        "unexpected_deletions": deleted,
        "new_entries": len(new),
        "new_entry_keys": new,
        "added_domains": added_domains,
        "removed_domains": removed_domains,
        "changed_json_lines": changed_json_lines,
        "errors": errors,
    }

    if report["metadata_sources_failed"]:
        errors.append("one or more metadata sources failed")
    if (refresh_report or {}).get("refresh_error"):
        errors.append("the refresh script reported an error")
    if append_only and deleted:
        errors.append("append-only validation found deleted entries")
    if reject_existing_mutations and mutations:
        errors.append("existing loader entries were mutated")
    if added_domains:
        errors.append("the domain allowlist gained entries")
    if removed_domains:
        errors.append("the domain allowlist lost entries")
    if max_new_entries is not None and len(new) > max_new_entries:
        errors.append(
            f"new entry count {len(new)} exceeds limit {max_new_entries}"
        )
    if (
        max_changed_lines is not None
        and changed_json_lines is not None
        and changed_json_lines > max_changed_lines
    ):
        errors.append(
            f"changed JSON line count {changed_json_lines} exceeds limit "
            f"{max_changed_lines}"
        )

    return report


def _load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--append-only", action="store_true")
    parser.add_argument("--reject-existing-mutations", action="store_true")
    parser.add_argument("--max-new-entries", type=int)
    parser.add_argument("--max-changed-lines", type=int)
    parser.add_argument("--refresh-report", type=Path)
    parser.add_argument("--quarantine", type=Path, default=QUARANTINE_PATH)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()

    try:
        before = _load_json(args.before)
        after = _load_json(args.after)
        upstream_report = (
            _load_json(args.refresh_report) if args.refresh_report else None
        )
        quarantined = (
            {
                record["key"]
                for record in _load_json(args.quarantine).get("entries", [])
                if not record.get("approved")
            }
            if args.quarantine.exists()
            else set()
        )
        changed_lines = _changed_json_lines(args.before, args.after)
        report = validate_catalog_delta(
            before,
            after,
            append_only=args.append_only,
            reject_existing_mutations=args.reject_existing_mutations,
            max_new_entries=args.max_new_entries,
            max_changed_lines=args.max_changed_lines,
            changed_json_lines=changed_lines,
            refresh_report=upstream_report,
            quarantined=quarantined,
        )
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"Loader catalog validation failed: {exc}", file=sys.stderr)
        return 1

    report["errors"] = list(report["errors"])
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    if report["errors"]:
        print("Loader catalog delta rejected:")
        for error in report["errors"]:
            print(f"- {error}")
        return 1

    print(
        f"Loader catalog delta accepted: {report['new_entries']} new entries, "
        f"{report['changed_json_lines']} changed JSON lines"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
