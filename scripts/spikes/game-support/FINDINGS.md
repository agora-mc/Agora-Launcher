# Multi-game spike: findings

Measured on one Windows 11 machine (NTFS, drives C: and D:, Windows PowerShell 5.1) on
2026-09-29. Raw reports are in [`spike results/`](spike%20results/). Each finding names the
report it comes from and what it changes in the design.

## 1. A private copy of a game version is nearly free, and SKSE runs from it

| | GOG 1.6.1179 | Steam 1.6.1170 |
|---|---|---|
| Hardlinked | 215 files, 32.0 GB | 110 files, 16.2 GB |
| Copied (every `.exe`/`.dll`) | 9 files, 60 MB | 7 files, 40 MB |
| Build time | 0.2 s | 0.1 s |
| Launched via | `SkyrimSE.exe` (no SKSE installed) | `skse64_loader.exe` |
| Ran from the copy | yes | yes; Steam did not relaunch it from its own folder, no `steam_appid.txt` needed |
| Main menu | yes | yes; SKSE's `config path` and `plugin directory` both inside the copy |

`stockroot-20260929-201036.json`, `stockroot-20260929-222423.json`

**Design:** each instance pins its own game version. A store update can no longer break SKSE,
because the instance keeps the executable it was built against.

## 2. Hardlinks are not safe for files the game writes

During the Steam run the game rewrote `d3dx9_42.log` in the copy's root. The copy was a hardlink,
so **the real Steam install's file changed too**. Skyrim AE also rewrote `plugins.txt` in AppData at
launch (restored by the script).

`stockroot-20260929-222423.json` (`realInstallFilesChanged`, `userFilesChangedByGame`)

**Design:** the pinned base is read-only. Files a game or tool writes must be copies, or be
redirected by the VFS to an instance-owned layer. `plugins.txt` is written by Agora per launch, per
profile, as MO2 does.

## 3. Steam replaces files rather than patching them in place (one data point)

Verify integrity repaired a corrupted file by writing a new file: the original path got a new file
identity and the hardlinked copy kept the old content.

`verifytest-20260929-195903.json`

**Design:** a hardlinked pinned copy survives Steam repairs. Pending: the same check across a real
update (`LinkArm` now, `LinkCheck` after the next update).

## 4. Under MO2's VFS, a tool's edits to existing files land in the mod's own folder

BodySlide, run through MO2 (portable instance `C:\Modding\MO2`), rewrote `BodySlide.xml` and
`Log_BS.txt` **in place inside its own mod folder**; nothing went to `overwrite`. MO2 rewrote seven
profile files by replacing them.

`toolrecord-20260929-224110.json`

**Design:** Agora's content store is shared between instances, so it cannot be the live folder a
VFS exposes for writing. Modified files need copy-on-write into an instance layer. Pending: a Nemesis
run on a complete pack, to see generated-output volume and where it lands.

## 5. A game's runtime identity is store + exact version

Three Skyrim installs on one machine: Steam 1.6.1170 (`D:`), GOG 1.6.1179 (`D:`), and an older copy
on `C:` that Steam no longer tracks, reporting **1.7.104.0**, which the MO2 instance above points at.
That instance's Address Library covers twelve runtimes up to 1.6.1179 but not 1.7.104. GOG builds
need their own SKSE build even at similar versions.

`inventory-20260929-223500.json`

**Design:** loaders, SKSE plugins and Address Library are matched against (store, version), and a
mismatch is a pre-launch diagnostic, not a crash.

## 6. MO2 setups carry enough to re-identify mods

The MO2 instance has 107 mods; 101 record a Nexus mod id and their original archive name in
`meta.ini`. Its profile uses local saves and local INIs. A third, forgotten instance turned up once
the script scanned beyond the usual locations.

`inventory-20260929-223500.json`

**Design:** MO2 import maps mods to their source and keeps per-profile saves and INIs. Discovery
scans drives for `ModOrganizer.ini` instead of trusting the usual locations.

## 7. Microsoft Store games: launchable, not readable

- `C:\Program Files\WindowsApps\<package>` resolves to `C:\XboxGames\<game>\Content`.
- Every executable of nine store games refused read access, so they can be neither hashed nor
  copied. The `Content` folder does accept new files.
- CK3 and Minecraft Dungeons both started through their own executable, `gamelaunchhelper.exe`, and
  their app id; the running process reports its path under `WindowsApps`. CK3's
  `MicrosoftGame.config` lists the Paradox Launcher, not `ck3.exe`.

`storeprobe-*.json`

**Design:** store games get no pinned copies and no executable hashes. Mods go where the game reads
them (Documents or AppData for CK3 and Dungeons) or as added files. A VFS is probably unnecessary
for them and untested. Pending: launching `binaries\ck3.exe` directly (`-Exe`).

## 8. Discovery has to tell games from add-ons and tools

GOG lists Cyberpunk three times (base, Phantom Liberty, REDmod) and Skyrim twice for one folder;
CK3 installs about twenty DLC folders under `XboxGames`; Epic lists texture packs and the Unreal
Engine editor. Several games match no engine family (Baldur's Gate 3, Enshrouded, Jurassic World
Evolution 2, Kingdom Come, PlanetSide 2), and the store Brotato has no separate `.pck` to detect.
`nxm://` links currently open Vortex.

`inventory-*.json`

**Design:** store adapters classify base game / DLC / tool. Engine families are optional parents; a
game definition declares its engine and loaders instead of relying on detection. Agora claims
`nxm://` only when the user asks it to.

## Still to run

- `ToolRecord -Mo2Instance <complete pack>` around a Nemesis run.
- `LinkArm -SteamAppId 281990`, then `LinkCheck` after Stellaris next updates.
- `StoreProbe -Name "Crusader Kings III" -TryLaunch -Exe binaries\ck3.exe`.
