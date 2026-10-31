# Spike 2: write isolation (MASTER_SPEC §26.5, §26.13 phase 0b)

Measured 2026-10-04 on Windows 11 Pro 26200, usvfs 0.5.7.2 (MO2's official release
`usvfs_v0.5.7.2.7z`, SHA-256 `c6252eed78ee1c307733a4412cb68522cffc48107be4795c4e38b2b8d7c76d01`).
Harness: `harness/` (Rust, `spike2 run --usvfs <bin> --sandbox <dir> --topology full|mo2 [--readonly] [--acl]`).

## Setup

Three lower layers, each with one file per operation:

- `content`: a mod folder (shared content store).
- `base_copied`: a pinned base's own copy.
- `base_linked`: a pinned base file hardlinked to `store/` (the Steam/GOG install).

`overwrite/` is linked with `LINKFLAG_CREATETARGET`. Topology `full` mounts base, mod and overwrite
onto an empty folder; `mo2` mounts mod and overwrite onto the base folder itself, as MO2 does with a
game's Data folder. The probe runs hooked through `usvfsCreateProcessHooked`, does ten operations per
layer, then starts a child of itself (hooked automatically) that repeats them on a second file set.

Operations: write in place (`OPEN_EXISTING` + `WriteFile`), truncate (`CREATE_ALWAYS`), replace by
`MoveFileExW(REPLACE_EXISTING)`, replace by handle rename (`SetFileInformationByHandle(FileRenameInfo)`),
`ReplaceFileW`, `DeleteFileW`, delete by handle (`FileDispositionInfo`), POSIX delete
(`FileDispositionInfoEx`), POSIX delete ignoring read-only, rename away (`MoveFileExW`), plus create new.

## Results (30 lower-file operations per process; game and child identical)

| Protection | Topology | Lower file changed | Refused (game sees an error) | Isolated |
|---|---|---|---|---|
| none | full | 24 | 6 | 0 |
| none | mo2 | 30, plus stray files left in the base folder | 0 | 0 |
| read-only attribute | full | 6 | 24 | 0 |
| read-only attribute | mo2 | 6 | 24 | 0 |
| ACL deny (no SYNCHRONIZE) | full | 0 | 30 | 0 |

Creating a new file always lands in `overwrite/` (isolated) in every configuration.

- **usvfs has no copy-on-write.** Writes, truncates and replaces change the real lower file; every
  delete deletes it; rename-away moves it out of the mod folder into overwrite. For hardlinked base
  files, in-place writes and truncates change the store install too. `usvfs.h` lists copy-on-write
  under "Maybe"; no fork implements it (checked 15 most recent forks).
- **Handle-based rename and `ReplaceFileW` bypass usvfs** (`NtSetInformationFile` is not hooked). With
  an empty mount they fail (error 3 / 1175); over a real folder they create physical files in the game
  folder, and in two cases the game then reads the mod's file instead of its own write.
- **Read-only attribute leaks two ways:** rename-away and `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`.
  It also sets read-only on the store's file for hardlinked base files (attributes belong to the file,
  not the link): 20 of 20 store files flipped.
- **ACL deny is complete** for the denied user: `WriteData, AppendData, WriteExtendedAttributes,
  WriteAttributes, Delete` on files and `CreateFiles, CreateDirectories, DeleteSubdirectoriesAndFiles`
  on folders. Reads keep working. Same catch as read-only: the ACL belongs to the file, so a deny on a
  hardlinked base file also denies the store's updater. Gotcha: `icacls /deny` widens the rights to
  its `W`, which includes SYNCHRONIZE and so denies every open, reads included; use `Set-Acl` /
  `FileSystemAccessRule(..., 'Deny')`, which omits SYNCHRONIZE from deny entries.
- Licence: GPL-3.0-or-later with a section 7 exception for FOSS (link and distribute unmodified
  binaries), conditional on crediting "usvfs - User-Space Virtual File System, Copyright (C) Sebastian
  Herbord", the licence and a repository link in the UI and user docs.

## Real games through usvfs (`spike2 launch`, full topology: base mounted on an empty folder)

