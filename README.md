# Agora Minecraft Launcher

> This is not a warehouse. This is a boutique.

Agora is a decentralized, ad-free, open-source Minecraft launcher and discovery platform. Community-curated manifests are compiled into a signed registry, and each Minecraft setup lives in an isolated instance.

## Download and documentation

- [Desktop and CLI releases](https://github.com/agora-mc/Agora-Launcher/releases) - choose the newest published `v*` release, not a `registry-*` data release.
- [Website](https://agoramc.com/)
- [Player documentation](https://agoramc.com/docs/)
- [Troubleshooting and support](./docs/TROUBLESHOOTING.md)
- [CLI reference](./docs/CLI.md)
- [Documentation index](./docs/README.md)

Agora also includes a searchable **Help & Guide** tied to the current desktop interface.

## First run

1. Download a packaged release for your operating system.
2. Complete onboarding and review the optional service choices.
3. Synchronize the signed registry.
4. Let Agora discover or provision a compatible Java runtime.
5. Create a small disposable instance or import a supported pack.
6. Review health findings before launching.
7. Back up valuable worlds separately before major changes.

Delegated launch is the global default. It leaves Microsoft/Xbox authentication and JVM startup to the official Minecraft Launcher. Direct launch is optional, requires a Microsoft account connected inside Agora, and provides integrated process status and console output. Instances stored with `Auto` follow the global mode; the current desktop UI does not expose a per-instance override selector.

## Key capabilities

- Curated and optional Modrinth discovery with instance-aware compatibility labels.
- Dependency-aware install plans with one review before files change.
- Loader compatibility evidence, recommended signed versions, compatible alternatives, and manual candidates when a capability cannot be verified.
- Health checks, Crash Doctor, snapshots, Last Known Good recovery, loadouts, and lockfiles.
- Imports from supported packs and launcher profiles, plus `.mrpack` and Agora pack export.
- A standalone CLI, optional integrated GitHub Copilot assistant, and authenticated local MCP automation.

## Safety and recovery boundaries

- Automatic pre-launch recovery protects mod and configuration state but intentionally excludes `saves/`.
- Full manual and transactional snapshots use a broader tracked scope that includes `saves/`.
- Last Known Good promotes the exact pre-launch snapshot only after a successful session of at least 60 seconds.
- Loadouts remember enabled state; lockfiles describe reproducible artifacts.
- None of these should be the only backup for an irreplaceable world.

Agora makes no automated analytics calls. Functional features can still contact their documented services. The individual Privacy endpoint switches are enforced by the backend; the current Lockdown toggle does not yet enforce a global network block, so disable each endpoint individually before relying on an offline test.

## Project principles

- **$0/month server footprint** - GitHub, GitHub Release Assets, and static hosting carry the public distribution load.
- **Secure defaults** - delegated launch is the default; direct launch is explicit and optional.
- **Curated, not warehoused** - boutique quality and community review take priority over inventory size.
- **Decentralized governance** - votes, reviews, and triage remain inspectable GitHub interactions.
- **Modrinth independence** - primary curated artifacts use pinned sources; Modrinth is optional.

## Contributor quick start

Build and test the Rust workspace components:

```bash
cargo fmt --all -- --check
cargo test -p agora-core --lib
cargo test -p agora-cli
```

Compile an unsigned local registry before the static website build:

```bash
cd compiler
python -m venv .venv
# .venv\Scripts\activate on Windows; source .venv/bin/activate on macOS/Linux
pip install -r requirements.txt
python compile.py --skip-sign --no-governance-write --governance-mode off --out ../registry.db
python ../scripts/verify_db.py ../registry.db
```

Build the desktop and website frontends from the repository root:

```bash
cd desktop
npm install
npm run build

cd ../web
npm install
NEXT_PUBLIC_GITHUB_REPOSITORY=agora-mc/Agora-Launcher npm run build
```

See [`docs/DEVELOPMENT.md`](./docs/DEVELOPMENT.md) for prerequisites, environment boundaries, disposable profiles, and the full validation matrix.

## Maintainer references

- [Development](./docs/DEVELOPMENT.md)
- [Releasing](./docs/RELEASING.md)
- [Governance operations](./docs/GOVERNANCE_OPERATIONS.md)
- [Registry curation](./REGISTRY_CURATION_REFERENCE.md)
- [Architecture ownership](./docs/architecture/layer-ownership.md)
- [Agent guide](./AGENTS.md)

Detailed compiler configuration, release mechanics, governance incident recovery, and agent-tool configuration live in those canonical references rather than this player-facing overview.

## License and community

Agora workspace packages declare [GPL-3.0-only](LICENSE).

Join the [project Discord](https://discord.gg/56tpsa2sTZ) for community support. Contributions and reviews follow the project [Code of Engagement](./CODE_OF_ENGAGEMENT.md): keep discussion technical, focused, respectful, and relevant to the asset or change under review.
