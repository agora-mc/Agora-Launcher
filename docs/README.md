# How Agora's documentation is organized

Agora's documentation is organized by audience and by depth. The goal is to help a player complete a task quickly without forcing maintainers, CLI users, and governance operators through the same page. This file describes the system; it is not itself a starting point for readers.

## Choose the right guide

| Audience | Start here | What belongs there |
| --- | --- | --- |
| Players using the desktop app | **Help & Guide** inside Agora | Task-based guidance tied to the current UI |
| Players before installation or sharing a link | [Website documentation](https://agoramc.com/docs) | Audience router, install and first run, visual tour, and the published task guides |
| CLI users and automation authors | [CLI reference](./CLI.md) | Commands, flags, output formats, safety, examples, and exit codes |
| Contributors | [Development guide](./DEVELOPMENT.md) | Local builds, tests, environment boundaries, and repository layout |
| Release maintainers | [Release guide](./RELEASING.md) | Registry and desktop release checklists |
| Governance operators | [Governance operations](./GOVERNANCE_OPERATIONS.md) | Read-only diagnostics, monitor state, decisions, and incident recovery |
| Registry curators | [Registry curation reference](../REGISTRY_CURATION_REFERENCE.md) | Manifest authoring and review rules |
| Review participants | [Code of Engagement](../CODE_OF_ENGAGEMENT.md) | Conduct and review boundaries |
| Troubleshooting and support | [Troubleshooting](./TROUBLESHOOTING.md) | Safe diagnosis and evidence collection |
| Support evidence and local data | [Support reference](./SUPPORT.md) | Data roots, logs, versions, minimal support bundles, redaction, and reset boundaries |

## Three tiers of detail

### 1. Task guidance

The in-app guide and website answer questions such as:

- How do I create or import an instance?
- Why is launch blocked?
- How do I switch to a compatible loader?
- What does a snapshot protect?
- How do I prepare an instance for offline play?

Task guidance should use the same labels the current interface uses.

### 2. Reference documentation

Reference pages describe stable interfaces that are awkward to teach inside the application:

- CLI syntax and exit codes
- data and log discovery
- reproducible support commands
- file-format boundaries
- troubleshooting evidence
- development prerequisites

Reference documentation should be searchable and linkable.

Superseded implementation plans and completed review logs are not kept as files. They live in Git history, where `git log --diff-filter=D` finds them by path. Current behavior comes from the built product, CLI help, tests, and the references above — a checked-in plan that no longer matches the product is worse than no plan at all.

### 3. Maintainer and operator documentation

Release, signing, compiler, governance-monitor, and incident-recovery procedures belong in narrowly scoped maintainer documents. They should not dominate the project landing page or the player-facing website.

## How the website is assembled

The website does not keep its own copy of any documentation. Two build steps feed it:

- `scripts/build_docs_web.py` reads every markdown file under `docs/` plus the configured root documents, and writes `docs-web.json`. For each page it derives the slug, title, description, and an **audience** (`user`, `developer`, `internal`) and **group** used to build the site's grouped navigation. It also emits a `body` with the leading H1 removed, because the website renders the title in its own page header.
- The website imports `desktop/src/data/guideContent.ts` directly and publishes those topics at `/docs/guides/`. There is no second copy of the guide text, so the app and the website cannot describe the product differently.

New markdown is published automatically, but it defaults to the `internal` audience — visible only under the collapsed "Working notes and archive" section. Add a rule to `DOC_CATEGORIES` in `scripts/build_docs_web.py` to place a page in front of players or contributors.

Pages classified `internal` are still published so cross-references never break, and they render with a banner marking them as non-authoritative working notes. No document currently carries that classification, so the section does not appear in the navigation; the bucket exists to catch new files that nobody has classified yet.

## Source-of-truth rules

- The current interface is the source of truth for button names and navigation.
- `crates/agora/src/main.rs` is the source of truth for CLI syntax. `docs/CLI.md` explains how to use that interface.
- `desktop/src/data/guideContent.ts` is the source of truth for in-app guide copy **and** for the website's published task guides.
- `REGISTRY_CURATION_REFERENCE.md` is the source of truth for registry manifests.
- `CODE_OF_ENGAGEMENT.md` is the source of truth for review conduct.
- Workflow files are the source of truth for automated release triggers.
- Secret values never belong in documentation, examples, screenshots, or support bundles.

## Documentation update checklist

A feature change is not complete until its documentation impact is reviewed.

1. Update the in-app guide when a user-visible workflow, label, warning, or recovery action changes. This updates the website's task guides at the same time.
2. Update `docs/CLI.md` and CLI help when a command, flag, output shape, or exit code changes.
3. Update the website's hand-written pages (`web/src/app/docs/`) when pre-install requirements, first-run behavior, or the screenshot tour changes.
4. Classify any new markdown document in `DOC_CATEGORIES` so it does not land in the internal bucket by default.
5. Update operator documents when workflows, variables, signing, or governance state changes.
6. Test commands against a temporary data directory.
7. Check internal and website links.
8. Avoid copying the same detailed procedure into multiple files. Link to the canonical page instead.

## README contract

Keep the root README scannable and in this order: product purpose; download and documentation links; first-run player path; key capabilities; safety and recovery boundaries; project principles; compact contributor quick start; canonical maintainer references; license and community links. Operator incident procedures and full environment tables do not belong there.