| | Skyrim SE GOG 1.6.1179 (Copied base) | Witcher 3 GOG 5.00c (Linked base) |
|---|---|---|
| Mount (`usvfsVirtualLinkDirectoryStatic`) | 224 files, 9 ms | 1,765 files, 0.2 s |
| Ran to main menu, clean exit | yes, 355 s | yes, 293 s |
| Files opened through the mount | 182 | 1,222 |
| Write-access opens of existing lower files | 0 | 1: `content/metadata.store` (45.6 MB), `GENERIC_WRITE` + `FILE_OVERWRITE_IF`, after one read; plus its `.stamp` |
| Deletes, renames | 0 | 0 |
| New paths | `Data/ShaderCache`, `Mods`, `Creations` (empty folders, in overwrite) | none |
| Writes outside the mount | `SkyrimPrefs.ini` (My Games) | none seen |
| Base afterwards | full hash verify clean, 224/224 | verify clean apart from its two declared writes; no linked archive opened for writing |

- **Copy-on-open is cheap for these games.** The one write was a whole-file rewrite (`OVERWRITE_IF`), which
  needs no copy: the new file simply goes to the writable layer. Only an in-place edit of existing bytes
  (`OPEN`/`OPEN_IF` with write access) has to copy first. Tools (BodySlide, Nemesis: F4) are where that
  will show up, so they are the next thing to measure.
- **The launch folder needs real binaries.** With the base mounted on an empty folder, Windows' loader
  resolves the exe's static imports before usvfs's hooks are live: Skyrim exited `0xC0000135`
  (`STATUS_DLL_NOT_FOUND`) on `bink2w64.dll` / `Galaxy64.dll`. The controller is not hooked either, so
  `CreateProcess` needs the exe and the working folder on disk (errors 2 and 267). Fixed by hardlinking
  every `.exe`/`.dll` in the exe's folder into the mount from the base's own copies (6 for Skyrim, 45 for
  Witcher). MO2 never meets this because its game folder is physical.
- Windows 11 24H2 game paths used `NtQueryDirectoryFileEx` (3,862 calls for Witcher), which usvfs
  hooks; a replacement must handle both `NtQueryDirectoryFile` and `...Ex`.

## Prototype: our own copy-on-write VFS in Rust (`agvfs/`)

An injected DLL (`retour` detours, `CreateRemoteThread(LoadLibraryW)` into a suspended process,
children injected from a `CreateProcessInternalW` hook) with one upper layer and ordered lower layers.
It hooks the NT calls every Win32 file API funnels through: `NtCreateFile`, `NtOpenFile`,
`NtSetInformationFile` (rename, delete), `NtQueryAttributesFile`, `NtQueryFullAttributesFile`,
`NtQueryInformationByName`, `NtQueryDirectoryFile(Ex)`, `NtClose`. Rules: a lower file is never opened
with write or delete rights; writing copies it up (a whole-file rewrite skips the copy); deleting
records a whiteout marker (`<upper>\.agvfs-wh\<path>.wh`); renaming copies and whites out. Listings are
merged from each real folder's raw entries in the caller's format (classes 1, 2, 3, 12, 37, 38, 60, 63),
upper first, whiteouts removed.

| Matrix (62 operations, game and child) | Changed a lower or store file | Failed for the game |
|---|---|---|
| agvfs | **0** | **0** |
| agvfs, lower files ACL-denied | **0** | **0** |

Every operation succeeds as the game expects, including `ReplaceFileW` and handle-based rename and
delete, which usvfs does not intercept. A consistency check passes too: what the game can open, list
(`FindFirstFile`), and get attributes for (`GetFileAttributesW`) are the same 38 names.

**Skyrim SE GOG ran to the main menu and into the world** under agvfs, from the Copied base. With
`verbose=1` it opened the same 177 base files as under usvfs (no difference apart from the `Data`
folder itself). Bugs found on the way, each now covered: Win32 sends relative paths as names relative
to the current-directory handle (resolved with `GetFinalPathNameByHandleW`); Windows 11 24H2's
`GetFileAttributes(Ex)W` asks by name (`NtQueryInformationByName`), and missing it sent Skyrim into a
fallback search (`data\DATA\<plugin>` probes, harmless); a whiteout marker for `Data\x` made a folder
that read as a whiteout of `Data` (markers now end in `.wh`); the close hook deadlocked against the
listing lock (internal work now bypasses it).

