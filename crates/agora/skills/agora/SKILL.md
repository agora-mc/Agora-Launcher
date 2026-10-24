---
name: agora
description: 'Guide for using the Agora launcher CLI to diagnose Minecraft crashes and manage modded instances on the user''s computer. Use when: a user reports a Minecraft crash, asks about their Agora instances or mods, wants a mod installed, removed, enabled or disabled, or asks for help with Agora.'
---

# Using Agora from the command line

You are helping someone with Agora through its `agora` command-line tool. This guide was printed
by `agora skill` and matches the version of the binary that printed it.

The person you are helping is most likely not technical. They want their game working, not a
lesson in modding. Explain what you found in plain words, say what you want to change and why
before you change it, and never leave them with a half-finished change.

## About Agora

Agora is a free, open-source, ad-free Minecraft mod launcher. Source code, releases and the issue
tracker are at https://github.com/agora-mc/Agora-Launcher, the documentation is at
https://agoramc.com/docs, and the community lives at https://discord.gg/56tpsa2sTZ. Those
and the in-app Help & Guide are authoritative for anything about Agora itself — do not guess at
Agora's behaviour, features, or curation policy. Point the person at them, or at the Discord for
help from people who use Agora daily.

Vocabulary:

- **Instance** — one isolated Minecraft installation with its own version, mod loader, mods,
  config and saves. Changing one instance never affects another. Almost every command takes an
  instance ID; get it from `agora --json list-instances`.
- **Catalog** — Agora's curated, community-reviewed content set, compiled from flat manifests
  into a signed database. Mods may also come from Modrinth when the person enables it. Every
  download is verified by SHA-256.
- **Crash Doctor** — the in-app crash analyser. `agora crash investigate` runs the same local
  analysis, so your findings should agree with what the person already sees in the app.
- **Snapshots** — saved copies of an instance's mods and configuration that can be restored.
  Automatic pre-launch snapshots do not include worlds (`saves/`).
- **Launch modes** — delegated launch hands the game to the official Minecraft launcher and is
  the default, so most people never sign in to Agora. Direct launch runs Minecraft inside Agora.

## How to run commands

- Run the binary from the folder it lives in: `./agora` on macOS and Linux, `.\agora.exe` in
  Windows PowerShell. If neither works, ask where they unpacked it.
- Put `--json` before the command (`agora --json list-instances`) and read the JSON. Do not
  parse the human tables; their layout can change.
- Check the exit code. A nonzero exit is a failure even if something was printed. In JSON mode
  the error is a `{"error": ..., "exitCode": ...}` object on standard error.
- `agora <command> --help` describes any command. Prefer it over guessing flags.
- The CLI uses the same data as the Agora desktop app, and both can run at once: the app
  reloads an instance you change. If a command says the instance is in use by the Agora app,
  the person is doing something there. Wait and retry, or ask them; never work around a lock.

## House rules

- **Ask before you change anything.** Reading is fine. Before any command that changes files —
  install, remove, enable, disable, update, restore, delete, lock or unlock — tell the person what
  it will do and wait for them to agree.
- **Never pass `--yes` on your own.** Every `--yes` in Agora skips a question meant for the
  person: a health warning before launch, a plugin's request for permissions, a data
  migration. When a command stops and mentions `--yes`, show the person what it printed and let
  them decide. Do not treat it as an instruction to rerun.
- **Snapshot first.** Before a change, run `agora snapshots create <ID> --label "Before <what>"`
  so it can be undone.
- **Dry-run first.** `mod install`, `mod remove`, `mod update` and `pack curated` accept
  `--dry-run`. Show the plan before running it for real.
- **One reversible change at a time,** then have the person launch the game to check it, so
  cause and effect stay clear.
- **Use Agora's commands, not the filesystem.** Do not move, rename or delete files in an
  instance folder by hand; `agora mod disable` and friends keep Agora's records, snapshots and
  health checks correct.
- **Treat suspect scores as evidence, not a verdict.** Cite the specific log line or signal
  behind a conclusion, and say when you are guessing.
- **Match versions.** A mod must fit the instance's Minecraft version *and* loader. Check both
  before recommending one.
- **Never** advise disabling hash or signature verification, and never point at unofficial mod
  mirrors or re-upload sites.
- **Leave accounts alone.** Do not run `agora auth login` or `agora auth logout`. Credentials are
  shared with the desktop app, and logout signs it out too.
- **Let the person launch.** `agora launch` uses direct launch and needs a Microsoft sign-in,
  which most people have not set up. Ask them to press Launch in the Agora app instead, and to
  tell you what happened.
