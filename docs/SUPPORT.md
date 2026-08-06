# Data, logs, and support evidence

This reference explains how to locate Agora data, identify a build, and collect the smallest useful evidence without copying an entire profile.

## Find the data root

The desktop and CLI use the same core-owned default data root. Ask the CLI to resolve it instead of relying on a platform path copied from documentation:

```bash
agora paths
```

If the CLI was started with `--data-dir` or the desktop was started with `AGORA_DATA_DIR`, run `agora paths` with the same override. The output identifies the root, instances, registry database, runtime caches, snapshots, staging area, and local state database.

For one instance, open its editor and choose **Open in Folder**. For the application data root, choose **Settings > Software Updates > Open application data folder**; it opens the same root that `agora paths` reports. This is safer than reconstructing a platform path by hand.

## Identify the installed version

For the standalone CLI:

```bash
agora --version
```

The desktop shows its exact packaged version in **Settings > Software Updates** in the form `Agora Launcher 0.1.0`. Record it together with the `v*` release from which the package came. **Check for Updates** reports update availability but is not a substitute for the displayed current-version label. For a source build, record the Git commit as well.

## Know which log is which

| Evidence | Location or source | Use |
| --- | --- | --- |
| Agora/CLI diagnostics | CLI stdout and stderr; optionally a file selected with `--log-file` | Command parsing, registry, planning, launch, and host errors |
| Desktop error text | Visible dialogs, notices, and process console | Desktop workflow and direct-launch status; the current app has no general persisted app-log exporter |
| Game log | The selected instance's `logs/latest.log` and, when present, `logs/debug.log` | Minecraft, loader, and mod initialization or runtime errors |
| Crash report | The selected instance's `crash-reports/` or a JVM `hs_err_pid*.log` at the instance root | Structured game crashes and fatal JVM failures |
| Compiler log | Local compiler terminal output or the Nightly Compiler workflow log | Registry-maintainer failures only; it is not stored in a player profile |

Crash Doctor reads a bounded, coherent set from recent crash reports, game logs, and JVM fatal-error logs. It does not make every file in the profile safe to share.

## Collect a minimal support bundle

Agora does not currently generate a one-click support archive. Build a small folder manually and review every file before compressing it:

1. Add a text note with the Agora version or release tag, operating system, exact user journey, instance name, Minecraft version, loader, loader version, launch mode, and reproduction rate.
2. Save the first relevant error and the health finding text.
3. Capture non-mutating CLI context when the CLI is available:

   ```bash
   agora --output json registry status
   agora --output json health <INSTANCE>
   agora crash list <INSTANCE>
   ```

4. Add only the relevant redacted `latest.log`, `debug.log`, crash report, or JVM fatal-error log.
5. State whether the problem reproduces in a disposable clean instance.

Do not include the whole Agora data root, `local_state.db`, `mcp_token`, credential-store exports, saves, server lists, private configuration, or unrelated instances. `agora paths` output contains local paths; redact user-identifying path segments before sharing it.

## Personal-data review

Logs and diagnostic output can contain:

- local account names and filesystem paths;
- Minecraft or GitHub names;
- server addresses and chat text;
- instance and private pack names;
- access tokens, authorization codes, or request headers;
- JVM arguments and environment-derived values;
- mod configuration or crash context that reveals private data.

Remove secrets rather than masking only part of them. When a token may have been exposed, revoke or regenerate it before sharing any cleaned evidence.

## Reset the right layer

Use **Settings > Appearance > Reset appearance** to restore visual preferences. Use **Reset layout** to restore shell/sidebar sizing. Neither action deletes instances, registry data, snapshots, accounts, or Minecraft content.

Agora has no in-app factory-reset action. Resetting application data is a different and destructive operation:

1. close Agora and Minecraft;
2. resolve and record the active root with `agora paths` using the same data-root override;
3. back up valuable worlds and any profile data that must survive;
4. rename the data root rather than deleting it;
5. start Agora and confirm it creates a clean profile before removing any backup.

Microsoft credentials are stored in the operating-system credential store and are not isolated by `--data-dir` or `AGORA_DATA_DIR`. Renaming the data root does not guarantee a sign-out, and `agora auth logout` affects the shared credential entry rather than only a disposable data directory.
