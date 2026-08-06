# Agora CLI reference

The `agora` binary exposes the same core instance, registry, health, launch, snapshot, runtime, and install services used by the desktop application. It is intended for advanced users, support diagnostics, scripting, and local AI/MCP integrations.

The CLI can modify the same data used by the desktop application. Read the safety section before experimenting.

## Build or install

From the repository root:

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
- Do not run desktop and CLI mutations against the same instance at the same time.
- Use `--dry-run` on install, remove, and update operations before executing a complex plan.
- Create a snapshot before manual or high-risk changes.
- Use disposable instances when learning destructive commands.
- `instance delete`, `snapshots restore`, `snapshots delete`, `lockfile import`, and some install conflict choices can replace or remove files.
- Never place access tokens or account codes in command history, log files, or bug reports.
- `--data-dir` isolates files under the Agora data root, but Microsoft credentials use the operating-system credential store. `auth status`, `auth login`, and `auth logout` therefore inspect or change the same credential entry used by the desktop app.

## Global syntax

```text
agora [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS]
```

Global options:

| Option | Purpose |
| --- | --- |
| `--data-dir <PATH>` | Override the Agora data root |
| `--json` | Shorthand for `--output json` |
| `--output <human|json>` | Select output format |
| `--registry-repo <OWNER/REPO>` | Override the registry repository for development or testing |
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

Synchronize the signed registry:

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

Search the curated registry:

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

Check health and launch:

```bash
agora health <INSTANCE_ID>
agora launch <INSTANCE_ID> --timings
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

### Registry

| Command | Purpose |
| --- | --- |
| `agora registry status` | Inspect cached registry and active catalog state |
| `agora registry sync` | Download and verify the latest signed registry |
| `agora sync` | Convenience alias for registry synchronization |

Use `--registry-repo` only for an intentional development or sandbox registry. A different repository changes the trust and governance boundary.

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
```

Examples:

```bash
agora instance recommend-memory <INSTANCE_ID>
agora instance lock <INSTANCE_ID>
agora instance clone <SOURCE_ID> "Debug Copy" --no-saves
```

Clone flags can omit saves, mods, resource packs, shader packs, screenshots, configuration, `servers.dat`, or options. Hard-link and symlink modes change filesystem behavior and should be used only when the destination and source relationship is understood.

Locking prevents ordinary content mutation. Unlock explicitly before applying intended changes.

### Mods and content

```text
agora mod list <INSTANCE>
agora mod search <QUERY>
agora mod install <PROJECT> <INSTANCE>
agora mod remove <PROJECT> <INSTANCE>
agora mod update <INSTANCE> <ITEM>
agora mod update-all <INSTANCE>
agora mod enable <INSTANCE> <FILE>
agora mod disable <INSTANCE> <FILE>
```

Common planning options:

| Option | Meaning |
| --- | --- |
| `--dry-run` | Resolve and print the plan without executing |
| `--include-optional <A,B>` | Include named optional dependencies |
| `--exclude-optional` | Exclude all optional dependencies |
| `--replace-conflicts` | Choose replacement for every resolvable conflict |
| `--abort-conflicts` | Abort when any conflict remains |
| `--allow-replace` | Permit replacement of existing files |
| `--skip-health-scan` | Skip the post-operation health gate |

`--replace-conflicts` is broad. Review a dry-run first, especially on an established instance.

The default source is Agora's curated strategy. Use `--source modrinth` only when the optional Modrinth integration and its network permissions are enabled.

Known defect: `mod enable` and `mod disable` currently return success when the named file does not exist. Confirm the filename with `mod list` and verify the resulting state rather than trusting the success line alone.

### Packs, import, and export

| Command | Purpose |
| --- | --- |
| `agora import <PATH>` | Import a supported local pack |
| `agora import --url <MRPACK_URL>` | Download and import a Modrinth pack URL |
| `agora pack install <PATH> <INSTANCE>` | Install an Agora pack manifest into an existing instance |
| `agora export <INSTANCE> <DEST>` | Export a standalone server environment |
| `agora migrate-data --from <PATH>` | Plan migration from an older CLI data root |
| `agora migrate-data --from <PATH> --yes` | Execute the migration |

`migrate-data` is a dry-run unless `--yes` is supplied. Read every reported conflict before executing it.

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
agora loadout apply <INSTANCE> <NAME>
agora loadout delete <INSTANCE> <NAME>
```

A loadout records enabled state. It does not revert mod versions or configuration files.

### Reproduction lockfiles

```text
agora lockfile export <INSTANCE> --out instance.lock.json
agora lockfile verify instance.lock.json
agora lockfile repair <INSTANCE> --out repaired.lock.json
agora lockfile import instance.lock.json <INSTANCE>
```

A lockfile describes reproducible artifact state. It is not a backup of private world or configuration contents.

Review a lockfile's source before importing it. Hashes prove byte identity, not safety or licensing.

Known defect: a lockfile exported from a vanilla instance can contain an empty loader version, and the current `lockfile verify` command rejects that export as incomplete. Do not present export-then-verify as a working vanilla round trip until the product bug is fixed.

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

### Launching

```bash
agora launch <INSTANCE>
agora launch <INSTANCE> --timings
agora launch <INSTANCE> --yes
```

The CLI writes the total session duration to standard error. `--timings` adds individual launch-phase durations there as well.

`--yes` bypasses the interactive health confirmation. It does not make an unhealthy instance safe and should not be used as a generic automation default.

The CLI launches directly through Agora core. Microsoft authentication is therefore required for normal online identity.

### Microsoft authentication

```text
agora auth login
agora auth status
agora auth logout
```

These commands manage the Microsoft/Xbox/Minecraft identity used for direct launch. They are separate from the GitHub account used by desktop governance features.

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
| `13` | Registry missing, invalid, or unsupported |
| `20` | Offline |
| `21` | Download or registry-download failure |
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
