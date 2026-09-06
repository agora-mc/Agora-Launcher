# Troubleshooting Agora

Start with the visible symptom, preserve the current state, and change one variable at a time.

## Before changing anything

1. Close Minecraft.
2. Note the instance name, Minecraft version, loader, and loader version.
3. Create a manual snapshot when the instance is still readable.
4. Back up valuable worlds separately.
5. Preserve the earliest useful error, not only the last line.
6. Avoid deleting files manually until Agora's health and recovery tools have been checked.

## Common symptoms

### The registry is unavailable or cannot be verified

- Confirm the device is online and registry networking is enabled.
- Use the Registry status in Settings or run:

  ```bash
  agora registry status
  agora registry sync
  ```

- A packaged release must contain the expected registry verification key. A signature error should not be bypassed by accepting an unsigned database.
- If a newly packaged build alone fails, include the build identity and whether Browse works in the previous release.

### A loader requirement blocks launch

Agora can inspect loader requirements declared by enabled mods.

- Read the unresolved requirements and affected mods.
- Prefer Agora's recommended signed compatible version when one exists.
- Use **Choose compatible version** to review another signed candidate rather than selecting the newest version blindly.
- **Switch and launch** applies the recommended version in a launch flow; health review uses **Switch version** instead.
- If no signed version satisfies every enabled requirement, Agora explains that automatic switching is unavailable. **Manual candidates** are signed versions whose capabilities remain indeterminate and require confirmation in the instance editor; they are not automatically declared compatible.
- `javafml` and `lowcodefml` are language-provider capabilities supplied by Forge or NeoForge when the active loader exposes the required version. They should not be treated as ordinary missing mod JARs.

After a switch, rerun health before launching.

### Java is missing or incompatible

- Leave Java management on Automatic unless a modpack or controlled test requires otherwise.
- Use the runtime list and inspection tools:

  ```bash
  agora runtime list
  agora runtime inspect <JAVA_PATH>
  ```

- A newer Java major is not automatically compatible with a Minecraft version expecting an exact major.
- Remove per-instance overrides after a test so automatic selection can regain control.

### An install plan requires a decision

Resolve the plan without executing:

```bash
agora mod install <PROJECT> <INSTANCE> --dry-run
```

Review:

- target instance;
- selected artifact;
- required and optional dependencies;
- files to add, replace, or remove;
- conflicts;
- loader or Minecraft-version blockers;
- post-operation health policy.

Do not combine `--replace-conflicts` with an unreviewed large update batch.

### A controller works in Agora but does nothing in Minecraft

This is expected. Minecraft has no built-in gamepad support, so a controller that
navigates Agora will not move your character.

- Install Controlify into the instance. Agora offers this when you launch with a
  controller in use, and you can also install it yourself from Browse.
- Controlify is published for Fabric, Quilt, and NeoForge. On any other loader
  there is no build to install, and Agora will say so rather than offering one.
- If you declined the offer earlier, it is remembered for that instance and will
  not be shown again; install it from Browse instead.

### A controller does not put Agora into handheld mode

- Press a button on the pad while the Agora window is focused. Agora is only told
  a controller exists once you use it, so one sitting connected and idle is not
  enough.
- If you left handheld mode with **B** or **Escape**, Agora deliberately does not
  pull you back in while the pad stays connected. Press **Start**, or unplug and
  reconnect.
- There is no setting to enable. If pressing buttons does nothing at all, confirm
  the controller works elsewhere on the system first.

### Minecraft crashes after launch

- Open Crash Doctor or run:

  ```bash
  agora crash investigate <INSTANCE>
  ```

- Preserve the first exception and nearby stack frames.
- Test one suspect or coherent dependency group at a time.
- Use a copy of the world when world data might be involved.
- A clean static health scan cannot prove runtime stability or world compatibility.

### The app is slow before Java starts

Use launch timings to identify the phase:

```bash
agora launch <INSTANCE> --timings
```

Record whether the delay is in health, resolution, materialization, snapshot, or process start. Compare:

- first launch after reboot;
- immediate second launch;
- first launch after restarting Agora;
- small and large instances.

Do not diagnose the cause from total launch time alone.

### Offline launch fails

Offline readiness is instance-specific.

Before disconnecting:

1. synchronize the registry;
2. launch the exact instance once;
3. ensure its Java runtime is present;
4. ensure game, loader, and mod artifacts are cached;
5. test the intended direct or delegated launch mode;
6. turn off each endpoint group in **Settings > Privacy**, verify the workflow again, and then disconnect.

Microsoft online identity, missing artifacts, updates, and uncached downloads cannot complete offline.

Lockdown Mode in **Settings > Privacy** is a global backend network block: while it is on, every endpoint permission is disabled and Agora sends no feature requests. Use it to force offline behavior, and turn individual endpoints back on when you need finer-grained access than an all-or-nothing block.

## Snapshots and world safety

Agora uses different recovery scopes:

- Automatic pre-launch recovery focuses on mod, configuration, resource, shader, datapack, options, and manifest state. It intentionally excludes `saves/` from the launch-critical scan.
- Full manual and transactional snapshots use a broader tracked scope.
- Loadouts record enabled state only.
- Lockfiles describe reproducible artifacts and settings, not private save contents.

No local snapshot, loadout, or lockfile should be the only backup for an irreplaceable world.

## Collecting useful evidence

See [Data, logs, and support evidence](./SUPPORT.md) for data-root discovery, log types, version identification, minimal support bundles, personal-data review, and reset boundaries.

Include:

- Agora version or test-run label;
- operating system;
- exact user journey;
- instance Minecraft, loader, and loader version;
- whether direct or delegated launch was used;
- first relevant error;
- reproduction rate;
- health finding text;
- whether the issue survives a restart;
- whether it occurs in a disposable clean instance.

CLI helpers:

```bash
agora paths
agora --output json registry status
agora --output json health <INSTANCE>
agora crash list <INSTANCE>
```

Redact:

- Microsoft or GitHub tokens;
- authorization codes;
- local usernames when unnecessary;
- server addresses;
- private chat;
- webhook URLs;
- filesystem paths outside the relevant Agora instance.

## Asking for help

Use the project Discord for user support and GitHub issues for reproducible Agora defects. A useful report explains what Agora did, what was expected, and how another person can reproduce it without receiving private data.
