# Developing Agora

This page covers local builds and validation. Player instructions belong in the in-app guide and website documentation.

## Repository map

| Path | Purpose |
| --- | --- |
| `crates/agora-core/` | Shared business logic |
| `crates/agora/` | Standalone CLI |
| `desktop/` | Tauri desktop application and React UI |
| `web/` | Public static directory |
| `compiler/` | Registry compiler |
| `registry/` | Curated source manifests and governance data |
| `loader-manifests/` | Pinned loader catalog inputs |
| `scripts/` | Validation and maintenance helpers |
| `docs/` | User, developer, release, and operator reference |

Keep reusable behavior in `agora-core`. Desktop, CLI, and MCP hosts should adapt the same services rather than implement parallel business rules.

## Prerequisites

- current stable Rust toolchain;
- Node.js and npm supported by the lockfile;
- Python supported by the compiler requirements;
- platform prerequisites required by Tauri;
- Git.

Use the repository lockfiles. Avoid replacing them merely to satisfy a local global-tool mismatch.

## Build the CLI

```bash
cargo build -p agora-cli
cargo test -p agora-cli
```

See [CLI.md](./CLI.md) for usage.

## Build the desktop application

```bash
cd desktop
npm install
npm run build
npm run tauri:dev
```

A production installer must be tested as a packaged release. Development mode does not prove that compile-time variables, updater metadata, resources, signing, or platform bundles are correct.

## Build the website

Generate the root `registry-web.json` through the local compiler procedure below before building. Static export needs registry entries to generate the dynamic catalog routes.

```powershell
cd web
npm install
$env:NEXT_PUBLIC_GITHUB_REPOSITORY = "agora-mc/Agora-Launcher"
npm run build
```

On bash-compatible shells, export the same variable before `npm run build`. The site module intentionally rejects an unset repository because release, issue, and source links cannot be constructed safely without it.

The website should remain useful without the desktop application installed. It owns pre-install guidance and shareable documentation, not the complete in-app learning curriculum.

## Compile the registry locally

```bash
cd compiler
python -m venv .venv
```

Activate the environment, then:

```bash
pip install -r requirements.txt
python compile.py --skip-sign --no-governance-write --governance-mode off --out ../registry.db
python ../scripts/verify_db.py
```

Use `--skip-sign` only for local development. Production clients must not trust unsigned registry output.

## Environment boundaries

### Compiler runtime environment

The Python compiler may load `.env` for local development. Copy `.env.example` to `.env` only when local compiler work needs it, and consult `.env.example` as the canonical variable list.

Secret examples include:

- the Ed25519 private signing key;
- GitHub tokens;
- Discord webhook URLs.

Never commit real values.

### Desktop compile-time configuration

The Rust desktop build embeds selected public configuration at compile time. A `.env` file used by the Python compiler is not automatically a Tauri build environment. Set these as shell variables for local Tauri builds and as repository Actions variables for releases:

| Variable | Purpose | Secret? |
| --- | --- | --- |
| `AGORA_OAUTH_CLIENT_ID` | Public GitHub application client ID used by the governance device flow | No |
| `AGORA_REGISTRY_PUBKEY` | Public Ed25519 key used to verify downloaded registry signatures | No |
| `AGORA_REGISTRY_REPO` | Registry repository in `owner/repo` form | No |
| `VITE_AGORA_REPOSITORY` | Repository used by desktop frontend links | No |

The matching `ED25519_PRIVATE_KEY`, GitHub tokens, updater signing key, and webhook URLs are secrets. They belong only in the intended protected environment and must never be copied into documentation or screenshots. GitHub device flow does not require embedding a client secret in the native app.

Release workflows must verify the packaged executable can:

- authenticate through intended public client configuration;
- verify the signed registry;
- synchronize the registry;
- populate Browse;
- load loader/runtime catalog data.

Do not assume a successful source build proves packaged configuration is present.

## Core validation

Before a substantial pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib
cargo test -p agora-cli
cargo check -p agora-desktop
```

Also run the frontend, compiler, generator, and end-to-end tests relevant to the changed area.

## Documentation validation

For a user-visible change:

1. update `desktop/src/data/guideContent.ts`;
2. update CLI help and `docs/CLI.md` when applicable;
3. update website docs when first-run behavior changes;
4. search for obsolete labels and defaults;
5. test all newly documented commands against a temporary data root;
6. verify links;
7. avoid copying secret-bearing operator commands into player documentation.

Run the hermetic documentation gate:

```bash
python scripts/check_docs.py
```

It checks current internal Markdown links, website routes, guide-topic mappings, product spelling, and known stale claims without making network requests or rewriting files. CLI help-contract checks run under `cargo test -p agora-cli`.

### Documentation screenshots

Documentation screenshots are generated from the current React UI with fixed synthetic Tauri responses. The fixture contains no personal profile, credentials, tokens, private paths, or private packs:

```bash
cd desktop
npx playwright test e2e/docs-screenshots.spec.ts --project=chromium --workers=1
```

Review every generated image under `web/public/screenshots/` before publishing. The browser fixture verifies interface copy and layout; use a packaged build with a disposable profile for release smoke testing.

## Development data

Prefer a separate data directory for experiments:

```bash
agora --data-dir ./tmp/agora-dev paths
```

Use disposable instances whose purpose is obvious from their name. Never run destructive tests on a real world or the normal user profile.

For a disposable desktop profile, set `AGORA_DATA_DIR` before starting Tauri:

```powershell
$env:AGORA_DATA_DIR = Join-Path $env:TEMP "agora-desktop-docs"
cd desktop
npm run tauri:dev
```

Close Agora before removing that temporary directory. Microsoft credentials use the operating-system credential store and are not isolated by `AGORA_DATA_DIR`; do not sign in or sign out with a personal account during disposable-profile tests.

## Pull-request evidence

Include the smallest relevant evidence:

- tests run;
- packaged or development build used;
- screenshots for UI changes;
- CLI help/output for CLI changes;
- migration or rollback behavior for persistent-state changes;
- timing methodology for performance changes;
- documentation files reviewed.
