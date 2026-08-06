# Nightly Compiler

The nightly compiler reads the flat JSON manifests under `registry/` and `crash-signatures/` and compiles them into a signed SQLite database (`registry.db`) plus its Ed25519 signature (`registry.db.sig`).

## Local development

Create a virtual environment and install dependencies:

```bash
cd compiler
python -m venv .venv
source .venv/bin/activate  # .venv\Scripts\activate on Windows
pip install -r requirements.txt
```

Build the database:

```bash
python compile.py --skip-sign --no-governance-write --governance-mode off --out ../registry.db
```

Use `--skip-sign` only for local development. A signed build requires the `ED25519_PRIVATE_KEY` environment variable and signing dependencies; if signing was requested but cannot complete, the compiler exits non-zero rather than emitting an empty signature placeholder. Never copy the private key into documentation, source, or shell transcripts.

## CI

`.github/workflows/compile.yml` runs the compiler every night at 02:00 UTC and on `workflow_dispatch`. It verifies the database and governance state, uploads the signed database and web export as workflow artifacts, and deploys them to the current date-tagged registry release.
