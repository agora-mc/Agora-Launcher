# agora-vfs

`agora-vfs` is Agora's copy-on-write virtual file system for Windows game isolation.
It compiles to `agora_vfs.dll`, which is injected into a target game process (and child processes
spawned by the game) to ensure that shared base and content files are never overwritten, deleted,
or modified in place.

## How it works

When injected into a process:
1. It reads its configuration from the file path pointed to by the `AGORA_VFS_CONFIG` environment variable.
2. It installs NT API hooks (`NtCreateFile`, `NtOpenFile`, `NtSetInformationFile`, `NtQueryAttributesFile`,
   `NtQueryFullAttributesFile`, `NtQueryInformationByName`, `NtQueryDirectoryFile`, `NtQueryDirectoryFileEx`,
   `NtClose`, and `CreateProcessInternalW`).
3. If all hooks install successfully, it opens and signals the named Win32 event specified in `ready_event`
   so the launcher knows it is safe to resume the suspended process.
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

If the configuration cannot be read or is invalid, the DLL installs no hooks and does not signal `ready_event`.
