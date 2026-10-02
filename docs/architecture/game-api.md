# Game contract and manifest v3

This is the first slice of [multi-game support](../../.kilo/plans/MASTER_SPEC.md#26-multi-game-support).
Minecraft has not moved out of core. There is no package registry, host implementation, store
discovery, base provisioning or VFS in this slice. The contract remains experimental until
Minecraft and a tracer from another family exercise it, and the write-isolation spike completes.

[`agora-game-api`](../../crates/agora-game-api/src/lib.rs) contains data and object-safe async
`GamePackage`/`GameHost` interfaces. Its only direct dependencies, including test/build/target
dependencies, may be `serde`, `semver` and `thiserror`. Packages describe behaviour; core owns
policy, paths, network checks, archive/FOMOD/load-order algorithms and process/transaction lifetimes.
Host operations use typed requests and managed identifiers; declaring a path or tool does not grant
permission to access or execute it. A future script bridge must expose these same operation names.
Launch arguments/environment can contain literals, managed paths or platform-separated path lists,
so Redirect folders and Java classpaths need no unspecified string interpolation language. Named
user-data roots also cover logs, profile files and saves outside a store install.
The scoped `Runtime` root names the selected base/install as visible through deployment, so a
declarative recipe does not need an install ID before discovery. `Artifact` names a downloaded file
without exposing its cache path, including jars needed on Minecraft's classpath.

Runtime versions are opaque exact strings, not implicitly semver: Minecraft snapshots, four-part
executable versions and store build identifiers must survive unchanged. Constraints select a game,
stores, versions and optional builds; constraints are alternatives, their fields conjunctive.
Semver comparison is available explicitly for games that use it. Game/store identifiers are open
so adding community packages does not require expanding an enum in core.

Layers are lowest-first references with typed ownership, whiteouts and generated input fingerprints
(including `Unknown` for imported output). Content priority and plugin load order are separate.
Only writable and staging layers are VFS write targets; successful tool staging is promoted by core,
never edited into an existing generated layer. These descriptions do not commit to a VFS backend.
Journaled swaps supplement Redirect for fixed per-user files; they do not deploy a game's mods.

## Manifest compatibility

The [model](../../crates/agora-core/src/models.rs) writes version 3 through the
[canonical helper](../../crates/agora-core/src/helpers.rs). The disk document contains `game`,
`runtime_identity`, `base`, `frameworks`, `layers`, and a `minecraft` section holding the existing
Minecraft version/loader and content lists. Common identity, provenance and preferences stay at the
top level. Existing Rust callers and IPC still see the old Minecraft fields; custom decoding accepts
both representations, including archived manifests. Only the disk encoding moves those fields.
While Minecraft remains in core, its section is authoritative for the adapter's version/loader
fields: the generic runtime and primary loader framework description are refreshed from those
fields on read/write. Other games retain their generic descriptions directly. This compatibility
projection avoids changing existing launch and install callers in the manifest slice.

Migration is lazy, retaining the existing read/heal/write lifecycle. Reading v1/v2 changes no files.
The first write keeps the original exact bytes at `instance_manifest.json.v1.bak` or `.v2.bak`,
publishes a synced temporary backup, then writes and syncs a separate temporary manifest and renames
it into place. Re-running a completed upgrade without edits preserves the current bytes and mtime.
A conflicting existing backup causes an error and preserves both originals. Abandoned temporary
files are ignored on restart; they are never automatically promoted.

Minecraft migrates to the `mojang` runtime source, preserving the exact Minecraft version, with no
invented build or pinned base. Vanilla has no framework; another loader records its existing version
without inventing compatibility evidence. `InstanceContent` layers reference the existing `mods`,
`resourcepacks`, `shaderpacks`, `datapacks` and `saves` directories. They are legacy Redirect content,
not a claim that these mutable directories are already in an immutable content store. Migration
does not inspect, copy, hash, create or change any instance content.

The v1 pack-management backfill still applies only to versions below 2, independently of the current
schema number, so a v2 user's deliberate override survives. Unknown top-level and Minecraft-section
fields survive writes. A future manifest is opened read-only without healing: even unknown enum
variants/types can yield an identity summary, with the entire document retained in memory. Writes
check both the supplied object's read-only marker/version and the live disk version, protecting
against an old object overwriting a newer file. This does not provide cross-process concurrent
editing; callers retain their existing instance locking responsibilities.

## Verification

Contract tests cover opaque/store/build matching, explicit semver matching and layer write ownership;
interface object safety is checked at compile time. Manifest helper tests exercise v1/v2 migration,
exact backups, unchanged content, interruption before and after replacement, retry/idempotency,
backup publication, backup conflicts, and future schemas (including unfamiliar types). Declarative
package fixtures also round-trip Skyrim/VFS and Valheim/Redirect definitions, runtime constraints,
tools, and host-resolved launch paths. The architecture rule fixtures
are in [`test_check_architecture.py`](../../scripts/test_check_architecture.py); run them with
`python -m unittest discover -s scripts -p test_check_architecture.py`.

The spec says the three-dependency restriction is “like agora-plugin-api”, but that crate currently
also depends on `serde_json`. This slice follows the explicit three-dependency requirement for the
new crate without changing the existing plugin contract.
