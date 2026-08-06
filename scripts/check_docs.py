#!/usr/bin/env python3
"""Hermetic documentation quality gates for the Agora repository.

Runs deterministic, stdlib-only checks against current public/operator
documentation, the website source, and the in-app guide. It never makes
network requests and never modifies files; every finding is reported as
``file:line`` so a human can fix the source directly.

Scope (current docs only; historical and generated content is excluded to
avoid false positives):
  * README.md, AGENTS.md, project README files, and top-level docs/*.md
    (docs/architecture/ is excluded as a historical verification record)
  * CODE_OF_ENGAGEMENT.md and REGISTRY_CURATION_REFERENCE.md (link targets
    only)
  * web/src/**/*.tsx (internal href routes; prose checks cover web/src/app)
  * desktop/src/data/guideContent.ts and desktop/src/pages/Guide.tsx

Excluded: .kilo/plans/*, BACKLOG.md, root scratch notes, registry data,
compiler fixtures, and generated/build directories (target/, node_modules/,
dist/, .next/, tmp/, __pycache__/, .venv/).

Checks:
  links           markdown relative-link targets resolve on disk
                  (fragments stripped; code fences, inline code, and image
                  links ignored; external URLs skipped)
  routes          website hrefs match routes under web/src/app
  guide-ids       duplicate `id` values in guideContent.ts
  guide-topics    TOPIC_DESTINATIONS keys == GUIDE_TOPICS ids, both ways
  blob-main       stale `blob/main` GitHub links (default branch is master)
  launch-mode     claims that Agora always delegates / never launches
                  directly, or that direct launch is the global default
  spelling        canonical product spelling: Modrinth, NeoForge,
                  Last Known Good, Crash Doctor
  installer-size  fixed installer-size claims ("~10-15 MB" style)
  workflow-refs   references to .github/workflows files that do not exist
  governance-mode stale "production is read-only" claims vs compile.yml

Usage:
    python scripts/check_docs.py
    python scripts/check_docs.py --verbose
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import unquote

REPO_ROOT = Path(__file__).resolve().parent.parent

MARKDOWN_ROOT = [
    REPO_ROOT / "README.md",
    REPO_ROOT / "AGENTS.md",
    REPO_ROOT / "CODE_OF_ENGAGEMENT.md",
    REPO_ROOT / "REGISTRY_CURATION_REFERENCE.md",
] + sorted((REPO_ROOT / "docs").glob("*.md")) + [
    REPO_ROOT / "compiler" / "README.md",
    REPO_ROOT / "desktop" / "README.md",
    REPO_ROOT / "web" / "README.md",
]

PROSE_MARKDOWN = [
    REPO_ROOT / "README.md",
    REPO_ROOT / "AGENTS.md",
] + sorted((REPO_ROOT / "docs").glob("*.md")) + [
    REPO_ROOT / "compiler" / "README.md",
    REPO_ROOT / "desktop" / "README.md",
    REPO_ROOT / "web" / "README.md",
]

PROSE_TSX = sorted((REPO_ROOT / "web" / "src" / "app").rglob("*.tsx"))
ROUTE_TSX = sorted((REPO_ROOT / "web" / "src").rglob("*.tsx"))
GUIDE_CONTENT = REPO_ROOT / "desktop" / "src" / "data" / "guideContent.ts"
GUIDE_TSX = REPO_ROOT / "desktop" / "src" / "pages" / "Guide.tsx"
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
COMPILE_YML = WORKFLOW_DIR / "compile.yml"

FENCE_RE = re.compile(r"^\s*(```|~~~)")
INLINE_CODE_RE = re.compile(r"`[^`\n]*`")
MD_LINK_RE = re.compile(r"!?\[[^\]\n]*\]\(([^)\s]+)(?:\s+[\"'][^\"']*[\"'])?\)")
SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.\-]*:")

QUOTED_HREF_RE = re.compile(r"href\s*=\s*([\"'])(.*?)\1")
TEMPLATE_HREF_RE = re.compile(r"href\s*=\s*\{\s*`([^`]*)`")

GUIDE_ID_RE = re.compile(r"^\s*id:\s*'([^']+)'")
KEYWORD_LINE_RE = re.compile(r"^\s*keywords:")

LAUNCH_MODE_PATTERNS = [
    r"never\s+launch\w*\s+(?:minecraft|the\s+game|directly)",
    r"always\s+delegates",
    r"never\s+touch\w*\s+(?:microsoft|xbox|jvm)",
    r"never\s+(?:executes?|handles?|performs?)\s+.*\bjvm\b",
    r"never\s+spawn\w*\s+(?:the\s+game\s+)?process",
    r"(?:fully|entirely|completely)\s+delegated",
    r"direct\s+launch\w*\s+is\s+the\s+(?:global\s+)?default",
    r"direct\s+is\s+the\s+(?:global\s+)?default",
    r"default\w*\s+to\s+direct\s+launch",
    r"always\s+launch\w*\s+directly",
]

CANONICAL_SPELLING = [
    ("Modrinth", [r"\bmodrinth\b"]),
    ("NeoForge", [r"\bneoforge\b"]),
    ("Last Known Good", [r"\blast-known-good\b", r"\blast known good\b"]),
    ("Crash Doctor", [r"\bcrash-doctor\b", r"\bcrash doctor\b"]),
]

SIZE_TOKEN_RE = re.compile(r"\d+(?:\s*[–—-]\s*\d+)?\s*(?:MB|MiB|GB)\b")
SIZE_CONTEXT_RE = re.compile(
    r"installer|app\s+is|binary|download|package\s+size|bundle|release\s+size",
    re.IGNORECASE,
)

WORKFLOW_REF_RE = re.compile(r"\.github/workflows/([\w.\-]+\.ya?ml)")

STALE_READONLY_PATTERNS = [
    r"production\s+mode\s+remains\s+read-?only",
    r"production\s+runs\s+are\s+observation-only",
    r"production\s+is\s+currently\s+read-?only",
    r"currently\s+uses\s+read-?only\s+mode",
    r"monitor\s+mode[^.]*not\s+yet\s+activated",
    r"not\s+yet\s+activated\s+in\s+CI",
]

BLOB_MAIN_RE = re.compile(r"blob/main")


class Finding:
    __slots__ = ("check", "path", "line", "message")

    def __init__(self, check: str, path: Path, line: int, message: str) -> None:
        self.check = check
        self.path = path
        self.line = line
        self.message = message

    def __str__(self) -> str:
        rel = self.path.relative_to(REPO_ROOT)
        return f"  {self.check}: {rel}:{self.line}: {self.message}"


def markdown_prose_lines(path: Path):
    """Yield (line_number, text) for markdown prose: fences and inline code
    spans removed so links and claims inside code are not checked."""
    fence = None
    for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        m = FENCE_RE.match(line)
        if m:
            marker = m.group(1)
            if fence is None:
                fence = marker
            elif marker == fence:
                fence = None
            continue
        if fence is not None:
            continue
        yield i, INLINE_CODE_RE.sub("", line)


def check_links() -> list[Finding]:
    findings = []
    count = 0
    for path in MARKDOWN_ROOT:
        for i, line in markdown_prose_lines(path):
            for m in MD_LINK_RE.finditer(line):
                target = m.group(1)
                if m.group(0).startswith("!"):
                    continue
                if not target or target.startswith(("<", "#")):
                    continue
                if SCHEME_RE.match(target):
                    continue
                path_part = unquote(target.split("#", 1)[0]).strip()
                if not path_part:
                    continue
                count += 1
                resolved = (path.parent / path_part).resolve()
                if not resolved.exists():
                    findings.append(
                        Finding(
                            "links",
                            path,
                            i,
                            f'broken relative link "{target}" '
                            f"(resolves to {resolved})",
                        )
                    )
    print(f"  checked {count} relative markdown link target(s)")
    return findings


def build_route_patterns() -> list[list[str]]:
    app_dir = REPO_ROOT / "web" / "src" / "app"
    patterns = []
    for page in app_dir.rglob("page.tsx"):
        rel = page.parent.relative_to(app_dir)
        segments = list(rel.parts)
        patterns.append(segments)
    if not patterns:
        raise SystemExit("ERROR: no page.tsx files found under web/src/app")
    return patterns


def route_matches(segments: list[str], pattern: list[str]) -> bool:
    if len(segments) != len(pattern):
        return False
    for seg, pat in zip(segments, pattern):
        if pat.startswith("[") and pat.endswith("]"):
            continue
        if seg != pat:
            return False
    return True


def check_routes() -> list[Finding]:
    patterns = build_route_patterns()
    findings = []
    count = 0
    for path in ROUTE_TSX:
        text = path.read_text(encoding="utf-8")
        for i, line in enumerate(text.splitlines(), 1):
            for m in QUOTED_HREF_RE.finditer(line):
                href = m.group(2)
                if href.strip().startswith("/"):
                    count += 1
                    check_href(path, i, href, patterns, findings)
            for m in TEMPLATE_HREF_RE.finditer(line):
                prefix = m.group(1).split("${", 1)[0].strip()
                if prefix.startswith("/"):
                    count += 1
                    check_href(path, i, prefix, patterns, findings)
    print(f"  checked {count} website internal href(s)")
    return findings


def check_href(
    path: Path,
    line: int,
    href: str,
    patterns: list[list[str]],
    findings: list[Finding],
) -> None:
    value = href.strip().split("#", 1)[0].split("?", 1)[0].rstrip("/")
    segments = [s for s in value.split("/") if s]
    if not any(route_matches(segments, p) for p in patterns):
        findings.append(
            Finding(
                "routes",
                path,
                line,
                f'internal href "{href}" does not match any route '
                "under web/src/app",
            )
        )


def guide_topic_ids() -> dict[str, int]:
    ids: dict[str, int] = {}
    for i, line in enumerate(
        GUIDE_CONTENT.read_text(encoding="utf-8").splitlines(), 1
    ):
        m = GUIDE_ID_RE.match(line)
        if m:
            ids[m.group(1)] = i
    return ids


def check_guide_ids() -> list[Finding]:
    findings = []
    seen: dict[str, int] = {}
    for i, line in enumerate(
        GUIDE_CONTENT.read_text(encoding="utf-8").splitlines(), 1
    ):
        m = GUIDE_ID_RE.match(line)
        if not m:
            continue
        value = m.group(1)
        if value in seen:
            findings.append(
                Finding(
                    "guide-ids",
                    GUIDE_CONTENT,
                    i,
                    f'duplicate topic id "{value}" '
                    f"(first defined at line {seen[value]})",
                )
            )
        else:
            seen[value] = i
    if not seen:
        findings.append(
            Finding(
                "guide-ids",
                GUIDE_CONTENT,
                1,
                "no topic-level `id:` fields found in GUIDE_TOPICS",
            )
        )
    print(f"  found {len(seen)} guide topic id(s)")
    return findings


def check_guide_topics() -> list[Finding]:
    findings = []
    ids = guide_topic_ids()
    text = GUIDE_TSX.read_text(encoding="utf-8")
    m = re.search(
        r"TOPIC_DESTINATIONS\s*:\s*Record\s*<[^>]*>\s*=\s*\{(.*?)\n\};",
        text,
        re.DOTALL,
    )
    if not m:
        findings.append(
            Finding(
                "guide-topics",
                GUIDE_TSX,
                1,
                "could not find TOPIC_DESTINATIONS object literal",
            )
        )
        return findings
    keys = re.findall(r"(?:'([^']+)'|([A-Za-z0-9_-]+))\s*:\s*\{", m.group(1))
    keys = [a or b for a, b in keys]
    for key in keys:
        if key not in ids:
            line = next(
                (
                    i
                    for i, line in enumerate(text.splitlines(), 1)
                    if f"'{key}':" in line
                ),
                -1,
            )
            findings.append(
                Finding(
                    "guide-topics",
                    GUIDE_TSX,
                    line,
                    f'TOPIC_DESTINATIONS key "{key}" has no matching '
                    "GUIDE_TOPICS id in guideContent.ts",
                )
            )
    for tid, line in ids.items():
        if tid not in keys:
            findings.append(
                Finding(
                    "guide-topics",
                    GUIDE_CONTENT,
                    line,
                    f'GUIDE_TOPICS id "{tid}" has no TOPIC_DESTINATIONS '
                    "entry in Guide.tsx",
                )
            )
    print(
        f"  compared {len(ids)} topic id(s) against "
        f"{len(keys)} TOPIC_DESTINATIONS key(s)"
    )
    return findings


def iter_prose_lines(path: Path):
    """Yield (line_number, text) for prose checks, respecting the file type."""
    if path.suffix == ".md":
        yield from markdown_prose_lines(path)
        return
    for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if path == GUIDE_CONTENT and KEYWORD_LINE_RE.match(line):
            continue
        yield i, line


def check_blob_main() -> list[Finding]:
    findings = []
    for path in PROSE_MARKDOWN + PROSE_TSX + [GUIDE_CONTENT]:
        for i, line in iter_prose_lines(path):
            for m in BLOB_MAIN_RE.finditer(line):
                findings.append(
                    Finding(
                        "blob-main",
                        path,
                        i,
                        f'stale "blob/main" link "{m.group(0)}" '
                        "(default branch is master)",
                    )
                )
    print(f"  scanned {len(PROSE_MARKDOWN) + len(PROSE_TSX) + 1} prose file(s)")
    return findings


def check_launch_mode() -> list[Finding]:
    findings = []
    patterns = [re.compile(p, re.IGNORECASE) for p in LAUNCH_MODE_PATTERNS]
    for path in PROSE_MARKDOWN + PROSE_TSX + [GUIDE_CONTENT]:
        for i, line in iter_prose_lines(path):
            for pat in patterns:
                m = pat.search(line)
                if m:
                    findings.append(
                        Finding(
                            "launch-mode",
                            path,
                            i,
                            f'stale launch-mode claim "{m.group(0)}" '
                            "(delegated launch is the default; direct launch "
                            "is opt-in)",
                        )
                    )
                    break
    return findings


def check_spelling() -> list[Finding]:
    findings = []
    for path in PROSE_MARKDOWN + PROSE_TSX + [GUIDE_CONTENT]:
        for i, line in iter_prose_lines(path):
            for name, patterns in CANONICAL_SPELLING:
                for pat_src in patterns:
                    for m in re.finditer(pat_src, line, re.IGNORECASE):
                        if re.match(r"\.(?:png|jpe?g|webp|svg)\b", line[m.end() :], re.IGNORECASE):
                            continue
                        if m.group(0) != name:
                            findings.append(
                                Finding(
                                    "spelling",
                                    path,
                                    i,
                                    f'non-canonical spelling "{m.group(0)}" '
                                    f"(canonical: {name})",
                                )
                            )
    return findings


def check_installer_size() -> list[Finding]:
    findings = []
    for path in PROSE_MARKDOWN + PROSE_TSX + [GUIDE_CONTENT]:
        for i, line in iter_prose_lines(path):
            if not SIZE_TOKEN_RE.search(line):
                continue
            if not SIZE_CONTEXT_RE.search(line):
                continue
            m = SIZE_TOKEN_RE.search(line)
            findings.append(
                Finding(
                    "installer-size",
                    path,
                    i,
                    f'fixed installer-size claim "{m.group(0)}" '
                    "(sizes change per build; reference the release instead)",
                )
            )
    return findings


def check_workflow_refs() -> list[Finding]:
    existing = {p.name for p in WORKFLOW_DIR.glob("*.yml")}
    existing |= {p.name for p in WORKFLOW_DIR.glob("*.yaml")}
    findings = []
    for path in PROSE_MARKDOWN:
        for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for m in WORKFLOW_REF_RE.finditer(line):
                name = m.group(1)
                if name not in existing:
                    findings.append(
                        Finding(
                            "workflow-refs",
                            path,
                            i,
                            f'stale workflow reference "{name}" (not in '
                            f".github/workflows; found: "
                            f"{', '.join(sorted(existing))})",
                        )
                    )
    return findings


def governance_mode() -> str:
    if not COMPILE_YML.exists():
        return "unknown"
    m = re.search(
        r"GOVERNANCE_MODE\s*:\s*(\w+)", COMPILE_YML.read_text(encoding="utf-8")
    )
    return m.group(1) if m else "unknown"


def check_governance_mode() -> list[Finding]:
    mode = governance_mode()
    findings = []
    if mode != "monitor":
        print(f"  compile.yml GOVERNANCE_MODE={mode}; stale read-only check skipped")
        return findings
    patterns = [re.compile(p, re.IGNORECASE) for p in STALE_READONLY_PATTERNS]
    for path in PROSE_MARKDOWN:
        for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for pat in patterns:
                m = pat.search(line)
                if m:
                    findings.append(
                        Finding(
                            "governance-mode",
                            path,
                            i,
                            f'stale "production read-only" claim '
                            f'"{m.group(0)}" (compile.yml sets '
                            "GOVERNANCE_MODE: monitor)",
                        )
                    )
                    break
    return findings


CHECKS = [
    ("links", check_links),
    ("routes", check_routes),
    ("guide-ids", check_guide_ids),
    ("guide-topics", check_guide_topics),
    ("blob-main", check_blob_main),
    ("launch-mode", check_launch_mode),
    ("spelling", check_spelling),
    ("installer-size", check_installer_size),
    ("workflow-refs", check_workflow_refs),
    ("governance-mode", check_governance_mode),
]


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Hermetic documentation quality gates: deterministic, stdlib-only "
            "checks of internal links and current documentation claims. "
            "No network requests are made and no files are modified."
        ),
        epilog=(
            "Scope: README.md, AGENTS.md, project README files, docs/*.md (docs/architecture/ excluded as a "
            "historical record), CODE_OF_ENGAGEMENT.md and "
            "REGISTRY_CURATION_REFERENCE.md (links only), web/src/**/*.tsx "
            "(routes) and web/src/app/**/*.tsx (prose), "
            "desktop/src/data/guideContent.ts and desktop/src/pages/Guide.tsx. "
            "Historical plan files, scratch notes, and generated/build "
            "directories are excluded."
        ),
    )
    parser.add_argument("--verbose", action="store_true", help="list scanned files")
    args = parser.parse_args()

    if args.verbose:
        for path in MARKDOWN_ROOT + PROSE_TSX + [GUIDE_CONTENT, GUIDE_TSX]:
            print(f"  scan: {path.relative_to(REPO_ROOT)}")
        print()

    findings: list[Finding] = []
    failed: list[str] = []
    for name, fn in CHECKS:
        results = fn()
        findings.extend(results)
        if results:
            failed.append(name)
            print(f"FAIL {name} ({len(results)} finding(s))")
            for finding in results:
                print(str(finding))
        else:
            print(f"OK   {name}")

    print()
    if findings:
        print(
            f"FAIL: {len(findings)} finding(s) across "
            f"{len(failed)} check(s): {', '.join(failed)}.",
            file=sys.stderr,
        )
        print(
            "No files were modified; no network requests were made. "
            "Fix the documented lines directly.",
            file=sys.stderr,
        )
        return 1
    print(
        f"OK: all documentation quality gates passed ({len(CHECKS)} checks). "
        "No files were modified; no network requests were made."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
