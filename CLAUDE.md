# CLAUDE.md

Operational guide for Claude Code in this repo. Project mission, directory map, governance
pipeline, and security defaults live in @AGENTS.md — read that first; this file covers *how to
work* here (gates, boundaries, gotchas) without repeating it.

## Sources of truth

| Question | Read |
|---|---|
| Mission, conventions, governance, env vars | `AGENTS.md` |
| Engineering blueprint (§0–§18 design; §19 supersedes on conflict) | `.kilo/plans/MASTER_SPEC.md` (read-only for agents) |
| Which layer owns a behavior | `docs/architecture/layer-ownership.md` |
| Build/validate/release procedure | `docs/DEVELOPMENT.md` |
| Interactive-experiences feature state | `docs/interactive/IMPLEMENTATION_STATUS.md` |

Prefer the smallest change that satisfies the request. No drive-by refactoring.

## Validation gates

These mirror `.github/workflows/ci-enforcement.yml`. Run the ones covering what you touched;
run all before calling a substantial change done.

Repo root, hermetic and fast (seconds — run these liberally):

```bash
python scripts/check_architecture.py && python scripts/check_docs.py && python scripts/check_tauri_bindings.py --check
```

Rust (`crates/agora-core`, `crates/agora` → package `agora-cli`, `desktop/src-tauri` → `agora-desktop`):

```bash
cargo fmt --all --check && cargo clippy -p agora-core -p agora-cli --all-targets --all-features -- -D warnings && cargo test -p agora-core --lib --tests && cargo test -p agora-cli && cargo check -p agora-desktop
```

Desktop frontend (from `desktop/`):

```bash
npm run build && npm run test:unit
```

`npm run build` = `check:boundaries` + `tsc` + `vite build`. E2E (`npx playwright test`, ~243
tests) is slow — run when asked or when touching UI flows.

Web (from `web/`) needs `registry-web.json` generated first and the repo variable set:

```bash
NEXT_PUBLIC_GITHUB_REPOSITORY=agora-mc/Agora-Launcher npm run build
```

Compiler (from `compiler/`, needs a venv + `pip install -r requirements.txt`):

```bash
python compile.py --skip-sign --no-governance-write --governance-mode off --out ../registry.db && python ../scripts/verify_db.py
```

Per `AGENTS.md`: after registry/loader/crash-signature edits run the registry gate; after
`desktop/` edits run the desktop build; after `web/` edits run the web build.

### Gate gotchas

- **`cargo fmt --all --check` exits 1 on failure but prints diffs to stdout.** Piping to `tail`
  swallows the status — check `${PIPESTATUS[0]}`, or just run it bare.
- **CI's `frontend` job runs `npx tsc --noEmit` + `npx vite build` directly, skipping
  `check:boundaries`.** The interactive import-boundary check is enforced *locally* by
  `npm run build`. Run it explicitly; CI will not catch a violation for you.
- `scripts/check_docs.py` and `check_architecture.py` are hermetic — no network, no file writes.
- The compiler has **no `.venv` checked out** in this working copy; create one before registry work.
- **Local Rust is 1.96.0; CI uses `dtolnay/rust-toolchain@stable`, which is now 1.99.x.** Clippy
  runs with `-D warnings`, so lints added after 1.96 fail CI while passing locally. A clean local
  clippy is not proof CI is clean — `rustup update` before trusting it on lint-sensitive work.

## Architecture boundaries (enforced, not advisory)

All three frontends (Tauri GUI, CLI, MCP) call the same `agora-core`. Business logic lives in
core; adapters own only transport and OS mechanism.

- `agora-core` must not depend on `tauri`, `clap`, or MCP protocol types — enforced by
  `scripts/check_architecture.py`.
- Need a platform primitive? Define a **trait in core**, implement it in the adapter.
- React owns UI state and returns user decisions via callback; it never executes business
  operations and must never call MCP HTTP (`127.0.0.1:39741`) directly — always `invoke()`.
- SQL is core-only, parameterized, via `tauri-plugin-sql`.
- Never `dangerouslySetInnerHTML` on community content.

### Interactive feature (`desktop/src/features/interactive/`)

Has its own stricter allowlist boundary on top of the above, enforced by
`desktop/scripts/check-interactive-boundaries.mjs` (fail-closed, TS-resolver based):

- `domain/`, `visual/`, `lab/` must not import `@tauri-apps/*`, `@/lib/tauri`, `live/`, or
  operation components. `live/` is the only app-boundary layer.
- Within `live/`: `read` (readAdapters, liveScene, freshness) may call only the read-command
  allowlist; `core` may use tauri *types* only; `operationBridges/` may host Standard
  controllers but not invoke tauri. Unclassified files fail.
- Shared visuals are controlled components emitting `VisualIntent` only — no operation-shaped
  callback props (checked via AST, both property and method signatures).

Negative fixtures live in `desktop/scripts/boundary-fixtures/`; every one must produce a
violation:

```bash
node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
```

## Environment

Windows 11, PowerShell primary (a Bash tool is also available — each takes its own syntax).
Rust 1.96, Node 25 (CI uses 24), Python 3.12, `gh` authenticated.

- Use a disposable data root for experiments: `AGORA_DATA_DIR` for Tauri, `--data-dir` for CLI.
  Microsoft credentials use the OS credential store and are **not** isolated by `AGORA_DATA_DIR` —
  do not sign in or out with a real account during disposable-profile tests.
- Never run destructive tests against a real instance or world.
- `.env` is for the Python compiler only; it is not a Tauri build environment. Desktop
  compile-time vars (`AGORA_OAUTH_CLIENT_ID`, `AGORA_REGISTRY_PUBKEY`, `AGORA_REGISTRY_REPO`,
  `VITE_AGORA_REPOSITORY`) are public and set as shell/Actions variables.
- Secrets (`ED25519_PRIVATE_KEY`, tokens, webhook URLs, updater key) never enter source,
  manifests, docs, or screenshots.

## Working-copy notes

- `web/public/screenshots/*.png` show as modified because they are **generated** deterministic
  doc assets (`desktop/e2e/docs-screenshots.spec.ts`). Byte drift from a re-render is expected —
  keep them, don't discard.
- Do not modify `.lock` files or history under `registry/archived/`.
- The `loader-manifests/` directory is owned solely by the loader-refresh workflow.

## Multi-agent convention

`docs/interactive/` documents a review workflow with named roles — Sol (architecture/safety
gates), DeepSeek (implementation), Terra (UX), Luna (smoke/regression). Findings are numbered
and tracked to closure in `ARCHITECTURE_REVIEW_SOL.md` / `UX_FINDINGS_TERRA.md` /
`IMPLEMENTATION_STATUS.md`. When working in that feature, append to the phase/fix logs rather
than rewriting them, and don't mark a gate passed that the corresponding reviewer hasn't run.
