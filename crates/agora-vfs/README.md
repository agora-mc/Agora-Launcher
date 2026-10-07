# agora-vfs

`agora-vfs` is Agora's copy-on-write virtual file system for Windows game isolation.
It compiles to `agora_vfs.dll`, which is injected into a target game process (and child processes
spawned by the game) to ensure that shared base and content files are never overwritten, deleted,
or modified in place.

## How it gets into a process

The launcher creates the game suspended and then rewrites its import table in memory so
`agora_vfs.dll` is its first import (`crates/agora-vfs-inject`, which uses Microsoft Detours'
`DetourUpdateProcessWithDll`). When the process is resumed, Windows' own loader loads the DLL before
the game's other DLLs, so the hooks exist before any of them runs code. The DLL therefore exports a
function at **ordinal 1** (`agora_vfs_ordinal1`, forced by `build.rs`); Detours imports it by number.
The DLL's `CreateProcessInternalW` hook does the same to every process the game starts.

On attach, before anything else, the DLL calls Detours' `DetourRestoreAfterWith`, which puts back the
headers and import table the injection rewrote. Without it the game's own code sees a patched image:
real Steam Skyrim (SteamStub reads its headers) hung at start-up at about 20 MB. With it, Skyrim
launched through SKSE's loader ran to `kDataLoaded` with all 16 plugins under import-table
injection, and Engine Fixes' early `d3dx9_42.log` write landed in the writable layer.

If the import table cannot be rewritten, a remote `LoadLibraryW` thread is the fallback. It loads the
DLL only after the program's static imports have run their `DllMain`s, so it is a fallback and the
log says when it was used. A process of another architecture is refused before anything is written.
`AGORA_VFS_INJECTION=remote-thread` skips the import table and uses the fallback.

## How it works

When injected into a process:
1. It reads its configuration from the file path pointed to by the `AGORA_VFS_CONFIG` environment variable.
2. It installs NT API hooks (`NtCreateFile`, `NtOpenFile`, `NtSetInformationFile`, `NtQueryAttributesFile`,
   `NtQueryFullAttributesFile`, `NtQueryInformationByName`, `NtQueryDirectoryFile`, `NtQueryDirectoryFileEx`,
   `NtClose`, and `CreateProcessInternalW`).
3. If all hooks install successfully, it opens and signals the named Win32 event specified in `ready_event`
   so the launcher knows the process is protected. (With import-table injection the launcher has already
   resumed the process, which is how the DLL gets to load; it waits for the event or for the process to
   end, and kills the process on a timeout. With the fallback it resumes only after the event.)
   If `AGORA_VFS_CONFIG` names a configuration that cannot be loaded, or the hooks cannot be installed,
   the DLL ends its own process (exit code `0xA6F50001`) rather than let it run unprotected. With no
   `AGORA_VFS_CONFIG` at all it does nothing. Before it ends a process it logs one line,
   `[pid] ending the process <exe>: <why>` (to `AGORA_VFS_LOG` when its own configuration, and so its
   log path, could not be read), so a program the game started that disappears can be named.
4. Any attempt to write or truncate a file located in a lower layer copies the file up to the `upper`
   directory first (or skips copy for full file rewrites).
5. Any delete of a lower layer records a whiteout marker (`<upper>\.agvfs-wh\<rel>.wh`).
6. Folder directory listings merge the upper layer over lower layers, with whiteout markers removed.
7. Any calls creating child processes (`CreateProcessInternalW`) intercept process creation and inject
   `agora_vfs.dll` into the child before resumption.

## Configuration Format

Configuration is loaded as JSON from the path in `AGORA_VFS_CONFIG`:

```json
{
  "version": 1,
  "mount": "D:\\Games\\Instance\\game",
  "upper": "C:\\Users\\User\\AppData\\Local\\Agora\\instances\\inst1\\writable",
  "lowers": [
    "D:\\Games\\Instance\\game"
  ],
  "dll": "C:\\Program Files\\Agora\\agora_vfs.dll",
  "log": "C:\\Users\\User\\AppData\\Local\\Agora\\instances\\inst1\\vfs.log",
  "verbose": false,
  "ready_event": "Local\\agora-vfs-12345"
}
```

### Fields

- `version` (`u32`): Configuration schema version (must be `1`).
- `mount` (`string`): The path the game sees as its root/mount folder. Trailing backslashes are stripped.
- `upper` (`string`): The instance's writable directory (layer 4) where copy-up files and whiteouts are stored.
- `lowers` (`array of strings`): Lower layer folders in priority order (highest priority first). In link deployments, `lowers[0]` is typically identical to `mount`.
- `dll` (`string`): Absolute path to `agora_vfs.dll` used to inject child processes.
- `log` (`string`, optional): File path for logging VFS operations and diagnostics.
- `verbose` (`bool`, default `false`): If true, logs every unique file opened through the mount.
- `ready_event` (`string`, optional): Name of the Win32 event object (`OpenEventW` with `EVENT_MODIFY_STATE`) signaled once all hooks are active.

If `AGORA_VFS_CONFIG` is not set the DLL installs no hooks and does not signal `ready_event`. If it is set but the configuration cannot be read or is invalid, the DLL ends the process (see above).

## Tests

All of these start real processes and are `#[ignore]`d. Build the DLL and the fixture first:

```
cargo build -p agora-vfs -p agora-vfs-early-import
cargo test -p agora-vfs -- --ignored                      # the write matrix and tests/early_import.rs
$env:AGORA_VFS_DLL = "$PWD\target\debug\agora_vfs.dll"   # PowerShell; the core tests find the DLL here
cargo test -p agora-core --test game_deploy -- --ignored  # real injection, including the ordering test
```

- `tests/matrix.rs`: the six-topology write matrix.
- `tests/early_import.rs`: a DLL that cannot do its job ends the process before it runs; no
  configuration installs nothing.
- `crates/agora-vfs/fixtures/early-import` is the fixture: `agora-early-import-exe.exe` statically
  imports `agora_early_import.dll` (a `raw-dylib` import, so there is no import library and no build
  ordering), whose `DllMain` rewrites `early_<program>.txt` beside it. With argument `spawn` the
  program starts `Child.exe`, another copy, as SKSE's loader starts the game. The tests find both next
  to `agora_vfs.dll` in the target folder (`agora_vfs.dll`'s folder, for the core tests).
- `real_injection_loads_the_vfs_before_the_games_own_imports` (`crates/agora-core/tests/game_deploy.rs`)
  deploys those files as a mod's links and runs the program under the VFS: both `DllMain` writes must
  land in the writable layer and leave the content store unchanged. It fails with
  `AGORA_VFS_INJECTION=remote-thread`, the old behaviour.
- `real_injection_leaves_the_games_headers_as_they_were_on_disk`: the fixture run with `headers`
  compares its in-memory import directory, bound-import directory and section headers with its
  file; it fails if the DLL's restore call is removed.
- `real_injection_a_32_bit_game_is_refused_cleanly` uses `C:\Windows\SysWOW64\cmd.exe`.