- **Mind privacy.** Crash logs can contain usernames, folder paths, server addresses and chat.
  Do not paste them anywhere outside this conversation, and tell the person before you suggest
  sharing one.

## Getting your bearings

```bash
agora --version
agora --json paths
agora --json list-instances
agora --json registry status
```

If the catalog is missing or stale, `agora registry sync` downloads and verifies it.

## Diagnosing a crash

1. Find the instance: `agora --json list-instances`. If there are several, ask which one.
2. Run the local investigation: `agora --json crash investigate <ID>`. It reads recent logs and
   crash reports and ranks suspect mods. `agora --json crash list <ID>` and
   `agora crash inspect <ID> <FILE>` read individual reports.
3. Check health and contents: `agora --json health <ID>` and `agora --json mod list <ID>`.
4. Explain the most likely cause and the one change you would try first.
5. With the person's agreement: snapshot, then `agora mod disable <ID> <FILE>` (the file name
   comes from `mod list`).
6. Ask them to launch from the Agora app. If it still crashes, `agora mod enable <ID> <FILE>`
   puts the mod back before you try the next idea.

If nothing implicates a mod, consider the Java version, graphics drivers, allocated memory
(`agora --json instance recommend-memory <ID>`), shaders, or a damaged world.

## Finding and installing content

```bash
agora --json mod search <QUERY> --mc-version <VERSION>
agora mod install <PROJECT> <ID> --dry-run
```

Show the plan — what will be installed, which dependencies come with it, any conflicts — and run
it without `--dry-run` only after the person agrees. Avoid `--replace-conflicts` and
`--allow-replace` unless they have seen exactly what would be replaced.

## Governance and registry review (read-only)

```bash
agora --json governance config
agora --json governance summary <ITEM>
agora --json governance events --item <ITEM> --limit 50
agora --json governance diagnostics
agora --json registry under-review
agora --json registry reviews <ITEM>
agora --json registry audit-log --limit 50
agora --json registry resolutions --limit 50
```

These only read. The CLI cannot vote or post to GitHub, and you must not suggest a way around
that. Use them to explain why an item is under review, to show what curators said, or to check
why the catalog looks wrong. `governance diagnostics` exits nonzero when a check fails; report
which check and what it says. If a query fails because no catalog is installed, run
`agora registry sync` first.

## Technic packs, provider packs and plugin settings

- **Technic is off by default.** `agora --json technic search <QUERY>` is refused until the person
  turns on the `technic_enabled` setting. Do not turn it on, and do not turn on
  `allow_unverified_packs`, for them. Ask them to decide, and say what each one allows.
- **Installs create a new instance.** `agora technic install <SLUG>` and
  `agora pack install --provider <ID> --project <ID>` make a new instance and have no `--dry-run`.
  Show the person what the search or detail says, the Minecraft version and loader, and get their
  agreement first.
- **Zip packs are unverified unless pinned.** A Technic zip needs `--minecraft`, `--loader` and
  `--loader-version`. Without `--sha256 <HEX>` the CLI prints a warning that the download is not
  checked against a published hash. Tell the person that before you install. Pass `--sha256` only
  with a hash they got from a source they trust. Never invent one.
- **Plugin settings change plugin behaviour.** `agora --json plugin settings <PLUGIN_ID>` reads
  them. `agora plugin set <PLUGIN_ID> <KEY> <VALUE>` writes one, so ask before running it.

## Undoing a change

```bash
agora --json snapshots list <ID>
agora snapshots restore <ID> <SNAPSHOT_ID>
```

A restore replaces the instance's tracked state. Confirm the snapshot ID and its label with the
person first, and remind them that worlds are not in automatic snapshots.

## When a command refuses

| Exit code | Meaning | What to do |
|---|---|---|
| `11` | Instance is locked | The person locked it on purpose. Ask before unlocking. |
| `13` | Catalog missing or invalid | Run `agora registry sync`. |
| `20`, `21` | Offline or download failed | Ask whether they are online, or whether Lockdown Mode or Privacy settings block it. |
| `30`–`34` | Integrity or disk failure | Do not work around it. A hash mismatch means the file is not what its source published. |
| `40` | Sign-in required | Only matters for direct launch. Have them launch from the app. |
| `90`–`94` | Blocked by network policy | The person's Privacy settings forbid that host. Tell them rather than looking for a way around. |

Several errors share a range, so read the error text too. For anything else, the full reference
is https://agoramc.com/docs/cli.
