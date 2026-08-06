# Agent Guide: Agora

## Mission & Ethos

Agora is a decentralized, ad-free, open-source Minecraft mod launcher and discovery platform. It returns platform control to the community by treating the GitHub repository itself as the database: flat-file manifests are compiled into a signed SQLite registry. Delegated launch is the global default; an explicit direct mode performs Microsoft authentication and JVM execution inside Agora.

> "If CurseForge were a beer, this would be Agora."

Core values:
- **$0.00/month server footprint.** No backend services; data ships via GitHub Release Assets and static sites.
- **Secure defaults.** Delegated launch keeps Microsoft/Xbox auth and JVM execution in the official launcher by default; direct launch is explicit and optional.
- **Curated, not warehoused.** Boutique quality over infinite inventory; every entry is community reviewed.

## Directory Map

| Path | Purpose |
|---|---|
| `registry/` | Curated flat-file manifests (mods, packs, shaders, resource packs, servers, datapacks, worlds, governance) |
| `crash-signatures/` | Crash triage regex definitions |
| `loader-manifests/` | Pinned modloader URLs + SHA-256 hashes |
| `compiler/` | Python compiler that builds `registry.db` from the flat files |
| `desktop/` | Tauri desktop app (Rust backend, React frontend) |
| `web/` | Next.js static web directory |
| `scripts/` | Sanity-check and utility scripts |
| `.github/` | Workflows, issue templates, and governance forms |
| `.kilo/` | Kilo AI tooling configuration, agent profiles, commands, and skills |
| `.kilo/plans/MASTER_SPEC.md` | Authoritative engineering blueprint (read-only for agents) |
| crates/ | Shared Rust workspace (agora-core shared lib, agora CLI binary) |
| BACKLOG.md | Phase-by-phase task tracker |
| CODE_OF_ENGAGEMENT.md | Canonical review-conduct rules |
| REGISTRY_CURATION_REFERENCE.md | Self-contained manifest-authoring reference |

## Agent Roles

| Agent | Use for |
|---|---|
| `code` | Primary implementation in Rust, TypeScript/React, and Python |
| `security` | Security audits, threat-model reviews, hardening guidance |
| `registry-curator` | Adding or reviewing registry entries and loader manifests |
| `reviewer` | Focused code review across security, logic, and deploy safety |

## Conventions

- Treat `AGENTS.md` and `.kilo/plans/MASTER_SPEC.md` as the source of truth. `MASTER_SPEC.md` §0-§18 are the original design spec; §19 captures architectural-evolution decisions and supersedes the earlier prose where they conflict. When the architecture genuinely pivots, append a new subsection under §19 (do NOT rewrite §0-§18 design prose as drive-by edits -- those are preserved for decision-rationale value).
- Prefer the smallest change that satisfies the request; avoid drive-by refactoring.
- Edit files via Kilo tools. Do not manually stage or edit files outside the project directory.
- After registry/loader/crash-signature changes, run `/registry`.
- After desktop changes, run `/desktop`.
- After web changes, run `/web`.
- Never commit or push from an agent session unless explicitly requested.
- Do not modify `.lock` files or existing data history in `registry/archived/`.
- Security defaults:
  - **Whitelist over denylist** for capabilities, shell scopes, and network access.
  - Verify every download with SHA-256 and package signatures.
  - Use `tauri-plugin-sql` with parameterized queries only.
  - Never render community content with `dangerouslySetInnerHTML`.
  - Never store secrets, tokens, or private keys in source files or manifests.

## MCP Server

The shipped Agora launcher app exposes an MCP server on `127.0.0.1:39741` when the user has *AI / MCP Server* enabled in Settings (disabled by default in the shipped app). Every request requires the persistent Bearer token generated when MCP is enabled and shown in Settings; localhost binding remains defense in depth. For local development, this project's `.kilo/kilo.json` enables the Kilo MCP client (`enabled: true`) to talk to a locally-running launcher instance. Keep MCP calls stateless and avoid privileged operations without explicit user approval.

## Governance Modes and Tracked State

The compiler governance pipeline (`compiler/governance.py`) detects vote-surge anomalies and manages quarantine state:

| Mode | Reads GitHub | Writes state file | Discord alerts |
|---|---|---|---|
| `off` | No | No | No |
| `read-only` | Yes | No | No |
| `monitor` | Yes | Yes | Yes |

- **State path**: production tracks `registry/governance/governance-state.json` and passes it explicitly as both state input and output. The compiler default remains `<output-dir>/governance-state.json` for isolated runs.
- **Repo resolution**: `AGORA_GOVERNANCE_REPO` → `AGORA_REGISTRY_REPO` → `GITHUB_REPOSITORY`.
- **Decisions**: Curators edit `registry/governance/quarantine_decisions.json` (compiler never writes it). Each entry maps `event_id` to `accepted` (lift quarantine) or `rejected` (permanently exclude).
- **Recovery**: production CI rejects missing, malformed, mismatched, duplicate, or incomplete state. Restore the last valid file from Git history or commit a curator-approved empty production envelope, then run `python scripts/validate_governance_state.py registry/governance/governance-state.json`.
- **Production currently uses `monitor`**: `.github/workflows/compile.yml` pins the production policy, validates the tracked state, requires alert configuration, and may commit meaningful state transitions.
- **Workflow ownership**: the loader-refresh workflow is the sole committer for `loader-manifests/`. The nightly governance commit stages only `registry/governance/governance-state.json`; signed registry artifacts are release assets, not Git commits.

### Local diagnostics

```powershell
# Read-only does not write monitor state or send monitor alerts.
$env:GITHUB_TOKEN = (gh auth token)
python compiler/compile.py --skip-sign --governance-mode read-only --governance-policy production --governance-repo agora-mc/Agora-Launcher --governance-state-in registry/governance/governance-state.json --out tmp/governance-read-only/registry.db
```

## Environment Variables

- `ED25519_PRIVATE_KEY` — CI-only Ed25519 key used to sign `registry.db`. Never expose or bundle it.
- `GITHUB_TOKEN` — Standard GitHub token for compiler and CI operations.
