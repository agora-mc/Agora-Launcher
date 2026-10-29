# Agora CLI reference

The `agora` binary exposes the same core instance, catalog, health, launch, snapshot, runtime, install, and community-plugin services used by the desktop application. It is intended for advanced users, support diagnostics, scripting, and AI assistant apps.

The CLI can modify the same data used by the desktop application. Read the safety section before experimenting.

## Build or install

The desktop app includes the CLI: `agora.exe` in the install folder on Windows and beside `Agora Launcher.exe` in the portable ZIP, `Agora Launcher.app/Contents/MacOS/agora` on macOS, and `/usr/bin/agora` from the Linux packages. Settings → Services → AI assistant shows its exact path. It is not added to `PATH`, so call it by that path or add it yourself. The standalone `agora-cli-*` archives on the release page are for machines without the desktop app.

To build it from source, from the repository root:

```bash
cargo build -p agora-cli
```

The development binary is written under Cargo's target directory:

```text
target/debug/agora
```

To install it into Cargo's binary directory:

```bash
cargo install --path crates/agora
```

Confirm the installed interface:

```bash
agora --version
agora --help
```

## Safety first

Before running a modifying command, print the paths Agora resolved:

```bash
agora paths
```

For experiments, use an isolated data root:

```bash
agora --data-dir ./tmp/agora-cli-test paths
```

Keep these rules in mind:

- The default data root may be shared with the desktop application.
- The desktop app and the CLI can run at the same time. An instance one of them is changing is locked, and the other waits up to 30 seconds and then names who holds it; the desktop app reloads an instance the CLI changed. Avoid editing the same instance setting from both at once: the last save wins.
- Use `--dry-run` on install, remove, and update operations before executing a complex plan.
- Create a snapshot before manual or high-risk changes.
- Use disposable instances when learning destructive commands.
- `instance delete` moves the instance to Recently deleted, where it can be restored for 30 days; then it goes to the OS trash. `snapshots restore`, `snapshots delete`, `lockfile import`, and some install conflict choices can replace or remove files.
- Never place access tokens or account codes in command history, log files, or bug reports.
- `--data-dir` isolates files under the Agora data root, but Microsoft credentials use the operating-system credential store. `auth status`, `auth login`, and `auth logout` therefore inspect or change the same credential entry used by the desktop app. The exception is a portable copy (a `portable.txt` beside the executable, with no `AGORA_DATA_DIR` override), which never uses the operating-system store and keeps its credentials encrypted in its own data folder.

## Global syntax

```text
agora [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS]
```

Global options:

| Option | Purpose |
| --- | --- |
| `--data-dir <PATH>` | Override the Agora data root. Without it the CLI resolves the root exactly as the desktop app does, so `AGORA_DATA_DIR` and a `portable.txt` marker beside the executable apply to both |
| `--json` | Shorthand for `--output json` |
| `--output <human|json>` | Select output format |
| `--registry-repo <OWNER/REPO>` | Override the catalog repository for development or testing |
| `--log-file <PATH>` | Append CLI diagnostics to a chosen file |
| `--help` | Show help |
| `--version` | Show the CLI version |

Human output is the default. Prefer `--output json` for scripts.

NDJSON is not a public output format. Commands return one ordinary JSON value when JSON output is selected. Line-delimited JSON-RPC is available only through `agora mcp serve --stdio`.

Progress and diagnostic messages may be written to standard error while command results are written to standard output. Scripts should capture the streams separately.

## A ten-minute tour

Inspect the environment:

```bash
agora paths
agora registry status
agora list-instances
```

Synchronize the signed catalog:

```bash
agora registry sync
```

Create a disposable instance:

```bash
agora instance create "CLI Test" \
  --mc-version 1.21.1 \
  --loader fabric \
  --loader-version 0.16.10
```

Use `list-instances` to obtain the generated instance ID:

```bash
agora list-instances
```

Search the curated catalog:

```bash
agora mod search sodium --content-type mod --mc-version 1.21.1
```

Resolve an install without changing files:

```bash
agora mod install sodium <INSTANCE_ID> --dry-run
```

Execute only after reviewing the plan:

```bash
agora mod install sodium <INSTANCE_ID>
```

A download whose bytes do not match a curator pin, or the hash Agora recorded when the same release file was installed before, is refused with `ERR_HASH_CONFIRMATION_REQUIRED`. The message names the file, the expected and downloaded SHA-256, and the source of the expectation. Check the file yourself, then install it anyway with `--install-anyway`. A mismatch with the hash the source published cannot be overridden. A file whose source published no checksum installs with a notice, and is recorded as not verified.

```bash
agora mod install sodium <INSTANCE_ID> --install-anyway
```

The same flag works on `agora mod update`.

Check health and launch:

```bash
agora health <INSTANCE_ID>
agora launch <INSTANCE_ID> --timings
```

Inspect and install a community plugin:

```bash
agora plugin list
agora plugin preview package ./example-plugin.zip
agora plugin install package ./example-plugin.zip
```

## Command map

### Discovery and paths

| Command | Purpose |
| --- | --- |
| `agora paths` | Print resolved application, database, cache, runtime, and instance paths |
| `agora list-instances` | List local instances |
| `agora get-instance <ID>` | Print one instance |
| `agora inventory <INSTANCE>` | Inspect installed content |
| `agora health <INSTANCE>` | Run the local health scanner |

### Games

Lists the game installs Steam, GOG, Epic and the Microsoft Store know about, each
classified as a base game, add-on or tool (MASTER_SPEC §26.3), and manages pinned bases
(MASTER_SPEC §26.4). `--json` is supported on discovery, listing, and all base commands.

