# CLAUDE.md

@AGENTS.md

Claude-specific notes. Everything else is linked from `AGENTS.md`; read the linked page when the
task touches it rather than loading all of it up front.

## Before calling work done

Run the gates for what you touched — they are listed, with their gotchas, in
[`docs/DEVELOPMENT.md` → Core validation](docs/DEVELOPMENT.md#core-validation). The hermetic
ones take seconds:

```bash
python scripts/check_architecture.py && python scripts/check_docs.py && python scripts/check_tauri_bindings.py --check
```

## Boundaries that are enforced by scripts

- `agora-core` is game-agnostic and never references a game package; Minecraft lives in
  `agora-game-minecraft`, whose use of core may only shrink (`scripts/game_package_core_budget.json`).
- `agora-core` does not depend on `tauri`, `clap` or MCP protocol types. A platform primitive is a
  trait in core, implemented in the adapter.
- `desktop/src/features/interactive/` has a stricter import boundary; see
  [layer-ownership](docs/architecture/layer-ownership.md#interactive-feature-boundary-desktopsrcfeaturesinteractive).

## Environment

The user's machine is Windows 11 with PowerShell; cloud sessions run Linux. Use a disposable data
root for experiments (`AGORA_DATA_DIR`, or `--data-dir` for the CLI). Microsoft credentials live in
the OS credential store and are not isolated by it, so do not sign in or out with a real account
while testing.
