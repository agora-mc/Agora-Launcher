# Spike 2: write isolation

Answers MASTER_SPEC §26.5's question before Phase 3 fixes the layer contract: can a game, or a tool it
starts, change a shared file (a mod in the content store, a pinned base, or through a hardlink the
store install) while it runs? Results and their consequences are in [`FINDINGS.md`](FINDINGS.md).
These are measurement tools, not product code; nothing in Agora calls them.

| Folder or script | What it is |
|---|---|
| `harness/` | `spike2`, the test driver (Rust). `run` scores usvfs, `agvfs` scores the prototype, `links` scores hardlink deployment, `launch` and `launch-agvfs` start a real game under either VFS. |
| `agvfs/` | The copy-on-write VFS prototype: a DLL injected into the game that hooks the NT file calls. |
| `run_matrix.sh`, `run_links.sh` | The 62-operation write matrix (ten ways to change a file, from three lower layers, in the game and in a child process) under agvfs and under link deployment, each with and without ACL-protected lower files. |
| `compat.py` | Launches installed games under agvfs with the store install as the read-only lower layer, and checks the install is byte-for-byte unchanged afterwards. |
| `analyze.py`, `compare_opened.py` | Read a usvfs debug log (what a game opened, and how), and compare it with an agvfs `verbose=1` log. |

Build with `cargo build --release` in `harness/` and `agvfs/` (each is its own workspace, outside
Agora's). Run the scripts from Git Bash. Sandboxes go to `%LOCALAPPDATA%\AgoraSpike\write-isolation`,
or `SPIKE2_WORK`. The usvfs modes need MO2's usvfs release (`usvfs_v0.5.7.2.7z`, checked against the
SHA-256 GitHub publishes) unpacked somewhere, passed as `--usvfs <bin folder>`; it is not in this
repository. Real-game modes start real games: run them only on installs you are happy to test with.