| Command | What it does |
|---|---|
| `agora games discover` | List game installs across supported stores |
| `agora games list` | List supported games and their identified installs |
| `agora games folder add <path>` | Add a game folder the store scan did not find (MASTER_SPEC §26.3), such as a copy on another drive or a portable install. The folder must exist, must not be a store install or inside one, must not already be listed, and must hold the executable of exactly one game Agora knows; otherwise it is refused and the message says why. The folder is then searched by `games discover` and `games list` |
| `agora games folder remove <path>` | Stop searching a folder added with `games folder add`. Its files are not touched; a folder that is gone from disk can still be removed |
| `agora games folder list [--json]` | List the folders you added, and whether each is still on disk. A folder that is gone is listed, and is not searched |
| `agora games catalog list <game> [--json]` | List the curated catalog entries for one game other than Minecraft, such as `skyrim-se`, with their stores, game versions and frameworks (MASTER_SPEC §26.8) |
| `agora games catalog install <instance> <item> [--install-anyway] [--dry-run] [--json]` | Install a curated catalog entry into a game instance (MASTER_SPEC §26.8). The entry's compatibility entry must match the instance's store and version, and its required frameworks must be installed at `min_version` or newer (an unreadable framework version is a warning). The file comes from the entry's GitHub release (the newest non-draft, non-prerelease release with a matching asset) or its `direct_hash` URL, and is checked against what its source published, with `--install-anyway` only overriding a curator pin or a hash remembered from an earlier install. An archive with a FOMOD installer is imported but not added. That is an expected outcome, not an error: the command prints what to run next (`agora games content fomod install`, with `--defaults` or `--choose`), without an `Error:` prefix, and exits `71` (the decision code in the exit-code table) so a script can tell it did not install. With `--json` it prints the report (`state` `needs_installer`) and the same exit code. `--dry-run` shows the plan and downloads nothing |
| `agora games base build <install-id> [--mode linked\|copied]` | Build a pinned base from an identified install (default mode: linked) |
| `agora games base list` | List all pinned bases and their availability. A record that cannot be read is listed as damaged, with its file and the reason |
| `agora games base verify <base-id> [--full]` | Verify base integrity (quick cheap check by default, full hashes all files) |
| `agora games base remove <base-id>` | Delete a pinned base and its manifest. A damaged record is removed with its folder in the bases folder, if there is one |
| `agora games content add <path> [--name N]` | Add a zip, 7z or RAR archive (RAR is read through Windows' own tar) or a folder to the content store |
| `agora games content list` | List all items in the content store |
| `agora games content show <item>` | Show files in a content item |
| `agora games content verify [<item>] [--full]` | Verify integrity of content items |
| `agora games content remove <item>` | Delete an item from the content store |
| `agora games content fomod show <item>` | Show the steps, groups and options of the FOMOD installer in an archive item |
| `agora games content fomod install <item> [--instance <id>] [--choose "Step/Group/Plugin"]... [--defaults]` | Run an archive item's FOMOD installer into a new content item (the archive's objects are shared, the choices are recorded); with `--instance`, check the installer's file conditions against that instance and add the result to it |
| `agora games launch <base-id> [--wait] [--launch-anyway] [--plain]` | Launch a game from its pinned base (`--plain` skips a framework loader such as SKSE's and starts the game's own executable). A framework built for another game version refuses the launch unless `--launch-anyway` is given, as a damaged base does |
| `agora games instance create <install-id> [--name N] [--id ID] [--mode linked\|copied]` | Create an instance for a game from an identified install |
| `agora games instance list` | List all game instances, Minecraft included |
| `agora games instance launch <id> [--wait] [--launch-anyway] [--plain] [--deployment virtual\|links\|copies\|auto] [--fall-back]` | Launch a game instance; it starts through a framework loader (SKSE's `skse64_loader.exe`) when one is installed, and says so (`--plain` starts the game's own executable instead). `--deployment` overrides the instance's rung for this launch. If the virtual file system cannot start (`agora_vfs.dll` missing, injection refused, the game ended before the DLL confirmed its hooks), nothing is launched and Agora says what failed and why, what linked files give up, and asks `Try again from linked files? [y/N]`; yes launches again from linked files for this launch only and prints `set-deployment <id> links` to keep it. Without a terminal, or with `--json`, it never asks: it exits 1 and prints both commands (`--json`: `nextDeployment` and `retry` in the error object). `--fall-back` answers yes in advance, for a rung you chose too, and announces the step down. After a `--wait` session under the virtual file system, a program the game started that the DLL had to stop (it could not protect it) is named with the reason, and the same choice is offered. Before it starts, the launch checks the framework files against the game's version: a framework built for another version (SKSE, or an Address Library for another runtime) refuses the launch and exits 1, naming the framework, what the game has, what it needs and the repair; `--launch-anyway` starts the game anyway and prints the findings as a warning. After the plugin list is synced, the launch also checks the plugin load order (see `plugins check`): a master that is missing or inactive, a master loop, or too many plugins refuses the launch and exits 1, naming each problem; `--launch-anyway` starts the game anyway. A master that loads after its plugin only warns, because the game still starts, but the plugin's references may resolve wrongly: the warning names `agora games instance plugins sort <id>`, which fixes it. An unreadable header or a plugin listed twice only warns too. With `--json`, stdout is exactly one JSON object: the human lines (`Deployed:`, `Restored N per-user file(s)`, the quick-exit hint) are not printed there. Their facts are fields instead: `deploy_summary` (`status` `built` with `linked`, `copied`, `copied_bytes`, `config_copied` and a `harvest` count object, or `up_to_date`; `null` when nothing was deployed), and, with `--wait`, `user_files_restored` (`{restored, changed}`, or `{error}` when the per-user files could not be put back) and `session_ended_quickly`. A warning or failure note still goes to stderr |
| `agora games instance check <id>` | Check the framework files of an instance against its game's version, without deploying or launching anything. It prints each finding (what the game has, what it needs, the repair) and exits 1 when a framework was built for another version. A rule that cannot be checked (a bad game definition) is listed as a warning and does not fail the check, and a launch never refuses for it. It makes the same check a launch makes; `--json` prints `status` (`ok`, `warnings` or `findings`) and the `findings`. For a pinned instance of a game with a plugin list it also checks the plugin load order, lists its findings (`--json`: `load_order_findings`), and exits 1 for the ones a launch refuses. It also lists the tool output that is `stale` or `unknown` (`--json`: `generated_findings`) as warnings, which never change the exit code |
| `agora games instance set-deployment <id> virtual\|links\|copies\|auto` | Choose how an instance is deployed and run: under the virtual file system, from hardlinks, or from copies. `auto` clears the choice. A launch always asks before stepping down from the virtual file system, whatever is chosen (see `launch`) |
| `agora games instance delete <id>` | Delete a game instance and its folder |
| `agora games instance content add <instance> <item> [--into <path>]` | Add a content item to an instance's layer stack. An archive with a FOMOD installer is not added: that is an expected outcome, not an error, so the command prints what to run next (`agora games content fomod install <item> --instance <instance>`, with `--defaults` or `--choose`) without an `Error:` prefix, and exits `71` (the decision code). With `--json` it prints one envelope on standard error (`status` `needs_installer`, `installer_command`, `exitCode` `71`) |
| `agora games instance content list <instance>` | List content layers on an instance in priority order |
| `agora games instance content remove <instance> <item>` | Remove a content layer from an instance |
| `agora games instance content enable <instance> <item>` | Enable a content layer on an instance |
| `agora games instance content disable <instance> <item>` | Disable a content layer on an instance |
| `agora games instance content move <instance> <item> <position>` | Move a content layer to a 1-based position |
| `agora games instance content own-copy <instance> <item> on|off` | Configure whether all files deployed by a content layer are copies rather than links |
| `agora games instance deploy <instance> [--copies\|--deployment virtual\|links\|copies]` | Deploy an instance's content layers to its game folder, as the instance's chosen rung, else the mode a launch would use |
| `agora games instance undeploy <instance>` | Harvest game writes and tear down an instance deployment |
| `agora games instance plugins <instance> [--json]` | List the instance's load order: each plugin's position, state, master (`M`) and light (`L`) flags, lock, whether Agora manages it (a content layer deploys it) or you wrote the line, and the findings. Always-loaded plugins are marked `always` |
| `agora games instance plugins enable <instance> <plugin>` | Activate a plugin in the instance's plugin list |
| `agora games instance plugins disable <instance> <plugin>` | Deactivate a plugin; it stays listed and a redeploy leaves it that way |
| `agora games instance plugins sort <instance> [--dry-run]` | Sort the list so every master comes before the plugins that need it, with as few moves as possible. Locked plugins do not move; `--dry-run` prints the moves and writes nothing |
| `agora games instance plugins move <instance> <plugin> (--to N \| --before P \| --after P)` | Move a plugin to a 1-based position, or just before or after another plugin. Locked plugins do not move |
| `agora games instance plugins lock <instance> <plugin>` | Lock a plugin's place, so sort and move leave it where it is |
| `agora games instance plugins unlock <instance> <plugin>` | Unlock a plugin's place |
| `agora games instance plugins check <instance> [--json]` | Print the load order findings (a master that does not come first, a master loop, an unreadable header, a plugin listed twice, too many plugins). A master that loads after its plugin is named with the `plugins sort` command that fixes it. Exits 1 only when a launch would refuse (a missing or inactive master, a master loop, too many plugins); warnings exit 0 |
| `agora games instance ini <id> [<file> [<section> [<key> [<value>]]]] [--unset]` | Read and change the game's INI files as the instance keeps them. With no file, lists the game's per-user files for the instance's store, whether the instance has its own copy, and the game's own file. With a file, lists its keys and where they came from: the instance's copy, the game's file while the instance has no copy yet, or nothing. With a section and key, prints the value and exits 1 when it is not set; with a value, sets it; with `--unset`, removes the key. An edit writes only the instance's copy, which is swapped in for the next launch; when the instance has no copy yet it starts from the game's file. The game's own file is never edited. Lines Agora does not change are kept as they are: comments, spacing around `=`, the spelling of keys and sections, line endings, a byte order mark, and lines it cannot parse. Names are case-insensitive. Refused while the game's session is running, or while an earlier session's files have not been put back (`games user-files restore`). `--json` prints the result |
| `agora games instance saves <id> [own\|shared] [--json]` | Show the instance's save choice, or make it `own` (saves of its own, through the game's own save-folder setting, such as `SLocalSavePath` for Skyrim) or `shared` (the game's shared folder). With no choice, it shows the folder in use, the other folder and its saves, and the setting's value. Switching changes only where the game looks. It never moves, copies or deletes a save file, and it says that the saves in the other folder stay where they are. `own` records the setting's value from before, so `shared` can put it back; `shared` leaves a setting that was changed by hand since, and says so. Refused while the game's session is running |
| `agora games instance tools list <instance> [--json]` | List each tool the game declares (for example `nemesis` for Skyrim SE), with its current and previous output, and whether that output is `current`, `stale` (an input changed since it was built) or `unknown` (it was imported, so nobody recorded its inputs). A tool with no output says `not built`. A launch and `games instance check` warn about stale and unknown output; they never refuse for it, and say which command rebuilds it |
| `agora games instance tools run <instance> <tool> [--capture auto\|vfs\|links\|swap] [--yes] [--json]` | Run a tool in the instance and capture what it writes, so it never lands in a mod or the game's own writable layer. A tool that works on the real install (Skyrim's `nemesis` and `bodyslide`, which read the install path from the game) cannot be captured by the virtual file system or by links, because it writes where the game reads. On a linked or copied instance, `auto` refuses such a tool before anything is deployed: in a terminal it explains the swap in two lines and asks `[y/N]`; with `--json` or no terminal it refuses and prints the `--capture swap` command. `swap` is the way such a tool runs, on a real, writable copy of the instance's `Data` that is swapped in for the length of the run. The copy is made from the instance's plan as real files, never as links. On the same volume as the install it is kept in the instance's swap folder (`deployments\<instance>\swap\Data`) between runs: each run copies only what changed, and the first run copies everything. On another volume a copy is made on the install's volume for the run and deleted afterwards, so every run copies everything in and the changes back out. For the length of the run the real `Data` is renamed to `Data.agora-aside-<run>`, the copy is renamed into its place, and the tool runs from the install folder. When the tool exits the copy goes back, and the real `Data` comes back from the aside. Before anything is copied or moved, the swap shows what it costs: the disk space the copy needs and the free space on its volume; whether the first run copies everything and later runs only what changed, or every run copies everything in and back; that the real `Data` is renamed for the length of the run, so Steam must not update the game and the game must not be started; and a warning when Steam is running. In a terminal it asks `[y/N]`. With no terminal it needs `--yes`, which confirms the swap after the same costs are shown, and otherwise it prints the command and refuses. The journal is written before the copy is touched, and a crash is put back by the next launch, deploy, tool run, or `games tools-swap restore`; recovery discards the copy's writes rather than capturing them. The changes are captured by comparing the copy with the record it held before the run, by the change detection link capture uses (size, then time, then the bytes against the source). `swap` refuses while a program runs from the install folder, when `Data` is already a link, when the aside name is taken, or when the copy's volume lacks the space for it. Steam and the game must not run during a swap. Every run also reports what it wrote to the real install folder outside what Agora captured (a report, never a refusal; a buffer overflow says changes may have been missed) | `--capture auto` (the default) on a virtual instance runs every tool under the virtual file system, a tool that works on the real install included. When the virtual file system cannot start (for example a 32-bit game, which `agora_vfs.dll` cannot load into), nothing runs: the run says what happened and what the next tier does, and names it. The next tier is link capture for a tool that uses its own folder, and the full-copy swap for one that works on the real install. In a terminal `auto` asks `[y/N]` before it captures from that tier, and says what failed first. `--yes` confirms it, as it does a swap, under `--json` too: the next tier then runs, and its outcome is printed as JSON, whose `capture_reason` names the virtual file system's failure. Without `--yes`, `--json` or no terminal prints the envelope (with the exact retry command for that tier, and any offers) and exits 1. A linked or copied instance (set with `set-deployment`) captures a tool that uses its own folder from links directly, with no attempt at the virtual file system. `vfs` runs under the virtual file system only: when it cannot start it refuses, names the next tier and prints its command, and exits 1. `links` captures from linked files: the tool runs in the game's folder, and what it changed is compared with the deployment record. New and changed files become the output and deletions become whiteouts; a linked file that the tool changes is refused, and the run is discarded and names it. The farm the run used is then taken away, so the game's next launch deploys the promoted output. The instance is deployed in the virtual file system first (or in links, for link capture), and the game's user files are swapped in for the run, as for a launch. Refused while a game session is recorded for the game (its per-user files are swapped in and not yet put back), and while a game process runs from the instance's folder. The run holds the instance's lock for its whole length, so a launch of the same instance waits for it (30 seconds at most) and then fails. A run that exits 0 becomes the tool's output (a generated layer above the mods and below the game's writes); the output it replaces is kept for rollback, and the one before that is deleted. Any other run (a non-zero exit, a cancel, a crash) is discarded and kept as `failed-<run>` (the last three are kept), and the output in effect stays. It lists what the run wrote and deleted, says which capture it used, and exits 1 when the output was not promoted. A tool that fails twice in a row carries offers, which are shown and never applied. A run the virtual file system refused to start counts as a failure, as a discarded run does. The offers: the full-copy swap, with its costs (asked `[y/N]` in a terminal, or printed as the command), when a failing run was not a swap; and switching the instance to linked files, printed as the `set-deployment` command, only when the virtual file system failed to start or ended a program the failing run started, and never for a tool that works on the real install. The count is kept in the tool's folder (`failures.json`); a promoted run resets it, and a missing or unreadable count counts as none |
| `agora games instance tools rollback <instance> <tool> [--json]` | Make the previous output current again, so the game reads it on its next launch. Nothing is deleted, and running it again swaps back |
| `agora games instance tools remove <instance> <tool> [--json]` | Remove a tool's output from the instance: its layer goes, and so do its generations and discarded runs. The mods are not touched |
| `agora games instance tools diff <instance> <tool> [--json]` | Show the files added, changed and removed between the tool's previous and current output. It needs a previous output, so it fails until a second successful run |
| `agora games user-files status [<game>]` | List per-user file swap sessions in progress |
| `agora games user-files restore <game> <store>` | Restore original per-user files from a finished session |
| `agora games tools-swap status [--json]` | List the tool swaps in progress: the game, store, instance and tool, whether a tool run is still going, and what the real `Data` folder and its aside folder are now. Nothing is changed |
| `agora games tools-swap restore` | Put back every interrupted swap whose tool run is no longer going: the copy in place of `Data` is moved home (only after its marker is checked), the aside folder is renamed back to `Data`, the copy is deleted (only when its marker matches the journal), and the journal is deleted last. Nothing is merged or overwritten. A case that needs a hand (both folders exist, or a folder at the copy's path is not Agora's copy) is reported with the steps, and nothing is changed. A journal from the old junction swap (version 1) is refused by name. The same check runs at every CLI start and before any launch, deploy, tool run or user-files session for the game |
| `agora games tools-swap remove-copy <instance> [--json]` | Delete the instance's kept copy of its `Data` (`deployments\<instance>\swap\Data`) to free its space. Only a copy whose marker names this instance is deleted; any other folder there is refused and nothing is deleted. The instance's next swap copies `Data` again. Deleting the instance removes its swap folder too |
| `agora games import mo2 scan [--json]` | List Mod Organizer 2 setups on this machine: the instance folders under `%LOCALAPPDATA%\ModOrganizer`, and a shallow scan of each fixed drive for `ModOrganizer.ini` (to depth 3, skipping `Windows`, `Program Files*` and `$Recycle.Bin`). Shows each setup's path, game, profiles and mod count. The mods are not read |
| `agora games import mo2 <ModOrganizer.ini> --profile <name> [--name N] [--dry-run] [--copy-saves] [--json]` | Import one profile of an MO2 setup as a new game instance (MASTER_SPEC §26.10). Every listed mod is stored as it is on disk, enabled and disabled, so local edits survive and nothing is downloaded. Content layers keep MO2's priority. `overwrite` is split: the files a declared tool produces (Nemesis's behaviour files and its engine folder) become that tool's generated layer with its inputs unknown, and the rest become one "MO2 overwrite" layer above the mods. The plugin list keeps MO2's load order, activation and locks; the profile's own INIs and saves become the instance's own. Mod `meta.ini` claims are recorded as provenance, not verified. The setup is only read. Refused when the same setup and profile were imported before, unless `--name` gives a new instance. `--dry-run` prints the plan and stores nothing. Saves are copied (never moved) under Documents only with `--copy-saves` |

