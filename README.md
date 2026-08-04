# Agora Minecraft Launcher

> This is not a warehouse. This is a boutique.

A decentralized, ad-free, open-source Minecraft mod launcher and discovery platform built to return platform control to the community. Curated mods, packs, shaders, and more are delivered as a signed SQLite database compiled nightly from flat JSON manifests stored directly in this repository.

## Links
- Discord - https://discord.gg/56tpsa2sTZ
- Site - https://agoramc.com/

## Mission

The **"Agora"** mission is simple: bypass centralized commercial infrastructure and serve a high-quality, community-governed catalog directly from developer-controlled sources. If traditional mod platforms are a beer, this is Agora.

Core principles:

- **$0/month server footprint** — GitHub, GitHub Release Assets, and the official Mojang launcher handle everything.
- **Security by delegation (optional)** — The default path delegates Microsoft/Xbox auth and JVM execution to the official Mojang launcher. An opt-in in-process mode performs MSA auth + direct JVM spawn for users who want tighter integration.
- **Decentralized governance** — Votes, reviews, and triage live as structured GitHub interactions.
- **Modrinth independence** — Primary source is `github_release`; Modrinth is an optional fallback.

## Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop backend | Tauri (Rust; crate `agora-desktop`) |
| Shared Rust library | `crates/agora-core/` -- business logic shared with CLI |
| Standalone CLI | `crates/agora/` -> `agora` binary |
| Desktop frontend | React + Tailwind CSS |
| Web directory | Next.js (static) |
| Client DB | SQLite (`tauri-plugin-sql`) |
| Compiler | Python (GitHub Actions) |
| Game execution | In-process MSA auth + direct JVM spawn (primary), Mojang Launcher delegation (fallback) |
| AI integration | Local MCP server (loopback only) |
| Data hosting | GitHub Release Assets |

## Monorepo Layout

```
/registry/          Curated data store (the "GitHub database")
  mods/            Curated mod manifests
  packs/           Curated modpack manifests
  shaders/         Shader pack entries
  resourcepacks/   Resource pack entries
  servers/         Listed server entries
  datapacks/       Datapack entries
  worlds/          World download entries
  governance/      Community governance data
  pack-overrides/  Config/resource override zips
  archived/        Removed items
/crash-signatures/ Regex-based crash triage signatures
/loader-manifests/ Pinned modloader hashes and domain allowlists
/.github/
  workflows/       CI/CD (nightly compiler, web build, desktop release, e2e)
  ISSUE_TEMPLATE/  Structured community forms
/compiler/         Python nightly compiler
/crates/
  agora-core/      Shared Rust business-logic library (no tauri/clap types)
  agora/           Standalone `agora` CLI binary
/desktop/          Tauri desktop application (React + Tailwind + Rust; crate `agora-desktop`)
/web/              Static Next.js public directory
/scripts/          Development helper utilities
AGENTS.md          Agent guide (canonical for AI sessions)
BACKLOG.md         Phase-by-phase task tracker
CODE_OF_ENGAGEMENT.md  Canonical review conduct rules
REGISTRY_CURATION_REFERENCE.md  Self-contained manifest-authoring reference
```

## Quick Start

### 1. Compile the registry database

```bash
cd compiler
python -m venv .venv
# .venv\Scripts\activate on Windows; source .venv/bin/activate on macOS/Linux
pip install -r requirements.txt
python compile.py --out ../registry.db            # uses ED25519_PRIVATE_KEY from .env if set
python compile.py --skip-sign --out ../registry.db # local dev without signing
python ../scripts/verify_db.py
```

The signed database is normally published as a GitHub Release Asset by `.github/workflows/compile.yml`. Locally, if the `ED25519_PRIVATE_KEY` env var is unset (or PyNaCl is not installed) and you omit `--skip-sign`, the compiler fails loudly with a non-zero exit code -- it will NOT silently produce an unsigned `registry.db.sig` placeholder. Use `--skip-sign` for local development.

### 2. Desktop app

```bash
cd desktop
npm install
npm run build      # builds the Vite frontend under dist/
# Rust toolchain required:
npm run tauri:dev  # or cargo tauri dev from src-tauri/
```

