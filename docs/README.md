# Agora documentation

Agora's documentation is organized by audience and by depth. The goal is to help a player complete a task quickly without forcing maintainers, CLI users, and governance operators into the same page.

## Choose the right guide

| Audience | Start here | What belongs there |
| --- | --- | --- |
| Players using the desktop app | **Help & Guide** inside Agora | Task-based guidance tied to the current UI |
| Players before installation or sharing a link | [Website documentation](https://agoramc.com/docs/) | Download, first run, launch modes, recovery, privacy, and troubleshooting |
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

Historical implementation plans are retained under [`docs/archive/`](./archive/) for audit. Their archive banners identify them as non-authoritative; current behavior comes from the built product, CLI help, tests, and the references above.

### 3. Maintainer and operator documentation

Release, signing, compiler, governance-monitor, and incident-recovery procedures belong in narrowly scoped maintainer documents. They should not dominate the project landing page or the player-facing website.

## Source-of-truth rules

- The current interface is the source of truth for button names and navigation.
- `crates/agora/src/main.rs` is the source of truth for CLI syntax. `docs/CLI.md` explains how to use that interface.
- `desktop/src/data/guideContent.ts` is the source of truth for in-app guide copy.
- `REGISTRY_CURATION_REFERENCE.md` is the source of truth for registry manifests.
- `CODE_OF_ENGAGEMENT.md` is the source of truth for review conduct.
- Workflow files are the source of truth for automated release triggers.
- Secret values never belong in documentation, examples, screenshots, or support bundles.

## Documentation update checklist

A feature change is not complete until its documentation impact is reviewed.

1. Update the in-app guide when a user-visible workflow, label, warning, or recovery action changes.
2. Update `docs/CLI.md` and CLI help when a command, flag, output shape, or exit code changes.
3. Update website documentation when pre-install requirements or first-run behavior changes.
4. Update operator documents when workflows, variables, signing, or governance state changes.
5. Test commands against a temporary data directory.
6. Check internal and website links.
7. Avoid copying the same detailed procedure into multiple files. Link to the canonical page instead.

## README contract

Keep the root README scannable and in this order: product purpose; download and documentation links; first-run player path; key capabilities; safety and recovery boundaries; project principles; compact contributor quick start; canonical maintainer references; license and community links. Operator incident procedures and full environment tables do not belong there.
