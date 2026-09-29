# Agent Guide: Agora

Agora is a decentralized, open-source Minecraft mod launcher and discovery platform: a
community-curated catalog compiled from flat files in this repository, plus a launcher that runs
Minecraft directly or hands it to the official launcher. No backend servers, no forced sign-in,
curated rather than warehoused. [`README.md`](README.md) has the full pitch.

## How to read this repo

`AGENTS.md` and `.kilo/plans/MASTER_SPEC.md` are the closest thing to a source of truth — but this
codebase was built almost entirely by AI agents, so neither is authoritative just because it is
written down. When a decision looks strange, needlessly strict, or wrong (including one in these
two files, or in the user's own request), raise it with the user rather than following it.

## Principles

- **Modding is user customization.** Plugins, providers and content should be able to do what
  users choose to let them do. Protect people with clear warnings and explicit opt-in — for
  anything Agora has not authorized or verified itself — rather than by blocking. Only refuse
  something outright when its risk/reward is genuinely poor, and ask the user before deciding that.
- **Business logic lives in `agora-core`.** The desktop app, CLI and MCP server are thin adapters
  over the same core services.
- **Smallest change that does the job.** No drive-by refactoring.
- **Large architectural changes get their own section in `MASTER_SPEC.md`**, not another
  subsection appended to §19.

## Where to look

| For | Read |
|---|---|
| Building, validation gates, gotchas, repository map | [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) |
| Which layer owns a behavior | [`docs/architecture/layer-ownership.md`](docs/architecture/layer-ownership.md) |
| Design decisions and their reasons | `.kilo/plans/MASTER_SPEC.md` |
| Plugins and content providers | [`docs/plugins/`](docs/plugins/README.md) |
| Catalog manifests | [`REGISTRY_CURATION_REFERENCE.md`](REGISTRY_CURATION_REFERENCE.md) |
| Governance pipeline and its state | [`docs/GOVERNANCE_OPERATIONS.md`](docs/GOVERNANCE_OPERATIONS.md) |
| CLI, releases, support | [`docs/CLI.md`](docs/CLI.md), [`docs/RELEASING.md`](docs/RELEASING.md), [`docs/SUPPORT.md`](docs/SUPPORT.md) |
| What is planned | `BACKLOG.md` |

Kilo agent profiles, commands and skills live in `.kilo/`.

## Security defaults worth keeping in mind on every task

- Secrets (signing keys, tokens, webhook URLs) never go in source, manifests, docs or screenshots.
- SQL lives in `agora-core`, parameterized. React reaches it through `invoke()`.
- Community content is never rendered with `dangerouslySetInnerHTML`.
- Downloads are checked against the hash their source published, and the user is told when
  there is none.