### 3. Web directory

```bash
cd web
npm install
npm run build      # static export to web/dist/
```

## Code of Engagement

All contributors and reviewers are bound by the canonical rules in [`CODE_OF_ENGAGEMENT.md`](./CODE_OF_ENGAGEMENT.md).

> **📜 Platform Code of Engagement**
>
> This platform is a curated asset repository, not a general discussion forum or social media feed. We built this ecosystem to keep modding open, high-quality, and hyper-focused.
>
> **Rules of Engagement (Zero Tolerance):**
> - Comments must strictly address the technical performance, stability, features, or usability of the mod or asset in question.
> - No memes, no off-topic banter, no update-begging ("1.21 when?"), no philosophical discussions.
> - No cultural, political, or social drama. Leave it at the door.
> - No aggression, entitlement, or personal attacks against mod creators, curators, each other, or anyone else.
> - Violations result in immediate and permanent removal from the registry's review system.
>
> If you want to socialize, share memes, or debate off-topic things, visit our community spaces instead:
> 🔗 Project Discord (for now I'm keeping this fairly restricted): https://discord.gg/56tpsa2sTZ

## Environment Setup

Copy the example environment file and fill in any values you need locally:

```bash
cp .env.example .env
```

See `.env.example` for the list of supported variables. Note: `.env` is loaded
at runtime by the Python compiler only; the Tauri desktop app does **not** read
`.env`.

### Environment variables for the Tauri build

The Tauri desktop app reads two values **at compile time** via Rust's
`option_env!` macro — they are embedded directly into the compiled binary.
This means they must be set as **real shell environment variables** in the
session that runs `npm run tauri:dev` (or the production build step). They
are **not** read from `.env` (which is loaded at runtime by the Python
compiler only, not by the Rust build).

For production GitHub Actions builds, set both as repository **Variables**
(not Secrets — neither value is sensitive) in
repo Settings → Secrets and variables → Actions → **Variables** tab:

| Variable | Purpose | Sensitive? | Example |
|---|---|---|---|
| `AGORA_OAUTH_CLIENT_ID` | GitHub OAuth App client ID for in-app sign-in (Device Flow) | ❌ Public | `Iv1.xxxxxxxxxxxxxxxx` |
| `AGORA_REGISTRY_PUBKEY` | Ed25519 public key (hex) for verifying downloaded `registry.db` signatures | ❌ Public | `47adee76cf587ee618f79eb2fa5bde003824d3bfc2dbb5080d33073c5a8f8c18` |

Without these, the desktop app fails fast with clear errors at the affected
feature (`ERR_AUTH_NOT_CONFIGURED` for OAuth, `ERR_REGISTRY_PUBKEY_NOT_CONFIGURED`
for signature verification) rather than silently misbehaving.

#### `AGORA_OAUTH_CLIENT_ID` — GitHub OAuth (in-app sign-in)

The desktop app's "Sign in with GitHub" button uses the OAuth Device Flow.
Register a GitHub App at <https://github.com/settings/developers> (Authorization
type: **GitHub App**, enable **Device Flow**), then grant these permissions on
the app's **Permissions** tab:

**Repository permissions:**
- **Contents** — Read-only (`GET /repos/{owner}/{repo}/releases` for mod
  install version resolution + registry release fetch)
- **Issues** — Read and write (covers issue reactions for voting, issue
  comments for reviews, and issue creation for crash reports / flag
  submission — Phase 5 governance)
  *(Metadata: Read-only is mandatory and always granted.)*

**Organization permissions:**
- **Members** — Read-only (org membership read for the Sybil/trust check,
  §3.1)

> **Note on scopes:** GitHub Apps ignore the `scope` parameter in the
> device-code request — permissions are determined solely by the app's
> settings in the GitHub UI. The Rust build does **not** send an OAuth-App
> scope string; configure everything via the app's Permissions tab above.

Then expose the app's Client ID (shown on the GitHub App's General tab —
the `Iv1.xxxxx` string; **this is public, not a secret**) in your shell:

```powershell
# PowerShell (one session)
$env:AGORA_OAUTH_CLIENT_ID = "Iv1.xxxxxxxxxxxxxxxx"
npm run tauri:dev
```

