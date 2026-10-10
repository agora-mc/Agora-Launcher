# Agent Guide: Agora

Agora is a decentralized, open-source Minecraft mod launcher and discovery platform: a
community-curated catalog compiled from flat files in this repository, plus a launcher that runs
Minecraft directly or hands it to the official launcher. No backend servers, no forced sign-in,
curated rather than warehoused. [`README.md`](README.md) has the full pitch. Support for games
beyond Minecraft is being built ([`MASTER_SPEC.md`](MASTER_SPEC.md) §26).

This file is the one instruction file for every coding agent, Claude Code included (it reads
`AGENTS.md` itself when no `CLAUDE.md` exists, so don't add one).

## How to read this repo

`AGENTS.md` and [`MASTER_SPEC.md`](MASTER_SPEC.md) are the closest thing to a source of truth — but this
codebase was built almost entirely by AI agents, so neither is authoritative just because it is
written down. When a decision looks strange, needlessly strict, or wrong (including one in these
two files, or in the user's own request), raise it with the user rather than following it.

## Principles

- **Modding is user customization.** Plugins, providers and content should be able to do what
  users choose to let them do. Protect people with clear warnings and explicit opt-in — for
  anything Agora has not authorized or verified itself — rather than by blocking. Only refuse
  something outright when its risk/reward is genuinely poor, and ask the user before deciding that.
- **Business logic lives in `agora-core`, game-specific logic in that game's package**
  (`agora-game-minecraft`, `agora-game-creation`). The desktop app, CLI and MCP server are thin adapters over the same
  core services and packages.
- **Smallest change that does the job.** No drive-by refactoring.
- **Large architectural changes get their own section in `MASTER_SPEC.md`**, not another
  subsection appended to §19.

## Where to look

| For | Read |
|---|---|
| Building, validation gates, gotchas, repository map | [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) |
| Which layer owns a behavior | [`docs/architecture/layer-ownership.md`](docs/architecture/layer-ownership.md) |
| Design decisions and their reasons | [`MASTER_SPEC.md`](MASTER_SPEC.md) |
| Plugins and content providers | [`docs/plugins/`](docs/plugins/README.md) |
| Catalog manifests | [`REGISTRY_CURATION_REFERENCE.md`](REGISTRY_CURATION_REFERENCE.md) |
| Governance pipeline and its state | [`docs/GOVERNANCE_OPERATIONS.md`](docs/GOVERNANCE_OPERATIONS.md) |
| CLI, releases, support | [`docs/CLI.md`](docs/CLI.md), [`docs/RELEASING.md`](docs/RELEASING.md), [`docs/SUPPORT.md`](docs/SUPPORT.md) |
| What is planned | `BACKLOG.md` |

Kilo agent profiles, commands and skills live in `.kilo/`.

## Before calling work done

Run the gates for what you touched. They are listed, with their gotchas, in
[`docs/DEVELOPMENT.md` → Core validation](docs/DEVELOPMENT.md#core-validation). The hermetic
ones take seconds:

```bash
python scripts/check_architecture.py && python scripts/check_docs.py && python scripts/check_tauri_bindings.py --check
```

## Boundaries that are enforced by scripts

- `agora-core` is game-agnostic and never references a game package. Minecraft lives in
  `agora-game-minecraft`, whose use of core may only shrink (`scripts/game_package_core_budget.json`);
  every other game package depends on `agora-game-api` only.
- `agora-core` does not depend on `tauri`, `clap` or MCP protocol types. A platform primitive is a
  trait in core, implemented in the adapter.
- `desktop/src/features/interactive/` has a stricter import boundary; see
  [layer-ownership](docs/architecture/layer-ownership.md#interactive-feature-boundary-desktopsrcfeaturesinteractive).

## Environment

The user's machine is Windows 11 with PowerShell; cloud sessions run Linux. Use a disposable data
root for experiments (`AGORA_DATA_DIR`, or `--data-dir` for the CLI). Microsoft credentials live in
the OS credential store and are not isolated by it, so do not sign in or out with a real account
while testing.

## Security defaults worth keeping in mind on every task

- Secrets (signing keys, tokens, webhook URLs) never go in source, manifests, docs or screenshots.
- SQL lives in `agora-core`, parameterized. React reaches it through `invoke()`. (Until §26 Phase 5,
  `agora-game-minecraft` still holds the queries that moved with it from core; new SQL goes in core.)
- Community content is never rendered with `dangerouslySetInnerHTML`.
- Downloads are checked against the hash their source published, and the user is told when
  there is none.
