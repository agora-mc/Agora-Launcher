#!/usr/bin/env python3
"""Build docs-web.json from the repository's markdown documentation.

Reads the markdown documentation under docs/ (and the root-level
CODE_OF_ENGAGEMENT.md / REGISTRY_CURATION_REFERENCE.md), derives a stable
slug, title, description, and audience/group classification for each page,
rewrites internal relative .markdown links to the website's /docs/<slug>
routes, and emits a single docs-web.json that the website consumes at build
time. This mirrors how compiler/compile.py emits registry-web.json.

The actual markdown -> HTML rendering happens in the website with
react-markdown (an existing dependency); this script only aggregates,
indexes, classifies, and routes links so the site can render docs for easy
navigation.

The emitted ``body`` has the leading H1 removed: the website renders ``title``
in its own page header, so leaving the H1 in place printed the heading twice.
``content`` is retained unchanged for consumers that want the raw source.

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
# Only these exact repo-root paths are skipped; a nested README.md such as
# docs/README.md is retained as documentation.
SKIP_PATHS = {
    Path("AGENTS.md"),
    Path("README.md"),
    Path("BACKLOG.md"),
}

H1_RE = re.compile(r"^\s*#\s+(.+?)\s*$")
MD_LINK_RE = re.compile(r"\[([^\]]*)\]\(([^)\s]+)(?:\s+[\"'][^\"']*[\"'])?\)")
SLUG_CLEAN_RE = re.compile(r"[^a-z0-9]+")
INLINE_CODE_RE = re.compile(r"`[^`\n]+`")
EMPHASIS_RE = re.compile(r"(\*{1,3}|_{1,3})(.+?)\1")

# Audience/group classification for the website's grouped documentation nav.
#
# audience is the top-level split the site navigates by:
#   user      - anyone running Agora, including power users driving the CLI
#   developer - contributors, curators, maintainers, and operators
#   internal  - working notes and archived plans; still published so no
#               cross-reference 404s, but kept out of the main nav
#
# Rules are matched longest-prefix-first against the repo-relative POSIX path,
# so a directory prefix ("docs/architecture/") classifies everything under it.
DOC_CATEGORIES: dict[str, tuple[str, str]] = {
    "docs/TROUBLESHOOTING.md": ("user", "Fix a problem"),
    "docs/SUPPORT.md": ("user", "Fix a problem"),
    "docs/CLI.md": ("user", "Power tools"),
    "CODE_OF_ENGAGEMENT.md": ("developer", "Contribute and curate"),
    "REGISTRY_CURATION_REFERENCE.md": ("developer", "Contribute and curate"),
    "docs/README.md": ("developer", "Contribute and curate"),
    "docs/DEVELOPMENT.md": ("developer", "Build Agora"),
    "docs/desktop-native-smoke-checklist.md": ("developer", "Build Agora"),
    "docs/architecture/": ("developer", "Architecture"),
    "docs/RELEASING.md": ("developer", "Ship and operate"),
    "docs/GOVERNANCE_OPERATIONS.md": ("developer", "Ship and operate"),
    "docs/interactive/": ("internal", "Working notes"),
    "docs/archive/": ("internal", "Archive"),
}

DEFAULT_CATEGORY = ("internal", "Working notes")

# Display order for the website's grouped nav. Groups not listed here sort
# last, alphabetically, so a new group is visible rather than silently hidden.
AUDIENCE_ORDER = ["user", "developer", "internal"]
GROUP_ORDER = [
    "Fix a problem",
    "Power tools",
    "Contribute and curate",
    "Build Agora",
    "Architecture",
    "Ship and operate",
    "Working notes",
    "Archive",
]


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


def classify(rel: Path) -> tuple[str, str]:
    """Return the (audience, group) pair for a repo-relative markdown path.

    Longest matching prefix wins, so an exact file rule overrides the
    directory rule containing it. Unclassified documents fall back to
    ``internal`` rather than leaking into the player-facing navigation.
    """
    key = rel.as_posix()
    best: tuple[str, str] | None = None
    best_len = -1
    for prefix, category in DOC_CATEGORIES.items():
        if key == prefix or (prefix.endswith("/") and key.startswith(prefix)):
            if len(prefix) > best_len:
                best, best_len = category, len(prefix)
    return best or DEFAULT_CATEGORY


def strip_title(content: str) -> str:
    """Drop the leading H1 so the website does not render the title twice.

    Only a heading that precedes any other prose is removed; blank lines and
    HTML comments before it are tolerated. Documents without a leading H1
    (CODE_OF_ENGAGEMENT.md opens with a blockquote) are returned unchanged.
    """
    lines = content.splitlines()
    for i, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or stripped.startswith("<!--"):
            continue
        if not H1_RE.match(line):
            return content
        rest = lines[i + 1 :]
        while rest and not rest[0].strip():
            rest.pop(0)
        return "\n".join(lines[:i] + rest)
    return content


def plain_text(markdown: str) -> str:
    """Flatten inline markdown to prose for card and meta-description use.

    Descriptions are rendered as plain text, so `code`, **bold**, _emphasis_,
    and [links](target) must lose their punctuation rather than show it.
    """
    text = MD_LINK_RE.sub(lambda m: m.group(1), markdown)
    text = INLINE_CODE_RE.sub(lambda m: m.group(0).strip("`"), text)
    text = EMPHASIS_RE.sub(r"\2", text)
    return " ".join(text.split())


def extract_description(content: str, limit: int = 240) -> str:
    """Return the first non-empty paragraph (skipping headings/frontmatter)."""
    for line in content.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("#", ">", "```", "---", "|")):
            continue
        # Skip list items and images; prefer a plain prose sentence.
        if stripped.startswith(("-", "*", "[!")):
            continue
        description = plain_text(stripped)
        if not description:
            continue
        if len(description) > limit:
            description = description[: limit - 1].rstrip() + "…"
        return description
    return ""


def rewrite_links(content: str, doc_rel: Path, slug_by_rel: dict[str, str]) -> str:
    """Rewrite internal relative .md links to /docs/<slug> routes.

    Absolute URLs, anchors, and non-markdown targets are left untouched.
    Unknown markdown targets are also left untouched. A link to a directory
    that has a README.md doc is routed to that doc; note the directory test
    runs against the process working directory, so this only resolves when the
    script is invoked from the repository root.
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
        audience, group = classify(rel)
        routed = rewrite_links(content, rel, slug_by_rel)
        docs.append(
            {
                "slug": slug_by_rel[rel.as_posix()],
                "title": title,
                "description": extract_description(content),
                "audience": audience,
                "group": group,
                "path": rel.as_posix(),
                "content": routed,
                "body": strip_title(routed),
            }
        )
    return {
        "schema_version": 2,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "audience_order": AUDIENCE_ORDER,
        "group_order": GROUP_ORDER,
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