For games that keep a plugin list (Skyrim's `Plugins.txt`), deploying an instance adds the
plugins its content layers provide to the instance's own copy of the list, active, and removes
the lines of layers you take away. A plugin you deactivated stays deactivated, and lines you
wrote yourself are never removed. The game's own masters are never listed. `--wait` follows
every process running from the instance's game folder, so a loader that starts the game and
exits does not end the wait early.

For Skyrim Special Edition the list also follows the Creation Engine's load order rules. The
game always loads its own masters and the Creation Club plugins named in `Skyrim.ccc` first, so
those are shown as `always` and are never written into your list. A plugin that needs a master
must load after it, and a plugin whose header cannot be read is reported by name. `plugins check`
prints these findings. `plugins sort` moves masters up with as few moves as it can, and never
moves a locked plugin. A plugin that loads above a locked master it cannot get past is reported as
blocked: the only fix is to move that plugin down below the master, which sort does not do.

### Catalog

| Command | Purpose |
| --- | --- |
| `agora registry status` | Inspect the cached catalog and its active state |
| `agora registry sync` | Download and verify the latest signed catalog |
| `agora sync` | Convenience alias for catalog synchronization |

Use `--registry-repo` only for an intentional development or sandbox catalog. A different repository changes the trust and governance boundary.

### Registry review and governance

These commands only read. They never vote, and they never post to GitHub.

| Command | Purpose |
| --- | --- |
| `agora registry under-review` | List registry items under review, lowest score first |
| `agora registry reviews <ITEM>` | Show the curator reviews recorded for one item |
| `agora registry audit-log [--limit N]` | Show recent audit log entries, newest first (1-1000, default 200) |
| `agora registry resolutions [--limit N]` | Show recent triage resolutions (1-1000, default 50) |
| `agora governance summary <ITEM>` | Show the vote counts and status for one item |
| `agora governance events [--item <ITEM>] [--limit N]` | List governance events, newest first (1-1000, default 100) |
| `agora governance config` | Show the governance repository and environment this build uses |
| `agora governance diagnostics` | Run the governance setup checks, local and GitHub |

These queries read the installed registry, so they fail with an error when none is installed; run `agora registry sync` first. A registry database that cannot be read is an error too, never an empty list. An empty list means there is genuinely nothing to show. `governance diagnostics` prints one line per check and exits with status 1 when any check fails. Its GitHub checks are read-only requests. When GitHub is unreachable or you are not signed in, those checks report their own failure rather than passing.

### Instances

```text
agora instance create
agora instance clone
agora instance rename
agora instance lock
agora instance unlock
agora instance repair-loader
agora instance recommend-memory
agora instance delete
agora instance deleted
agora instance restore
agora instance purge
agora instance health <INSTANCE_ID> | --all
agora instance revert <INSTANCE_ID>
agora instance loader-version <INSTANCE_ID> <VERSION> [--yes]
agora instance java <INSTANCE_ID> [--path <FILE> | --clear-path] [--memory <MB>] [--jvm-args <ARGS>]
agora instance wrapper <INSTANCE_ID> [<COMMAND> | --clear]
agora instance window <INSTANCE_ID> [<WIDTH>x<HEIGHT> | --clear]
agora instance hooks <INSTANCE_ID> [--pre <COMMAND>] [--post <COMMAND>] [--env NAME=VALUE ...] [--clear]
agora instance notes <INSTANCE_ID> [<TEXT> | --clear]
agora instance group <INSTANCE_ID> [<NAME> | --clear]
agora instance share <INSTANCE_ID> [--options on|off] [--servers on|off]
```

Examples:

```bash
agora instance recommend-memory <INSTANCE_ID>
agora instance lock <INSTANCE_ID>
agora instance clone <SOURCE_ID> "Debug Copy" --no-saves
agora instance health --all
agora instance java <INSTANCE_ID> --memory 6144 --jvm-args "-Dexample.flag=true"
agora instance wrapper <INSTANCE_ID> "mangohud --dlsym"
```

Clone flags can omit saves, mods, resource packs, shader packs, screenshots, configuration, `servers.dat`, or options. Hard-link and symlink modes change filesystem behavior and should be used only when the destination and source relationship is understood.

Locking prevents ordinary content mutation. Unlock explicitly before applying intended changes.

Deleted instances wait in Recently deleted for 30 days. `instance deleted` lists them with their entry ID (the ID to pass to `restore` and `purge`, which is not the instance ID), name, deletion time, and size. `instance restore` brings one back with its files and records; if its original ID has been taken, it returns under a new one and says so. `instance purge` sends one to the OS trash now and asks first unless you pass `--yes`; Agora cannot restore it afterwards, though the OS trash may still hold the files. Agora also runs the same 30-day cleanup when an instance command starts, so expired entries reach the trash without the desktop app open.

```bash
agora --json instance deleted
agora instance restore <ENTRY_ID>
agora instance purge <ENTRY_ID> --yes
```

`instance health` runs the same cached health check the desktop app uses. With `--all` it checks every instance, prints the whole table, and then exits nonzero if any instance is red or cannot be read. `instance revert` returns an unlocked instance to the snapshot taken when it was last locked, and takes an undo point first. It refuses an instance that was never locked.

`instance loader-version` shows the current loader, the compatible versions for the enabled mods, and the recommendation, then asks before switching. A version outside the compatible list is refused before anything changes. `--yes` skips the question.

`instance java` with no options shows the current settings. `--path` must name an existing file, and `--clear-path` goes back to the global Java setting. `--memory` is in MB; Agora keeps heap sizes between 2048 and 32768 and says so when it adjusts one. Setting memory makes the choice manual, as creation does. `--jvm-args` replaces the instance's arguments.

`instance wrapper` sets the command the game is launched under, for example a frame-timing overlay. It is checked the way the launcher reads it, so an unmatched quote is refused. An empty command or `--clear` removes it.

`instance window` sets the size the game window opens at, for example `1280x720`; each side must be between 200 and 16384. `--clear` goes back to the global default (`agora_window_size` in settings), and without either the game picks its own size. It applies to direct launch; the official launcher keeps the resolution set on its own installation.

`instance hooks` sets a command run in the instance folder before a direct launch (if it fails, the game is not started), a command run after the game exits, and environment variables for the game and both commands. Commands are split on spaces with quotes respected; no shell is used. Both get `INST_NAME`, `INST_ID`, `INST_DIR`, `INST_MC_DIR` and `INST_JAVA`, the same variables Prism Launcher provides. An option replaces only what it names; `--env` replaces all variables; `--clear` removes everything.

`instance notes` keeps free-form text with the instance, in plain text in its manifest. `instance group` files the instance under a group on My Instances. `instance share` keeps game settings (`options.txt`, except resource packs and other per-instance keys) and the server list the same across every instance that shares them: the shared copy is written in before each launch and taken back when the game closes. The first instance to share provides the shared copy.

### Worlds and servers

```text
agora world list <INSTANCE_ID>
agora world rename <INSTANCE_ID> <WORLD_FOLDER> <NAME>
agora world copy <INSTANCE_ID> <WORLD_FOLDER>
agora world delete <INSTANCE_ID> <WORLD_FOLDER> [--yes]
agora world export <INSTANCE_ID> <WORLD_FOLDER> --out <FILE.zip>
agora world import <INSTANCE_ID> <FILE.zip>
agora server list <INSTANCE_ID> [--ping]
agora server add <INSTANCE_ID> <ADDRESS> [--name <NAME>]
agora server remove <INSTANCE_ID> <POSITION>
```

A world is named by its folder under `saves/`, which `world list` shows. `world rename` changes the name shown in game and keeps the folder, so data pack choices and `launch --world` keep pointing at it. `world delete` moves the world to the system recycle bin and asks first unless you pass `--yes`. `world import` accepts a zip holding the world at its root or inside one folder, and picks a free folder name.

`server` edits the game's own multiplayer list (`servers.dat`), keeping the icons and other details the game stored. Positions start at 1, in the order `server list` prints. `--ping` also asks each server whether it is online, with its player count and latency, using the game's own status protocol (Minecraft 1.7 and later). Servers reachable only through a DNS `SRV` record show as offline. Pinging is covered by Lockdown Mode and the Server status switch on the Privacy page.

### Logs, proxy and skins

```text
agora log list <INSTANCE_ID>
agora log share <INSTANCE_ID> [--path logs/latest.log] [--preview] [--yes]
agora proxy show | system | none
agora proxy set <URL> [--username <NAME>]
agora skin show
agora skin wear <FILE.png> [--slim]
agora skin reset
agora skin cape <NAME | none>
```

`log share` uploads the latest game log or a recent crash report to mclo.gs and prints the link. User folder names, anything shaped like a token and the signed-in account's own token are removed first; `--preview` prints exactly what would be sent. It asks before uploading unless `--yes`, and is covered by Lockdown Mode and the Log sharing switch.

`proxy` sets how Agora's own requests reach the internet: `system` uses the `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` environment variables (the default), `none` connects directly, and `set` takes an `http`, `https`, `socks5` or `socks5h` URL. A password is read from the `AGORA_PROXY_PASSWORD` environment variable and kept in the operating-system credential store. The change applies the next time Agora starts. Minecraft itself does not use this proxy.

`skin` changes the active Microsoft account's skin and cape, as changing them on minecraft.net would. A skin must be a 64×64 (or 64×32) PNG.

All of these refuse to run while the game has the instance open, because Minecraft rewrites both files when it closes.

### Mods and content

```text
agora mod list <INSTANCE>
agora mod search <QUERY>
agora mod install <PROJECT> <INSTANCE>
agora mod remove <PROJECT> <INSTANCE>
agora mod update <INSTANCE> <ITEM>
agora mod update-all <INSTANCE>
agora mod pin <INSTANCE> <FILE>
agora mod unpin <INSTANCE> <FILE>
agora mod changelog <INSTANCE> <FILE>
agora mod enable <INSTANCE> <FILE> [--with-deps]
agora mod disable <INSTANCE> <FILE>
agora mod worlds <INSTANCE> <FILE> --worlds <A,B> | --all
agora mod sync-datapacks <INSTANCE>
agora mod graph <INSTANCE>
agora mod orphans <INSTANCE>
agora mod why <INSTANCE> <FILE>
agora mod disable-plan <INSTANCE> <FILE>
```

Examples:

```bash
agora mod graph <INSTANCE_ID>
agora mod why <INSTANCE_ID> <FILE>
agora mod enable <INSTANCE_ID> <FILE> --with-deps
```

Common planning options:

| Option | Meaning |
| --- | --- |
| `--dry-run` | Resolve and print the plan without executing |
| `--include-optional <A,B>` | Include named optional dependencies |
| `--exclude-optional` | Exclude all optional dependencies |
| `--replace-conflicts` | Choose replacement for every resolvable conflict |
| `--abort-conflicts` | Abort when any conflict remains |
| `--remove-anyway` | `mod remove` only: remove the file even though an installed mod still requires it |
| `--allow-replace` | Permit replacement of existing files |
| `--skip-health-scan` | Skip the post-operation health gate |

`--replace-conflicts` is broad. Review a dry-run first, especially on an established instance.

`mod remove` stops when the file is a required dependency of another installed mod. `--remove-anyway` accepts that for the file being removed (and nothing else), prints a warning naming the mods that lose their dependency, and leaves the instance with a health alert until the dependency is restored. It cannot be combined with `--abort-conflicts`.

The default source is Agora's curated strategy. Use `--source modrinth` only when the optional Modrinth integration and its network permissions are enabled.

`mod enable` and `mod disable` fail with a clear error and a nonzero exit when the named file does not exist. Confirm the filename with `mod list` if the error is unexpected.

`mod graph` lists every dependency between installed mods that are switched on. `mod orphans` lists dependency-installed mods that no user-installed mod still needs. `mod why` traces one file back to the mod that asked for it. `mod disable-plan` shows which installed mods declare a dependency on a file, so you can see what switching it off would affect; it changes nothing. These four only read the instance.

`mod enable --with-deps` also re-enables the required dependencies of the file that are switched off. It is best-effort, as in the desktop app: a dependency that cannot be re-enabled is reported as a warning, and the file itself is still enabled.

Minecraft only loads data packs from inside a world, so Agora copies enabled data packs into the instance's worlds before each launch and after installs and removals. `mod worlds` limits one data pack to the named world folders (or `--all` to go back to every world), and `mod sync-datapacks` runs the copy now, for example for a world created since the last launch. Agora only removes or replaces data pack files it placed itself.

### Packs, import, and export

| Command | Purpose |
| --- | --- |
| `agora import <PATH>` | Import a supported local pack |
| `agora import --url <MRPACK_URL>` | Download and import a Modrinth pack URL |
| `agora pack install <PATH> <INSTANCE>` | Install an Agora pack manifest into an existing instance |
| `agora pack versions <PACK>` | List a curated registry pack's locked releases, newest first |
| `agora pack curated <PACK> <INSTANCE> --release <VERSION>` | Install a locked release (the instance must be on its Minecraft version and loader) |
| `agora pack curated <PACK> <INSTANCE>` | Install the pack's flexible recipe against the instance's own Minecraft version |
| `agora pack export <INSTANCE> --format mrpack\|agora\|lockfile --out <FILE>` | Export an instance as a Modrinth pack, an Agora pack, or a lockfile |
| `agora pack review <FILE>` | Show what importing a `.mrpack` or `.agora-pack.json` would do and stage the reviewed copy; prints a review id |
| `agora pack import <REVIEW_ID> [--name <NAME>] [--import-anyway]` | Import exactly the copy `pack review` staged |
| `agora import launchers [--root <DIR>]` | List Prism, CurseForge and Modrinth instances that can be imported |
| `agora import plan <KEY>... [--name <NAME>] [--no-settings]` | Show what importing launcher instances would copy; prints a plan fingerprint |
| `agora import run <KEY>... [--fingerprint <HASH>]` | Import launcher instances (items commit independently) |
| `agora backup export <INSTANCE> --out <DIR> [--snapshot <ID>]` | Write a snapshot (a new one unless `--snapshot` is given) to a folder as one zip |
| `agora backup import <FILE> [--instance <INSTANCE>]` | Read a backup zip into an instance as a restorable snapshot |
| `agora backup prune <INSTANCE> [--keep <N>] [--yes]` | Delete snapshots beyond the newest N (default 10) |
| `agora export <INSTANCE> <DEST>` | Export a standalone server environment |
| `agora pack update-preview <INSTANCE> <MRPACK>` | Show what a newer .mrpack would change. Changes nothing |
| `agora pack update <INSTANCE> <MRPACK>` | Apply a newer .mrpack; asks for `--yes` and an answer to every conflict |
| `agora migrate-data --from <PATH>` | Plan migration from an older CLI data root |
| `agora migrate-data --from <PATH> --yes` | Execute the migration |

`pack export` warns when an Agora pack would leave out mods that have no catalog or Modrinth source (a `.mrpack` bundles them). It asks first, or pass `--yes` to export anyway. `pack import` refuses a pack with hard problems whatever the flags; files that cannot be fetched stop it unless `--import-anyway`, which leaves exactly those out. A launcher key that two launchers share is written `prism:KEY`, `curseforge:KEY` or `modrinth:KEY`. A backup import never changes the live instance; restore it with `snapshots restore`. `backup prune` deletes snapshots and, without a terminal, needs `--yes`.

`migrate-data` is a dry-run unless `--yes` is supplied. Read every reported conflict before executing it.

`pack curated` plans first and installs nothing if a mod marked `required` has no build for the target; recommended and optional mods without one are listed as left out. Add `--dry-run` to print the plan without installing. Optional dependencies of the pack's mods are not added, because a CLI run cannot answer that prompt.

`--symlink-saves` makes imported saves depend on the original path. It is not a copy or backup.

### Snapshots

```text
agora snapshots list <INSTANCE>
agora snapshots create <INSTANCE> --label "Before update"
agora snapshots restore <INSTANCE> <SNAPSHOT_ID>
agora snapshots delete <INSTANCE> <SNAPSHOT_ID>
```

A restore replaces tracked instance state. Preserve valuable worlds separately and verify the intended snapshot ID before restoring.

Agora's automatic pre-launch recovery snapshot is optimized for mod, configuration, and layout recovery and does not include `saves/`. Full manual and transactional snapshots use a broader scope, but no local snapshot system should be the only backup for an irreplaceable world.

### Loadouts

```text
agora loadout create <INSTANCE> <NAME>
agora loadout list <INSTANCE>
agora loadout apply <INSTANCE> <NAME> [--others keep|off]
agora loadout update <INSTANCE> <NAME> [--yes]
agora loadout delete <INSTANCE> <NAME>
```

A loadout records the on/off state of mods, resource packs, shader packs, data packs and worlds. Applying one sets each item it records to that state. An enabled mod it does not list (one added since it was saved, say) is a choice: `--others keep` leaves it on and `--others off` switches it off. When there are such mods and no `--others` is given, apply refuses and names them, so nothing changes. Packs and worlds it does not list are left as they are. Apply refuses a locked instance, and an instance whose game is running or starting, and changes nothing in either case. Before switching, apply saves an undo point labelled `before-loadout` (only the newest is kept). `loadout update` saves a profile again from the instance's current setup, so a mod added since it was saved stops being unlisted. It lists what would change (added, turned on or off, or dropped because it is gone) and asks first on an interactive terminal. Without a terminal it changes nothing and names the changes; pass `--yes` once the person using this computer has agreed. It does not touch the instance, so a locked instance or a running game does not block it.

A loadout does not revert mod versions or configuration files.

### Templates

```text
agora template list
agora template create <NAME> [--from <INSTANCE> --include <FILE>...] [--memory <MB>] [--gc <GC>] [--java-path <FILE>] [--jvm-args <ARGS>] [--description <TEXT>]
agora template apply <TEMPLATE_ID> <INSTANCE>
agora template delete <TEMPLATE_ID> [--yes]
```

Examples:

```bash
agora template create "Sodium options" --from <INSTANCE_ID> --include config/sodium-options.json
agora template create "Big heap" --memory 8192 --gc low_latency
agora template apply <TEMPLATE_ID> <INSTANCE_ID>
```

A template keeps chosen configuration files and JVM settings so you can copy them to another instance. Only the files named with `--include` are captured, by path relative to the source instance; `--include` needs `--from`. A template with no files and no JVM setting is refused. Applying a template copies its files over the same paths in the target instance, and takes an undo point first when it copies any. A JVM setting the template does not name is left alone. Deleting a template asks first; without a terminal, pass `--yes`.

### Mod bisect

```text
agora bisect start <INSTANCE> [--suspect <FILENAME>]
agora bisect status <INSTANCE>
agora bisect apply <INSTANCE>
agora bisect good <INSTANCE>
agora bisect bad <INSTANCE>
agora bisect back <INSTANCE>
agora bisect cancel <INSTANCE>
agora bisect run <INSTANCE> [--suspect <FILENAME>]
```

Bisect finds the mod that breaks a launch by turning half of the enabled mods off for each round. `apply` switches the round's mods off and leaves the others on. Mods that depend on a switched-off mod go off with it, so a missing library is not mistaken for the culprit. You launch the game yourself, then `good` (the problem did not happen) or `bad` (it did) narrows the suspects. `back` undoes the last answer and tries the other half. `cancel` ends the bisect and puts every mod back the way it was, at any point. `--suspect` names a mod the crash report implicated and tests it first; it only applies when starting. `run` walks through the same rounds with prompts and needs an interactive terminal. A bisect does not start the game itself; you launch it. The session is kept in the instance folder, so it survives closing the terminal. `start`, `status`, `good`, `bad` and `back` do not move any mod files; `apply` and `cancel` do. `start` and `apply` refuse a locked instance, and `start`, `apply` and `cancel` refuse a game that is running or starting, changing nothing. `cancel` still restores a locked instance, because putting the baseline back is the way out. If `run` is stopped with Ctrl+C, the mods stay as the current round left them until `agora bisect cancel` is run.


### Reproduction lockfiles

```text
agora lockfile export <INSTANCE> --out instance.lock.json
agora lockfile verify instance.lock.json
agora lockfile repair <INSTANCE> --out repaired.lock.json
agora lockfile import instance.lock.json <INSTANCE>
```

A lockfile describes reproducible artifact state. It is not a backup of private world or configuration contents.

Review a lockfile's source before importing it. Hashes prove byte identity, not safety or licensing.

A lockfile exported from a vanilla instance passes `lockfile verify` immediately; export-then-verify is a valid round trip for vanilla and modded instances alike.

### Java runtimes

```text
agora runtime list
agora runtime inspect <JAVA_PATH>
agora runtime ensure <MAJOR>
agora runtime remove-unused
```

Examples:

```bash
agora runtime ensure 21
agora runtime inspect "C:\Program Files\Java\jdk-21\bin\java.exe"
```

Use `instance recommend-memory` to inspect Agora's memory estimate without changing settings.

### Loader profiles

```text
agora loader list
agora loader list --mc-version 1.21.1
agora loader install <LOADER> <MC_VERSION> <LOADER_VERSION>
```

Loader installation is restricted to Agora's pinned catalog. `--force` reinstalls a verified profile.

Instance health can identify enabled mods whose loader-version requirements are not satisfied. The desktop app provides the richest interactive candidate-selection flow; the CLI should be used with explicit versions and a follow-up health check.

### Community plugins

```text
agora plugin list
agora plugin status                         # alias for list
agora plugin preview package <PATH>
agora plugin preview development <PATH>
agora plugin install package <PATH>
agora plugin install development <PATH>
agora plugin enable <PLUGIN_ID>
agora plugin disable <PLUGIN_ID>
agora plugin remove <PLUGIN_ID>
agora plugin remove <PLUGIN_ID> --purge-data
agora plugin log <PLUGIN_ID> --lines 200
agora plugin disable-all
agora plugin settings <PLUGIN_ID>
agora plugin set <PLUGIN_ID> <KEY> <VALUE>
agora plugin check-update [PLUGIN_ID]
agora plugin update <PLUGIN_ID>
agora plugin update <PLUGIN_ID> --yes
agora plugin keygen --out <PATH> --key-id <ID>
agora plugin sign <DOCUMENT> --key <PATH> --key-id <ID>
```

`plugin settings` prints the settings a plugin declares, with their types and current values. `plugin set` writes one of them. The value is read as JSON (`true`, `42`, `"text"`); text that is not valid JSON is taken as a plain string. Core checks the value against the type the plugin declared and refuses a wrong type or an undeclared key, so nothing is stored in that case.

`plugin list` reads the core-resolved install state without starting plugins. Its status column explains why an installed plugin is not runnable, including disabled plugins, incompatible host APIs, dependency problems, cycles, and recorded activation failures. `plugin status` is a convenience alias.

`plugin preview` validates a package or development folder and prints the same install preview used by the install flow. It does not install or activate anything. A package is copied into Agora's managed plugin directory; a development folder is registered in place so edits remain visible to the author.

An install that requests capabilities prints each required or optional capability, its user-facing explanation, and whether it can change data, then asks for consent. Answer `y` or `yes` to continue. `--yes` skips this prompt for a deliberately scripted invocation:

```bash
agora plugin install package ./example-plugin.zip --yes
agora plugin install development ./example-plugin --yes
```

Without `--yes`, a non-interactive invocation cannot grant capabilities. The CLI shows the preview and passes a negative consent answer to core, so `PluginService` refuses the install. This keeps scripting fail-closed; do not use `--yes` unless the package's preview is trusted. In JSON mode, a pending interactive preview and prompt are written to standard error; the completed plugin summary is the single JSON value on standard output.

### Content providers

```text
agora provider list
agora provider enable <PROVIDER_ID>
agora provider disable <PROVIDER_ID>
agora provider search <PROVIDER_ID> [QUERY] --content-type pack
agora provider plan-digest <PROVIDER_ID>:<PROJECT_ID>@<VERSION_ID>
```

`provider list` shows the official providers (Modrinth, Technic) and every plugin provider, with why one is unusable right now. Enabling or disabling a plugin's provider enables or disables that plugin. `plan-digest` resolves one pack version and prints the digest a curator pins as `sha256` for a `provider_pack` catalog entry (see `REGISTRY_CURATION_REFERENCE.md`).

```text
agora pack install --provider <PROVIDER_ID> --project <PROJECT_ID>
agora pack install --provider <PROVIDER_ID> --project <PROJECT_ID> --version <VERSION_ID> --name <NAME>
agora technic search <QUERY> --limit 20
agora technic install <SLUG>
agora technic install <SLUG> --build <BUILD> --name <NAME>
agora technic install <SLUG> --minecraft <VERSION> --loader <LOADER> --loader-version <VERSION> --sha256 <HEX>
```

`pack install --provider` installs a provider's modpack as a **new** instance, through the same registry the desktop uses. It cannot be combined with a manifest path and instance. `--version` is optional; without it the provider chooses.

Technic consent is off by default. It is stored as the `technic_enabled` setting, and zip packs also need `allow_unverified_packs`; `agora settings set technic_enabled true` changes it. `technic search` and `technic install` are refused while the setting is off. Solder packs install at their recommended build unless `--build` names one. Zip packs carry no metadata, so you must give their Minecraft version, loader, and loader version. Pass `--sha256 <HEX>` (64 hex digits) to check the zip download against a published SHA-256; the download fails if it does not match. Without `--sha256` the CLI warns that the zip is installed unverified, and it does so before the install starts. `--sha256` is refused for Solder packs. Core checks both settings on every call, whatever flags you pass.

### Updates

```text
agora instance updates <INSTANCE>
agora instance updates <INSTANCE> --apply
agora instance updates <INSTANCE> --apply --yes
agora mod pin <INSTANCE> <FILE>
agora mod unpin <INSTANCE> <FILE>
agora mod changelog <INSTANCE> <FILE>
agora pack update-preview <INSTANCE> <MRPACK>
agora pack update <INSTANCE> <MRPACK> --yes
```

`instance updates` checks every installed item against its source and lists what is newer, the same check the desktop's update panel runs, and it stores the result for the instance badge. Rows that are only notes (a newer build for another Minecraft version) or whose release date is unknown are listed but never applied. `--apply` plans the applicable updates as one batch with one snapshot and full rollback, and prints that plan. Nothing changes until you add `--yes`.

`mod pin` keeps one installed file out of update checks, and `mod unpin` lets it be offered again. Pins are stored in the instance manifest. `mod changelog` prints what changed between the installed file and its pending update, from the signed registry. It reports when there is no newer version or when the source published no changelog.

`pack update-preview` compares an .mrpack against the instance and lists the changes, the downloads, and every conflict, without changing anything. `pack update` applies the same merge. It refuses unless every conflict has an answer: `--keep-ours <KEY>` keeps the instance's file and `--take-theirs <KEY>` adopts the pack's file, and each key must be one the preview listed. It also refuses without `--yes`.

`plugin check-update` fetches a plugin's signed update document and reports what it means. It downloads only the metadata, never a package. With no plugin id it checks everything installed, and reports plugins with no update source rather than skipping them silently. The result distinguishes cases that a single "up to date" would blur together: a newer release that needs a newer Agora is reported as exactly that, and an installed version ahead of what the publisher lists is reported rather than downgraded.

`plugin update` applies what the publisher is offering. It re-checks rather than trusting an earlier result, downloads the package, verifies it against the size and SHA-256 in the signed document, and then goes through the same install path a manual install uses. A release asking for capabilities or network hosts the installed version was not granted stops and prints what is new; `--yes` accepts that. Development folders are never updated.

Both require the plugin system to be on. Nothing contacts a publisher until you ask it to, or until you turn on automatic checking, which is off by default.

### Publishing

`plugin keygen` and `plugin sign` are author tooling. The launcher never holds a private key.

```bash
agora plugin keygen --out ./signing.key --key-id 2026-09
```

writes an Ed25519 private key and prints the public half as a ready-to-paste `agora-plugin-update.json`. It refuses to overwrite an existing file, because a signing key cannot be regenerated and overwriting one destroys the only copy. Keep it out of your repository: losing it means you cannot ship updates to existing installs, and nothing can restore that.

```bash
agora plugin sign ./updates.json --key ./signing.key --key-id 2026-09
```

signs an update document in place. The document does not need a `signatures` block beforehand — requiring a valid signature in order to produce one is a requirement nobody can meet — but everything else about it is validated first, so a bad release fails here rather than at every user who later fetches it.

See `docs/plugins/publishing.md` for what the signature does and does not prove.

`plugin enable` and `plugin disable` change only the installed plugin's enabled state. `plugin remove` removes the plugin and its log while keeping plugin data by default; add `--purge-data` only when that separate data-removal decision is intended. Removing a development plugin never deletes its source folder. `plugin log` reads the requested number of recent lines, defaulting to 200. `plugin disable-all` is the recovery path: it stops and disables every currently enabled plugin without removing installations or stored data.

### Launching

```bash
agora launch <INSTANCE>
agora launch <INSTANCE> --timings
agora launch <INSTANCE> --yes
agora launch <INSTANCE> --world <WORLD_FOLDER>
agora launch <INSTANCE> --server <HOST[:PORT]>
```

`--world` opens a world as soon as the game starts (Minecraft 1.20 or newer), and `--server` joins a server (any version that can join servers from the command line). They cannot be combined.

The CLI writes the total session duration to standard error. `--timings` adds individual launch-phase durations there as well.

`--yes` bypasses the interactive health confirmation. It does not make an unhealthy instance safe and should not be used as a generic automation default.

The CLI launches directly through Agora core. Microsoft authentication is therefore required for normal online identity.

### Microsoft authentication

```text
agora auth login
agora auth login --no-browser
agora auth status
agora auth list
agora auth switch <USERNAME_OR_UUID>
agora auth logout [--account <USERNAME_OR_UUID>]
```

These commands manage the Microsoft/Xbox/Minecraft identity used for direct launch. They are separate from the GitHub account used by desktop governance features.

`auth login` uses the Microsoft device-code flow: it prints a short code and a verification URL, opens the URL in your browser unless `--no-browser` is given, and then waits until you finish signing in. Press Ctrl-C to cancel. The device code and every token stay inside Agora; nothing is pasted back into the terminal.

Sessions created before Agora moved to its own Microsoft application cannot be renewed. `auth status` reports those as `sign_in_required`; run `auth login` once to restore direct launch. Instances and other accounts are unaffected.

More than one account can be signed in. `auth login` with a different account adds it and makes it the active one; the previous account stays signed in. `auth list` marks the active account with `*`, and `auth switch` changes which one launches use. `auth logout` signs out the active account, and the next signed-in account becomes active; `--account` signs out a particular one.

Authentication is not scoped by `--data-dir`. `auth logout` removes the shared operating-system credential entry and can sign the desktop app out even when the CLI uses a disposable data root.

### Settings

```text
agora settings list
agora settings get <KEY>
agora settings set <KEY> <VALUE>
```

`settings set` parses the value as JSON and falls back to a string. Quote strings explicitly in scripts when the distinction matters:

```bash
agora settings set launch_mode '"direct"'
agora settings set always_pre_touch false
```

Unknown or internal setting keys may change between releases. Prefer dedicated commands when one exists.

### Crash investigation

```text
agora crash list <INSTANCE>
agora crash inspect <INSTANCE> <FILE>
agora crash investigate <INSTANCE>
agora crash investigate <INSTANCE> --file extra-log.txt
```

`--file` may be repeated. Review logs for local paths, usernames, server addresses, tokens, or private chat before sharing output.

### Guide for AI agents

```bash
agora skill
```

Prints, in the SKILL.md format agent tools read, how an AI agent should use this CLI: Agora's vocabulary, house rules (ask before changing anything, snapshot and dry-run first, never pass `--yes` on the person's behalf), the crash-diagnosis workflow, and what the exit codes mean. The text is compiled into the binary, so it matches that binary's commands, and printing it opens no Agora data. Save the output as `SKILL.md` in an agent's skills folder to keep it loaded, or ask the agent to run the command itself. The player-facing walkthrough is the [AI assistant task guide](https://agoramc.com/docs/guides/ai-assistant).

Commands that stop for consent in a non-interactive session (`launch` on a red health check, plugin capability grants, plugin data restores) say that `--yes` should be passed only once the person using the computer has agreed.

### MCP over standard input/output

```bash
agora mcp serve --stdio
```

The CLI MCP transport uses standard input/output. It is different from the desktop application's optional authenticated localhost server.

When an external client launches this command:

- reserve standard input and output for JSON-RPC;
- do not wrap it in a shell that injects banners;
- keep the process local;
- use Agora's approval and instance-locking controls for modifying tools.

The stdio transport reads one JSON-RPC request per line and writes one response per line. Notifications do not receive a response. Diagnostics remain on standard error so standard output stays protocol-only.

## Structured output

Examples:

```bash
agora --output json paths
agora --output json health <INSTANCE_ID>
agora --output json registry status
```

Do not parse human tables. For automation:

1. select JSON explicitly;
2. check the process exit code;
3. capture standard error separately;
4. tolerate additive JSON fields;
5. never treat a printed success message as sufficient when the exit code is nonzero.

On a command failure in JSON mode, standard error contains an envelope shaped like:

```json
{
  "error": "Instance 'missing' not found",
  "exitCode": 1
}
```

Standard output remains reserved for a successful result. Planning failures can add a structured status envelope; always compare the embedded exit code with the process exit code.

## Exit codes

The CLI maps many core failures to stable semantic ranges.

| Code or range | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Generic or unclassified failure |
| `2` | Command-line usage error |
| `7` | Minecraft process classified as a crash |
| `10` | Local-state or database failure |
| `11` | Instance locked or profile missing/corrupt |
| `12` | Instance creation failure |
| `13` | Catalog missing, invalid, or unsupported |
| `20` | Offline |
| `21` | Download or catalog-download failure |
| `30–34` | Integrity, trust, archive, or disk failure |
| `40` | Authentication required or expired |
| `50` | Feature disabled or unavailable |
| `60–62` | Official-launcher, version/loader, or Java failure |
| `70–72` | Dependency, decision, or migration failure |
| `80–82` | MCP rate limit, denial, or authentication failure |
| `90–94` | Network-policy denial |
| `100+` | Process identity or process-state failure |

Scripts should still inspect structured error output because several distinct errors share a range.

Most ordinary lookups for a nonexistent instance currently return generic exit code `1`, not `11`.

## Support checklist

When reporting a CLI problem, include:

```text
Agora version:
Operating system:
Exact command with secrets removed:
Exit code:
Standard output:
Standard error:
Output of `agora paths` with private usernames redacted:
Instance Minecraft and loader versions:
Whether the desktop app was open:
```

Do not attach the whole data directory. Start with the smallest relevant output.
