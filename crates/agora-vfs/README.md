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
   `agora_vfs.dll` into the child before resumption. A child that cannot be injected is ended and the
   start fails; it never runs unprotected. A child of the other bitness cannot be injected from here
   (its import table and its DLL load belong to a process of its own bitness), so the DLL of the
   child's bitness is handed to `agora-vfs-helper.exe` of that bitness: it is started through the
   original `CreateProcessInternalW` (so this hook does not try to inject it), waited for, and
   ended after 15 seconds. The helper never resumes the child.
8. **Aliases (configuration version 2).** Each alias folder shows exactly the view of `mount`. A path
   under an alias resolves to the same mount-relative path, so a tool that reads the real install
   folder (Nemesis reads it from the registry, BodySlide hard-codes it) sees the mod files at its own
   path. The view is opaque: a real file in the install that is in no layer is **not found**, and is
   absent from listings. Writes copy up into `upper` as through the mount, and nothing under the real
   folder is opened for write, created, renamed or deleted by a hooked process.
9. **Reverse names.** A handle reports the folder it was opened through (the mount, or an alias) plus its
   relative path, whichever layer it really holds. The hooks are `NtQueryInformationFile` (classes 9,
   48 and the name part of 18) and `NtQueryObject` (class 1), so `GetFinalPathNameByHandleW` (every flag
   in use), `GetFileInformationByHandleEx(FileNameInfo)` and the NT name queries agree. Untracked
   handles pass through. The short-buffer answers were measured on Windows 11 (NTFS) against unhooked
   calls on a real file, for every buffer length from 0 to 260, and are matched (see
   `tests/alias.rs`).
10. **Start paths.** A process start (`CreateProcessInternalW`) translates the application name, the first
    token of the command line and the current directory from a name under the view to the real path the
    view resolves them to (for the folder itself, the mount folder). The kernel opens the image itself,
    so an image that exists only in a lower can start. A name under the view that is in no layer, or
    whited out, is refused with `ERROR_FILE_NOT_FOUND` (`ERROR_PATH_NOT_FOUND` for the directory).

### Path forms

For the mount and each alias, the DLL computes at start-up every NT name that names the same folder,
and matches any of them, case-insensitively:

- the drive form `\??\X:\...` (Win32 `\\?\` and `\\.\` paths arrive in this form);
- the volume GUID form `\??\Volume{GUID}\...`, from `GetVolumeNameForVolumeMountPointW`;
- the device form `\Device\HarddiskVolumeN\...` and `\??\GLOBALROOT\Device\HarddiskVolumeN\...`,
  from `QueryDosDeviceW`;
- the 8.3 form of the folder, from `GetShortPathNameW`, when it differs, combined with each of the above.

**Known gap:** 8.3 components *below* the mount or alias folder are not recognised. A name such as
`...\Skyrim Special Edition\DATA~1\x.esp` is not in the view. Only the folder's own 8.3 form is.

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

- `version` (`u32`): Configuration schema version: `1`, or `2` when there are `aliases` or a `dll_x86`.
  Any other version is refused. A version 1 configuration with `aliases` or `dll_x86` is refused too: a
  reader that ignored them would show a tool a vanilla game, or end its 32-bit children. Core writes
  version 2 only when there are aliases or a 32-bit DLL.
- `aliases` (`array of strings`, version 2, optional): Folders that show exactly the view of `mount`
  (see point 8 above). Each is normalized as the others are. Refused when empty, or when it is equal to,
  inside, or containing the mount, the upper, any lower, or another alias. The comparison ignores case.
- `mount` (`string`): The path the game sees as its root/mount folder. Trailing backslashes are stripped.
- `upper` (`string`): The instance's writable directory (layer 4) where copy-up files and whiteouts are stored.
- `lowers` (`array of strings`): Lower layer folders in priority order (highest priority first). In link deployments, `lowers[0]` is typically identical to `mount`.
- `dll` (`string`): Absolute path to `agora_vfs.dll` (64-bit) used to inject 64-bit child processes.
- `dll_x86` (`string`, version 2, optional): Absolute path to the 32-bit `agora_vfs.dll`, used for 32-bit
  children of either bitness of parent. Its helper, `agora-vfs-helper.exe`, sits beside it; the 64-bit
  helper sits beside `dll`. A 32-bit child with no `dll_x86`, or whose DLL or helper is missing, is ended
  and its reason names the path that was looked for.
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
- `tests/bitness.rs`: a 64-bit parent with a 32-bit child (through the 32-bit helper), a 32-bit parent
  with a 64-bit child (through the 64-bit helper), a missing 32-bit DLL (the child is ended and the
  reason names the path), and a helper that hangs (`agora-vfs-sleeper`, a bin that
  sleeps, copied beside the DLL as the helper; the child is ended after the timeout). Needs both builds:
  `cargo build -p agora-vfs` and `cargo build -p agora-vfs --target i686-pc-windows-msvc`.
- The write matrix and the alias tests run again for the 32-bit probe (`test_matrix_x86_*`,
  `*_32_bit`), with the probe built for i686.
- `tests/alias.rs`: an alias over a temp install, with a lower-only program started by its alias path.
  The probe (`src/bin/probe.rs`, `src/bin/probe/alias.rs`) runs the checks under the DLL. The name
  queries (`tests/support/names.rs`) are run on the probe's handles and, unhooked, on the real files;
  every answer must be equal for every buffer length from 0 to 260. The temp install's tree must be
  byte-for-byte unchanged at the end.
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
