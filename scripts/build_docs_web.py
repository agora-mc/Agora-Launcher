#!/usr/bin/env python3
"""Build docs-web.json from the repository's markdown documentation.

Reads the markdown documentation under docs/ (and the root-level
CODE_OF_ENGAGEMENT.md / REGISTRY_CURATION_REFERENCE.md), derives a stable
slug, title, and description for each page, rewrites internal relative
.markdown links to the website's /docs/<slug> routes, and emits a single
docs-web.json that the website consumes at build time. This mirrors how
compiler/compile.py emits registry-web.json.

The actual markdown -> HTML rendering happens in the website with
react-markdown (an existing dependency); this script only aggregates,
indexes, and routes links so the site can render docs for easy navigation.

Usage:
    python scripts/build_docs_web.py [--out path/to/docs-web.json]

Pure helpers are unit-tested in scripts/test_scripts.py.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import unquote

REPO_ROOT = Path(__file__).resolve().parent.parent
DOCS_ROOT = REPO_ROOT / "docs"

# Root-level markdown documents that belong in the public documentation.
ROOT_DOCS = [
    "CODE_OF_ENGAGEMENT.md",
    "REGISTRY_CURATION_REFERENCE.md",
]

# Root-level repo files that are not part of the public documentation.
# Only these exact repo-root paths are skipped — nested docs/README.md and
# docs/archive/README.md are retained as documentation.
SKIP_PATHS = {
    Path("AGENTS.md"),
    Path("README.md"),
    Path("BACKLOG.md"),
}

H1_RE = re.compile(r"^\s*#\s+(.+?)\s*$")
MD_LINK_RE = re.compile(r"\[([^\]]*)\]\(([^)\s]+)(?:\s+[\"'][^\"']*[\"'])?\)")
SLUG_CLEAN_RE = re.compile(r"[^a-z0-9]+")


def collect_doc_paths() -> list[Path]:
    """Return repo-relative markdown paths to publish on the website.

    Includes every .md under docs/ (archives included as historical records)
    plus the configured root-level documents, excluding internal files like
    AGENTS.md / README.md / BACKLOG.md.
    """
    paths: set[Path] = set()
    if DOCS_ROOT.exists():
        for path in DOCS_ROOT.rglob("*.md"):
            paths.add(path.relative_to(REPO_ROOT))
    for name in ROOT_DOCS:
        candidate = REPO_ROOT / name
        if candidate.exists():
            paths.add(Path(name))
    return sorted(
        (p for p in paths if p not in SKIP_PATHS),
        key=lambda p: str(p).lower(),
    )


def slugify(rel: Path) -> str:
    """Derive a URL-safe slug from a repo-relative markdown path.

    docs/CLI.md -> cli; docs/architecture/baseline.md -> architecture-baseline;
    CODE_OF_ENGAGEMENT.md -> code-of-engagement. The leading docs/ directory is
    dropped so routes read /docs/cli rather than /docs/docs-cli.
    """
    parts = list(rel.with_suffix("").parts)
    if parts and parts[0] == "docs":
        parts = parts[1:]
    joined = "-".join(parts).replace("_", "-")
    slug = SLUG_CLEAN_RE.sub("-", joined.lower()).strip("-")
    return slug or "doc"


def extract_title(content: str, rel: Path) -> str:
    """Use the first H1 heading, falling back to a humanized filename."""
    for line in content.splitlines():
        m = H1_RE.match(line)
        if m:
            return m.group(1).strip()
    return rel.with_suffix("").name.replace("_", " ").replace("-", " ").title()


def extract_description(content: str, limit: int = 240) -> str:
    """Return the first non-empty paragraph (skipping headings/frontmatter)."""
    for line in content.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("#", ">", "```", "---", "|")):
            continue
        # Skip list items and images; prefer a plain prose sentence.
        if stripped.startswith(("-", "*", "[!")):
            continue
        description = " ".join(stripped.split())
        if len(description) > limit:
            description = description[: limit - 1].rstrip() + "…"
        return description
    return ""


def rewrite_links(content: str, doc_rel: Path, slug_by_rel: dict[str, str]) -> str:
    """Rewrite internal relative .md links to /docs/<slug> routes.

    Absolute URLs, anchors, and non-markdown targets are left untouched.
    Unknown markdown targets are also left untouched. A link to a directory
    that has a README.md doc (e.g. ./archive/) is routed to that doc.
    """

    def resolve_target(target: str) -> Path | None:
        path_part, _, _ = target.partition("#")
        if not path_part or path_part.startswith("http"):
            return None
        try:
            resolved = (REPO_ROOT / doc_rel.parent / unquote(path_part)).resolve()
            return resolved.relative_to(REPO_ROOT)
        except (OSError, ValueError):
            return None

    def replace(match: re.Match[str]) -> str:
        label = match.group(1)
        target = match.group(2)
        if target.startswith(("<", "#", "http:", "https:", "mailto:", "data:")):
            return match.group(0)
        rel = resolve_target(target)
        if rel is None:
            return match.group(0)
        if rel.is_dir():
            rel = rel / "README.md"
        if rel.suffix.lower() != ".md":
            return match.group(0)
        slug = slug_by_rel.get(rel.as_posix())
        if slug is None:
            return match.group(0)
        path_part, _, anchor = target.partition("#")
        href = f"/docs/{slug}"
        if anchor:
            href += f"#{anchor}"
        return f"[{label}]({href})"

    return MD_LINK_RE.sub(replace, content)


def build_docs_json() -> dict[str, Any]:
    """Assemble the docs-web.json payload from the repository markdown."""
    doc_paths = collect_doc_paths()
    slug_by_rel = {rel.as_posix(): slugify(rel) for rel in doc_paths}
    docs: list[dict[str, Any]] = []
    for rel in doc_paths:
        source = REPO_ROOT / rel
        content = source.read_text(encoding="utf-8")
        title = extract_title(content, rel)
        docs.append(
            {
                "slug": slug_by_rel[rel.as_posix()],
                "title": title,
                "description": extract_description(content),
                "path": rel.as_posix(),
                "content": rewrite_links(content, rel, slug_by_rel),
            }
        )
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "docs": docs,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=REPO_ROOT / "docs-web.json",
        help="Output path for docs-web.json (default: repo root)",
    )
    args = parser.parse_args(argv)

    payload = build_docs_json()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    tmp = args.out.with_suffix(args.out.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2)
        fh.write("\n")
    tmp.replace(args.out)
    print(
        f"Wrote docs-web.json ({len(payload['docs'])} docs) to {args.out}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