**What it does not do yet:** usvfs hooks 47 functions to agvfs's 12. Not needed by Skyrim so far but
likely needed elsewhere: virtual current directories (`SetCurrentDirectory` into a folder that exists
only in a lower layer), handle-name queries (`NtQueryInformationFile` name classes, `NtQueryObject`,
`GetFinalPathNameByHandle` returning the real path), `GetModuleFileName` for binaries loaded from a
lower layer, 32-bit processes, `FILE_OPEN_BY_FILE_ID`, and listings in the newer
`FileId64Extd*`/`FileIdAllExtd*` classes (logged, passed through).

## Link deployment (no injection), the fallback for games that block it

The mount is a real folder of hardlinks, the mod over the base; the game runs on it directly.

| Matrix | Changed a lower or store file | Failed for the game |
|---|---|---|
| links | 12: in-place writes and truncates, through to the store for store-linked files | 0 |
| links, lower files ACL-denied, deployment folder grants delete-child | **0** | 12 (the in-place writes) |
| links, lower files ACL-denied, deployment folder inherits only Modify | **0** | 60 (every change) |

- Deletes and renames touch only the deployment's link names, so without ACLs they are safe and
  only in-place writes leak.
- With ACLs nothing leaks. Whether deleting or renaming a deployed link works depends on the
  **deployment folder**: the file's deny entry refuses `DELETE`, but Windows also allows a delete when
  the parent folder grants `FILE_DELETE_CHILD`. Full Control includes it (a sandbox under
  `%LOCALAPPDATA%`: 12 refused), Modify does not (a sandbox on a second drive inheriting Modify:
  60 refused). Agora creates the deployment folder, so it grants the user delete-child there
  explicitly, and then only true in-place edits fail. A game that edits a deployed file needs its
  own copy of it.
- **Creating a hardlink needs `FILE_WRITE_ATTRIBUTES` on the target**, so the content store's deny
  must leave `WriteAttributes` open, or links cannot be made after the files are protected
  (`ERROR_ACCESS_DENIED` otherwise).

**Witcher 3 GOG ran 42 minutes under agvfs** (menu and play, clean exit) from its Linked base, whose
archives are hardlinked to the GOG install. Under agvfs it deleted `content/metadata.store` (a whiteout;
the base file untouched) and wrote a new one (45.6 MB) plus its `.stamp`, both into the upper layer, with
no copy of the old bytes. The base's `metadata.store` kept its timestamp from the earlier usvfs run, and
the base verifies clean apart from those declared writes. Under usvfs the same moment showed as an
`OVERWRITE_IF` open of the base file, which then rewrote it in place.

## Compatibility run: agvfs with the store install as the read-only lower layer

`compat.py`: snapshot the install (path, size, mtime of every file), launch under agvfs, stop at
45 s, re-snapshot. Every `.exe`/`.dll` in the install is placed physically at its path in the mount
(copied, never linked, for a store install).

| Game | Engine | Running at 45 s | Files opened | Writes | Install afterwards |
|---|---|---|---|---|---|
| RimWorld | Unity (Mono) | yes, 825 MB, crash-handler child started | 977 | none | unchanged (1,146 files) |
| Slay the Spire | bundled Java runtime | yes, 740 MB | 55 | its log appended: one copy-up | unchanged |
| Balatro | LÖVE, reads its own exe as an archive | yes, 515 MB | 3 | none | unchanged |
| Satisfactory | Unreal 5, launcher starts the shipping exe | yes, 1.9 GB (child hooked) | 530 | none | unchanged |

- **Every binary must be physical, not only those beside the first exe.** With only the launcher's
  folder placed, Satisfactory's launcher found `Engine\Binaries\Win64\...-Shipping.exe` through the VFS
  but `CreateProcess` maps the image in the kernel, so the game never started. Placing all 509
  binaries (811 MB) fixed it. In the product this is cheap: a Copied base already holds its own copies
  of every binary, and the launch folder hardlinks to those.
- Slay the Spire is the first measured in-place edit (appending to a log), the case copy-on-open
  exists for.

## Not yet measured

- Tools that edit in place (BodySlide, Nemesis), which decide how often copy-on-open copies bytes.
- Mount cost: agvfs has none (it resolves per open); usvfs links every file at mount (1,765 files, 0.2 s).
- A game with anti-cheat through link deployment.
