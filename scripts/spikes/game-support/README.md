# Multi-game spike

`agora-game-spike.ps1` answers, on a real Windows machine, the questions the multi-game design
depends on. It is a measurement tool, not product code, and nothing in Agora calls it. Findings go
into the multi-game plan; this folder can be deleted once they are recorded.

Run from PowerShell (Windows PowerShell 5.1 or PowerShell 7). If scripts are blocked, run
`Set-ExecutionPolicy -Scope Process Bypass` in that window first. Do **not** run it elevated.

Every mode writes a JSON report to `%LOCALAPPDATA%\AgoraSpike\reports`. `C:\Users\<name>` paths
and the computer name are redacted unless `-NoRedact` is passed.

## Modes, in the order worth running them

| Mode | Question it answers | What it touches |
|---|---|---|
| `Inventory` (default) | Which stores and games are installed, on which drives and file systems; which engine and mod loader each game uses; what MO2 and Vortex setups exist and how they are laid out; whether Skyrim's SKSE and Address Library match the game version; where Paradox mods live; who owns `nxm://` links. | Read-only, except one empty probe file created and deleted in each Xbox game folder to learn whether it is writable. |
| `VerifyTest -SteamAppId <id>` | When Steam repairs a file, does it rewrite it in place or replace it with a new file? This decides whether a hardlinked copy of a game version stays frozen when Steam changes the real install. | Hardlinks one small game into `<drive>:\AgoraSpike\links-<id>`, flips one byte of its smallest file, and asks you to run Steam's *Verify integrity*. Use a small game you do not mod. |
| `LinkArm` / `LinkCheck -SteamAppId <id>` | The same question for a real update, which may behave differently from a verify. Arm a game that updates often, then check after its next update. | Hardlinks only. |
| `StockRoot [-Game SkyrimGOG\|SkyrimSteam]` | Does Skyrim run from a private copy of its install, outside the Steam or GOG folder, through SKSE? This is the pinned base layer of each Skyrim instance, whether mods are layered with a virtual file system or with links. | Builds `<drive>:\AgoraSpike\stock-<flavor>`: hardlinks for data, copies of every `.exe`/`.dll`. Mod files deployed by Vortex, SKSE plugins and ENB/ReShade are left out. Backs up and restores `plugins.txt`, `loadorder.txt` and the Skyrim INIs. Reach the main menu, do not load or save, quit, then press Enter. If Skyrim keeps running in the background afterwards (common), the script says so and offers to end it. The files are restored even if the script is interrupted. |
| `ToolRecord [-Root <dir>...]` | What exactly does Nemesis (or Pandora, BodySlide, ...) write, and does it write in place, replace files, or write through a hardlink into a mod manager's staging folder? | Read-only snapshots before and after you run the tool yourself. When a Skyrim MO2 instance exists (global, or portable beside `ModOrganizer.exe`), watches its `mods`, `overwrite` and `profiles` folders and the game folder it points at; otherwise the Steam or GOG install. Always adds the AppData and My Games folders. `-Root` replaces all of that. |
| `RestoreUserFiles [-Game ...]` | | Puts back the `plugins.txt` / INI files from the newest StockRoot backup, after showing which ones differ. Backups from the first version of the script carry no record of their install: it lists them, you pick one, and `-Game` says which install it belongs to. |
| `StoreProbe [-Name <text>] [-TryLaunch]` | How Microsoft Store / Xbox app games are put together: what `MicrosoftGame.config` declares (the real executable, any mod-folder settings), the Windows app id that launches them, whether their executables are readable or store-encrypted, and whether folders are real or redirected. With `-TryLaunch` and a `-Name` matching one game, which of three ways of starting it works: its own exe, `gamelaunchhelper.exe`, or the app id the Xbox app uses. | Read-only, apart from the Xbox folder write probe. `-TryLaunch` starts the game up to three times and asks you what happened each time. |
| `Cleanup` | | Removes every `<drive>:\AgoraSpike\*` folder that carries this script's marker file. Hardlink removal deletes only the extra name, never the game's own files. Reports and backups stay. |

Suggested order: `Inventory`; `VerifyTest` on a small, unmodded game (Balatro, Slay the Spire
or Brotato); `StockRoot -Game SkyrimGOG`, then `StockRoot -Game SkyrimSteam` (add
`-AddSteamAppId` if Steam relaunches the game from its own folder); `ToolRecord` around a Nemesis
run; `LinkArm` on a game that updates often, and `LinkCheck` after its next update.

## What the answers change

- **Steam replaces files** → a pinned game version can be hardlinked from the Steam install at
  almost no disk cost. **Steam patches in place** → big archives have to be copied instead
  (about 15 GB for Skyrim), or Agora keeps its own copy of the version.
- **Skyrim runs from a stock root through SKSE** → instances can pin their game version, so a
  Steam update can never break SKSE. **Steam relaunches it from its own folder** → that needs
  `steam_appid.txt`, or it is a GOG-first feature.
- **Nemesis writes in place through hardlinks** → a hardlink deployment would corrupt shared
  mod files, which is the strongest argument for copy-on-write (a virtual file system or an
  overwrite layer).

`test-spike-parsers.ps1` checks the script's parsers against fake Steam, MO2, Vortex and game
folders and runs anywhere PowerShell runs: `pwsh -File test-spike-parsers.ps1`.
