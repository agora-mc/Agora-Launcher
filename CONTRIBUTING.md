# Contributing to Agora

This file exists so GitHub's "Contribute" prompts point somewhere. Everything below
is a pointer; the guidance itself lives in the documents linked here, and those stay
the source of truth.

## Before you start

- Read the [Code of Engagement](./CODE_OF_ENGAGEMENT.md). It governs review conduct on
  every issue, pull request, and catalog review.
- Read [AGENTS.md](./AGENTS.md) for the project's mission, directory map, and security
  defaults, and [CLAUDE.md](./CLAUDE.md) for the validation gates and architecture
  boundaries a change has to satisfy.

## Reporting something

Open an issue with the matching template under
[`.github/ISSUE_TEMPLATE/`](./.github/ISSUE_TEMPLATE/): bug reports, mod submissions,
and review forms each have one. For troubleshooting a build you are running, start
with [docs/TROUBLESHOOTING.md](./docs/TROUBLESHOOTING.md) and
[docs/SUPPORT.md](./docs/SUPPORT.md).

## Changing code

[docs/DEVELOPMENT.md](./docs/DEVELOPMENT.md) has the prerequisites, per-component
build commands, disposable-profile setup, and the full validation matrix.
[CLAUDE.md](./CLAUDE.md) lists the same gates in the order CI runs them. Run the ones
covering what you touched before opening a pull request:

```bash
python scripts/check_architecture.py && python scripts/check_docs.py && python scripts/check_tauri_bindings.py --check
```

Business logic belongs in `agora-core`; the desktop, CLI, and MCP hosts are adapters
over it. [docs/architecture/layer-ownership.md](./docs/architecture/layer-ownership.md)
says which layer owns a given behavior.

Pull requests are gated by the `CI Enforcement`, `Launch Planner CI`, and `E2E Tests`
workflows. The same three workflows gate release builds — see
[docs/RELEASING.md](./docs/RELEASING.md).

## Adding catalog entries

Catalog manifests are authored by hand and reviewed like code. The self-contained
reference is [REGISTRY_CURATION_REFERENCE.md](./REGISTRY_CURATION_REFERENCE.md);
governance mechanics are in
[docs/GOVERNANCE_OPERATIONS.md](./docs/GOVERNANCE_OPERATIONS.md).

## Art and writing

The README says plainly which parts of Agora were built with AI assistance.
Human-made art and prose are welcome; see the AI disclaimer in
[README.md](./README.md).

## License

Agora's workspace packages declare [GPL-3.0-only](./LICENSE). Contributions are
accepted under that license.