```bash
# bash/zsh (one session)
export AGORA_OAUTH_CLIENT_ID="Iv1.xxxxxxxxxxxxxxxx"
npm run tauri:dev
```

A Client Secret is **not** needed — Device Flow is specifically designed for
native apps that can't safely store a secret. If GitHub prompts for one,
generate-and-discard; never place it in this codebase.

#### `AGORA_REGISTRY_PUBKEY` — registry.db signature verification

Before trusting a downloaded `registry.db`, the desktop app verifies its
Ed25519 signature against a public key compiled into the binary. The matching
private key (`ED25519_PRIVATE_KEY`, a real secret) is held by the CI compiler
workflow only; the public key is needed on the desktop side.

If you don't yet have a keypair, generate one once (e.g. via `openssl` or the
`cryptography` Python package), store the private key in GitHub Actions
Secrets as `ED25519_PRIVATE_KEY`, and derive the public key:

```bash
# Derive the 32-byte Ed25519 public key (hex) from a 32-byte seed:
python -c "from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey; \
  from cryptography.hazmat.primitives import serialization; \
  seed = bytes.fromhex('YOUR_32_BYTE_PRIVATE_SEED_HEX'); \
  pub = Ed25519PrivateKey.from_private_bytes(seed).public_key(); \
  print('AGORA_REGISTRY_PUBKEY=' + pub.public_bytes(\
    encoding=serialization.Encoding.Raw, \
    format=serialization.PublicFormat.Raw).hex())"
```

Then set the resulting public key (without the `AGORA_REGISTRY_PUBKEY=`
prefix) in your shell before building:

```powershell
$env:AGORA_REGISTRY_PUBKEY = "47adee76cf587ee618f79eb2fa5bde003824d3bfc2dbb5080d33073c5a8f8c18"
npm run tauri:dev
```

In debug builds (`npm run tauri:dev`), an unset `AGORA_REGISTRY_PUBKEY` is
non-fatal: signature verification is skipped with a console warning, to
keep the local-dev loop smooth. In release builds (`npm run tauri:build`),
the app refuses to verify any registry without the key compiled in.

## Compiler `.env` variables

These are loaded at runtime by the Python compiler (NOT read by the Tauri Rust build):

| Variable | Purpose | Sensitive? |
|---|---|---|
| `ED25519_PRIVATE_KEY` | CI-only Ed25519 key used to sign `registry.db` | ✅ Secret |
| `GITHUB_TOKEN` | Standard GitHub token used by the compiler for issue-form / reaction / GraphQL calls | ✅ Secret |
| `DISCORD_WEBHOOK_URL` | Optional. Discord webhook for curator alerts on velocity circuit-breakers / coordinated-attack detection. When unset, Discord notifications are silently skipped (audit-trail + admin-alert GitHub issue still fire) | Optional |
| `AGORA_REGISTRY_REPO` | Registry and governance repository in `owner/repo` form. Falls back to GitHub Actions' `GITHUB_REPOSITORY` in compiler jobs; launcher builds must pass it explicitly. | ❌ Public |

`AGORA_OAUTH_CLIENT_ID` and `AGORA_REGISTRY_PUBKEY` are intentionally NOT loaded from `.env` -- they are read by the Tauri Rust build at compile time via `option_env!()`. See "Environment variables for the Tauri build" below.

## Releasing the Desktop App

Desktop releases are built by GitHub Actions via `.github/workflows/release-desktop.yml`. When a `v*` tag is pushed, the workflow builds native installers for Windows (`.msi` + `.exe`), macOS (`.dmg`), and Linux (`.AppImage` + `.deb`), then uploads them as assets on a GitHub Release.

### Prerequisites (one-time setup)

Add these as GitHub repository secrets (Settings → Secrets and variables → Actions):

| Secret | Description | Example |
|---|---|---|
| `AGORA_OAUTH_CLIENT_ID` | GitHub OAuth App client ID (same one used for local dev) | `Iv1.xxxxxxxxxxxxxxxx` |
| `AGORA_REGISTRY_PUBKEY` | Ed25519 public key (hex) matching the compiler's signing key. Public, so it can live as an Actions variable; the release workflow reads `vars.AGORA_REGISTRY_PUBKEY || secrets.AGORA_REGISTRY_PUBKEY` | `47adee76cf587e...` |

