#!/usr/bin/env python3
"""Validate governance-state.json against the strict schema.

Usage:
    python scripts/validate_governance_state.py path/to/governance-state.json

Exits 0 on valid, 1 on invalid (errors printed to stderr).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def validate(path: Path) -> list[str]:
    """Validate a governance-state.json file. Returns list of error messages."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return [f"Invalid JSON: {exc}"]
    except OSError as exc:
        return [f"File error: {exc}"]

    errors: list[str] = []

    if not isinstance(data, dict):
        return [f"Top-level value must be a JSON object, got {type(data).__name__}"]

    if data.get("schema_version") != 1:
        errors.append(
            f"schema_version must be 1, got {data.get('schema_version')!r}"
        )

    expected_repo = "agora-mc/Agora-Launcher"
    actual_repo = data.get("governance_repository")
    if actual_repo != expected_repo:
        errors.append(
            "governance_repository must be "
            f"{expected_repo!r}, got {actual_repo!r}"
        )

    expected_policy = "production"
    actual_policy = data.get("policy")
    if actual_policy != expected_policy:
        errors.append(
            f"policy must be {expected_policy!r}, got {actual_policy!r}"
        )

    events = data.get("events")
    if not isinstance(events, list):
        errors.append(
            f"events must be a list, got {type(events).__name__}"
        )
        return errors

    seen_ids: set[str] = set()
    for i, event in enumerate(events):
        if not isinstance(event, dict):
            errors.append(
                f"events[{i}] must be a JSON object, got {type(event).__name__}"
            )
            continue

        event_id = event.get("event_id", "")
        if not isinstance(event_id, str) or not event_id:
            errors.append(f"events[{i}].event_id must be a non-empty string")
        elif event_id in seen_ids:
            errors.append(f"Duplicate event_id: {event_id!r}")
        else:
            seen_ids.add(event_id)

        for field in ("item_id", "event_type"):
            value = event.get(field, "")
            if not isinstance(value, str) or not value:
                errors.append(
                    f"events[{i}].{field} must be a non-empty string"
                )

    return errors


def main() -> int:
    if len(sys.argv) != 2:
        print(
            f"Usage: {sys.argv[0]} <path to governance-state.json>",
            file=sys.stderr,
        )
        return 1

    path = Path(sys.argv[1])
    errors = validate(path)

    if errors:
        for err in errors:
            print(f"ERROR: {err}", file=sys.stderr)
        return 1

    print("OK: governance-state.json is valid.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