`AGORA_REGISTRY_REPO` is set automatically by the workflow from `github.repository`. `GITHUB_TOKEN` is provided automatically by Actions.

### Cutting a release

```bash
# 1. Update the version in desktop/src-tauri/tauri.conf.json (if not already bumped)
#    and desktop/package.json to match.

# 2. Commit and tag
git add -A
git commit -m "release: v0.1.0"
git tag v0.1.0
git push origin v0.1.0

# 3. The workflow runs automatically — builds 3 platforms, creates a DRAFT release
#    with all installers attached.

# 4. Review the draft release on GitHub, edit the release notes, then click "Publish."
```

You can also trigger a build manually via Actions → "Release Desktop App" → Run workflow (enter the tag name).

### What users download

Users go to the GitHub Releases page and download the file for their platform:
- **Windows**: `Agora_0.1.0_x64.msi` (installer) or `Agora_0.1.0_x64.exe` (standalone)
- **macOS**: `Agora_0.1.0_aarch64.dmg` (Apple Silicon) or `Agora_0.1.0_x64.dmg` (Intel)
- **Linux**: `Agora_0.1.0_amd64.AppImage` (portable) or `Agora_0.1.0_amd64.deb` (apt)

No Node.js, no npm — just a standard installer. The app is ~10–15 MB (Tauri uses the OS native webview, not a bundled Chromium).

### Registry vs Desktop release streams

| Tag pattern | Contents | Frequency |
|---|---|---|
| `registry-YYYY-MM-DD` | `registry.db`, `registry.db.sig`, `registry-web.json`, `registry-web.json.sig` | Nightly (automated by `compile.yml`) |
| `v0.1.0`, `v0.2.0`, ... | Desktop installers per platform | On-demand (when you cut a release) |

The two streams are independent. The desktop app fetches `registry.db` from the `registry-*` releases at runtime; the `v*` releases only ship the app binary.

## Governance Monitor and Tracked State (Work Package 7)

The compiler's governance pipeline (`compiler/governance.py`) detects anomalous voting patterns and manages quarantine state. It operates in three modes, all non-mutating with respect to GitHub:

| Mode | Reads GitHub | Writes state file | Discord alerts |
|---|---|---|---|
| `off` | No | No | No |
| `read-only` | Yes | No | No |
| `monitor` | Yes | Yes | Yes |

### Tracked state path

Production state is tracked at `registry/governance/governance-state.json` and committed to `master`. The compiler's generic default is `<output-dir>/governance-state.json`; production passes `--governance-state-in` and `--governance-state-out` explicitly. The public file records stable event IDs, affected GitHub reaction IDs, timestamps, and curator-visible status so the community can audit operational quarantine history.

### Production repo and policy

The governance repo defaults to `AGORA_GOVERNANCE_REPO` → `AGORA_REGISTRY_REPO` → `GITHUB_REPOSITORY`. Production uses `agora-mc/Agora-Launcher` with the `production` policy, which enforces a 30-day account-age threshold and a 6-hour, 5×-baseline raid window. Sandbox policy removes the age requirement and uses a 10-minute, 3-reaction threshold. State files carry `governance_repository` and `policy`; the loader rejects mismatched prior events with a warning, and production CI rejects the file before compilation.

### Monitor semantics and meaningful-only state commits

State is written only when `mode=monitor`. A state commit represents a meaningful change: a new anomaly event, an expanded anomaly (more reactions in the same window), or a resolved event (curator decision applied). Compiles that produce no state delta do not trigger a commit. This prevents spurious `governance-state.json` diffs on every nightly run.

### Curator decision status updates

Curators record decisions in `registry/governance/quarantine_decisions.json` (compiler never writes this file). Each decision maps an `event_id` to `accepted` or `rejected`. `accepted` lifts the quarantine so reactions count again; `rejected` permanently excludes those reactions. The compiler reads this file every run and persists the resulting event-status transition when monitor state changes meaningfully.

### Public operational and audit rationale

Every state event records `event_id`, `item_id`, `event_type`, `status`, `detected_at`, the exact `affected_reactions` IDs, and `details_json` with threshold, window, vote counts, and conflict users. This provides a transparent audit trail: anyone can verify which reactions were quarantined, why, and whether a curator decision resolved them. The audit log (`registry/governance/audit_log.json`) separately captures compile-level events.

### Why governance state is NOT a release asset

The signed database and web export are published as GitHub Release Assets because clients consume them. `governance-state.json` is compiler-only operational history: it stays in Git for transparent review and use by subsequent nightly runs, and is never uploaded as a release asset.

### Local PowerShell monitor command

```powershell
# WARNING: This changes the tracked production state and can send real alerts.
$env:GITHUB_TOKEN = (gh auth token)
$env:DISCORD_WEBHOOK_URL = "YOUR_PRODUCTION_WEBHOOK"

python compiler/compile.py `
  --governance-mode monitor `
  --governance-policy production `
  --governance-repo agora-mc/Agora-Launcher `
  --governance-state-in D:/Agora/registry/governance/governance-state.json `
  --governance-state-out D:/Agora/registry/governance/governance-state.json `
  --no-governance-write `
  --skip-sign `
  --out D:/Agora/registry.db
```

`--no-governance-write` suppresses the legacy audit-log append without disabling monitor state. Use `agora-mc/governance-sandbox-testing`, `--governance-policy sandbox`, a temporary state path, and a test-channel webhook for sandbox validation. Never point sandbox validation at the tracked production state.

### Read-only diagnostics (never writes state)

```powershell
$env:GITHUB_TOKEN = (gh auth token)
python compiler/compile.py `
  --governance-mode read-only `
  --governance-policy production `
  --governance-repo agora-mc/Agora-Launcher `
  --governance-state-in D:/Agora/registry/governance/governance-state.json `
  --skip-sign `
  --out D:/Agora/registry.db
```

`read-only` reads GitHub issues, reactions, and reviews, runs the full anomaly detection pipeline, and produces governance results in memory — but **never writes state to disk, never sends Discord alerts, and has no effect on the working tree or remote**. Use this for dry-run diagnostics.

### Malformed and stale state recovery

Production CI fails before compiling if the tracked state is missing, malformed, uses an unsupported schema, contains duplicate or incomplete events, or names the wrong repository or policy. Recover by restoring the last valid file from Git history, or, after curator review confirms that no pending event history must be retained, commit this valid empty envelope: `{"schema_version":1,"governance_repository":"agora-mc/Agora-Launcher","policy":"production","events":[]}`. Validate it with `python scripts/validate_governance_state.py registry/governance/governance-state.json` before rerunning the workflow. Do not truncate or silently regenerate malformed production state.

### Workflow ownership boundaries

- The loader-refresh workflow is the sole committer for the three tracked files under `loader-manifests/`; the nightly compile may regenerate them for compilation but never includes them in its governance commit.
- The nightly governance commit stages only `registry/governance/governance-state.json`. Compiled database and web artifacts are uploaded to the registry release, not committed to Git.
- `quarantine_decisions.json` is curated manually via PR; the compiler reads but never writes it.

### Production mode remains read-only

The production compiler workflow (`compile.yml`) currently uses `read-only` mode for governance (no state writing, no Discord alerts). **Full production monitor mode (state commits + Discord alerts) is not yet activated in CI** — it awaits completion of sandbox testing gates and manual curator sign-off. Until those gates are passed, production runs are observation-only.

## Agent Tooling

This repository includes Kilo agent configuration under `.kilo/`:

- `.kilo/kilo.json` — project-level model, permissions, MCP, and skill settings.
- `.kilo/agent/*.md` — agent profiles (`code`, `security`, `registry-curator`, `reviewer`).
- `.kilo/command/*.md` — slash commands: `/registry`, `/desktop`, `/web`, `/review`.
- `.kilo/skills/*/` — project-specific skills: `agora-architecture`, `tauri-security`, `registry-curation`.

See [`AGENTS.md`](./AGENTS.md) for the canonical guide to agent interactions and standards.
