# Agora Registry Curation Reference

A self-contained reference for an agent who does **not** have access to the Agora repository. With this document alone, you can author valid mod / pack / shader / resource pack / datapack / server / world manifests for the curated registry.

---

## 1. Repository layout (flat-file manifest)

Everything lives under `registry/` in the Agora monorepo. Each content type has its own subdirectory, and one JSON file = one registry entry. The filename (minus `.json`) should match the manifest's `id`.

```
registry/
├── mods/              ← .jar mods
│   ├── sodium.json
│   ├── fabric-api.json
│   └── ...
├── packs/             ← Curated modpacks
│   └── optimized-survival.json
├── shaders/           ← Shader packs
├── resourcepacks/     ← Resource packs
├── servers/           ← Server configs
├── datapacks/         ← Datapacks
├── worlds/            ← Pre-built worlds
├── pack-overrides/    ← (Optional) zip bundles of configs for a pack
├── governance/        ← Cross-cutting policy files (see §5)
│   ├── known_conflicts.json
│   ├── poll_blacklist.json
│   ├── quarantine_decisions.json
│   ├── governance-state.json
│   └── audit_log.json
└── archived/          ← Disabled entries (compiler SKIPS this dir entirely)
```

A nightly compiler (`.github/workflows/compile.yml`) walks the 7 content dirs (`mods`, `packs`, `shaders`, `resourcepacks`, `servers`, `datapacks`, `worlds`) and compiles every `*.json` into a signed SQLite database (`registry.db`) that the desktop + web apps consume. Anything in `registry/archived/` is ignored entirely — that's how you retire an entry without deleting history.

---

## 2. Mod manifest schema (`registry/mods/<id>.json`)

Required fields: `id`, `name`, `content_type`, `author`, `license`, `sha256`, and a statement of
where the file comes from — either `download_sources` (preferred) or the legacy
`download_strategy` + `source_identifier` pair. Other fields are optional or auto-populated.

### Where the file comes from: `download_sources`

An entry declares an **ordered list** of places its file can be fetched from. The launcher walks
the list at install time and uses the first source that is both enabled in the user's Settings and
actually answering. Index 0 is your preference; everything after it is a fallback.

```json
  "download_sources": [
    { "strategy": "modrinth_id", "identifier": "AANobbMI" },
    { "strategy": "github_release", "identifier": "CaffeineMC/sodium" }
  ],
```

This is what makes an entry survive a bad day upstream: GitHub's unauthenticated API allows 60
requests an hour, Modrinth has outages, and a self-hosted mirror can simply be down. With one
source, any of those is a failed install; with a list, it is a slower one. It also gives the user a
real choice — turning a source off in Settings (*Content sources*) is respected at install time,
not just in Browse, and an entry stays installable as long as one of its sources is still enabled.

Rules:

- Each entry is `{"strategy": ..., "identifier": ...}`. `strategy` is one of `github_release`,
  `modrinth_id`, `direct_hash`, `technic_pack`, `curated_pack`; `identifier` is what that strategy
  resolves (repo, project id, or pinned URL — same values as `source_identifier`).
- The list is what the launcher walks, so **every pinned source in it is held to the full
  `direct_hash` contract**, not just the first. A fallback that only fails once the preferred
  source is down would be worse than no fallback at all.
- Additional `direct_hash` sources are *mirrors of the same bytes*. All sources of an entry share
  one `sha256`; a genuinely different file needs its own registry entry.
- `download_strategy` and `source_identifier` may be omitted when `download_sources` is present —
  the compiler derives them from index 0 for the website and for older launcher builds. If you do
  write them, they must match index 0 or the build fails.
- Omitting `download_sources` entirely is still valid: the compiler builds the list from
  `download_strategy` + `source_identifier`, plus `modrinth_id` as an implicit fallback. That is
  exactly what the launcher already did for such entries.

### Full example (Modrinth preferred, GitHub fallback)

```json
{
  "id": "sodium",
  "name": "Sodium",
  "content_type": "mod",
  "author": "CaffeineMC",
  "license": "LGPL-3.0",
  "download_sources": [
    { "strategy": "modrinth_id", "identifier": "AANobbMI" },
    { "strategy": "github_release", "identifier": "CaffeineMC/sodium" }
  ],
  "modrinth_id": "AANobbMI",
  "sha256": "ee9d62778c8b664aa8501af83ec4738e01d20f2cdca133208c7bf66cbcaa37b8",
  "package_signatures": [
    "me.jellysquid.mods.sodium",
    "net.caffeine.sodium"
  ],
  "base_categories": ["optimization", "rendering"],
  "community_categories": ["client-only", "performance-boost", "essentials"],
  "curator_note": "Essential rendering engine replacing legacy OpenGL pipelines. Significantly boosts framerates on nearly all hardware. Incompatible with OptiFine; use Iris for shader support instead.",
  "icon_url": "https://raw.githubusercontent.com/CaffeineMC/sodium/main/assets/icon.png",
  "gallery_urls": [
    "https://raw.githubusercontent.com/CaffeineMC/sodium/main/assets/screenshot1.png"
  ],
  "governance": {
    "immune": false,
    "override_justification": null,
    "allow_comments": true
  }
}
```

### Minimal example (single Modrinth source)

The shortest valid manifest, still using the legacy single-source form — which stays valid, and is
the right shape when there genuinely is only one place to get the file. The compiler queries
Modrinth's API to hydrate icon, gallery, description, body markdown, page URL, license, and
`compatible_versions` automatically.

```json
{
  "id": "xaeros-minimap",
  "name": "Xaero's Minimap",
  "content_type": "mod",
  "author": "Xaero96",
  "license": "LicenseRef-ARR",
  "download_strategy": "modrinth_id",
  "source_identifier": "1bokaNcj",
  "modrinth_id": "1bokaNcj",
  "sha256": "acba53bff782903d64ed8d92fc1f21116830f389c003eb37bc49579980f333bf",
  "package_signatures": ["xaero.minimap"],
  "base_categories": ["utility", "navigation"],
  "community_categories": ["client-only", "minimap", "vanilla-plus"],
  "curator_note": "Lightweight client-side minimap with waypoints. A solid vanilla-plus navigation aid; disable for a purist experience.",
  "icon_url": "https://cdn.modrinth.com/data/1bokaNcj/354080f65407e49f486fcf9c4580e82c45ae63b8_96.webp",
  "gallery_urls": [],
  "governance": {
    "immune": false,
    "override_justification": null,
    "allow_comments": true
  }
}
```

### Closed-source / direct-hash example

For mods that are self-hosted and not on GitHub or Modrinth. The hash is **manually pinned** by the developer; if they silently change the file without a PR, every existing download is blocked for all users.

This is the fully hand-curated escape hatch: **no API resolves any part of it**, so the manifest
must carry everything the launcher needs. That makes `compatible_versions` **required** here (it
is optional for every other strategy, where the compiler hydrates it), and each entry must name a
real `mod_version` — the `"latest"` placeholder is rejected. The compiler fails the build on an
incomplete `direct_hash` entry rather than shipping one that is browsable but uninstallable.

```json
{
  "id": "proprietary-mod",
  "name": "Proprietary Mod",
  "content_type": "mod",
  "author": "Developer Name",
  "license": "LicenseRef-Proprietary",
  "download_strategy": "direct_hash",
  "source_identifier": "https://developer.com/releases/mod-v1.0.0.jar",
  "sha256": "a1b2c3d4e5f6...(64 hex chars)",
  "compatible_versions": [
    { "mc_version": "1.21", "loader": "fabric", "mod_version": "1.0.0" },
    { "mc_version": "1.21.1", "loader": "fabric", "mod_version": "1.0.0" }
  ],
  "package_signatures": ["com.developer.mod"],
  "base_categories": ["content"],
  "community_categories": [],
  "curator_note": "",
  "governance": {
    "immune": false,
    "override_justification": null,
    "allow_comments": true
  }
}
```

Rules the compiler enforces for **every** `direct_hash` source an entry declares — the preferred
one and any pinned mirror listed after it:

| Requirement | Why |
|---|---|
| The identifier is an `https://` URL | Pinned artifacts are never fetched over plaintext. |
| The URL ends in a filename (`.../mod-1.0.0.jar`) | There is no `filename` field; the launcher names the download from the URL's last path segment. A URL like `.../download?id=12` cannot be used. |
| `sha256` is manually pinned | It is the only integrity guarantee — nothing else verifies this file. |
| `compatible_versions` is present and non-empty | Nothing can infer which MC versions and loaders the file supports. |
| Every entry has `mc_version`, `loader`, and a real `mod_version` | `"latest"` is the unhydrated placeholder and cannot identify a pinned file. |

The host does **not** need to be on the launcher's GitHub/Modrinth allowlist: a `direct_hash` URL is
fetched through the signed-manifest host policy, where the pinned URL's own host (plus its
subdomains) authorizes the request. Every other guard still applies — HTTPS only, port 443, no
userinfo, no IP-literal hosts, no private/loopback destinations, per-hop redirect re-validation
within the pinned host, and the response-size cap. The SHA-256 hash is curator-pinned
out-of-band, so it stays authoritative regardless of where the file happens to be hosted.

One entry describes one file. All declared `compatible_versions` point at the same pinned URL and
hash, so a mod needing genuinely different files per Minecraft version needs one registry entry
per file. Adding a Modrinth source is still worthwhile when the project also exists there: it
hydrates display metadata, and the launcher falls back to it if the pinned host is unreachable —
the download is SHA-256-verified whichever source delivers it.

A pinned **mirror** is written as a second `direct_hash` source. Because the entry has a single
`sha256`, the mirror must serve byte-identical content:

```json
  "download_sources": [
    { "strategy": "direct_hash", "identifier": "https://developer.com/releases/mod-v1.0.0.jar" },
    { "strategy": "direct_hash", "identifier": "https://mirror.example.org/mod-v1.0.0.jar" }
  ],
```

Each pinned source authorizes only its *own* host, taken from the signed manifest — the fallback
does not inherit the preferred source's host policy.

### Field reference

| Field | Type | Required? | Description |
|---|---|---|---|
| `id` | string | Yes | Unique slug, lowercase, hyphenated. Must match the filename (minus `.json`). |
| `name` | string | Yes | Display name shown to users. |
| `content_type` | string | Yes | Always `"mod"` for this directory. Other valid values: `pack`, `shader`, `resourcepack`, `server`, `datapack`, `world`. |
| `author` | string | Yes | Creator or organization name. |
| `license` | string | Yes | SPDX license identifier (see §3 below). |
| `download_sources` | array | Yes, unless the legacy pair is used | Ordered `{strategy, identifier}` objects, best first. The launcher installs from the first source that is enabled and reachable. See "Where the file comes from" above. |
| `download_strategy` | string | Only without `download_sources` | One of: `github_release`, `modrinth_id`, `direct_hash`, `technic_pack`, `curated_pack`. Describes the *preferred* source; derived from `download_sources[0]` when that list is present. |
| `source_identifier` | string | Only without `download_sources` | Depends on strategy: `github_release` → GitHub `"owner/repo"`; `modrinth_id` → Modrinth project ID; `direct_hash` → direct HTTPS URL ending in the file's name. |
| `sha256` | string | Yes | SHA-256 hash of the downloadable file (64 lowercase hex chars). For `github_release` and `modrinth_id`, the compiler populates this from API metadata. For `direct_hash`, it MUST be manually provided. One hash covers the whole entry, so every pinned source must serve identical bytes. The launcher **blocks download** if the computed hash doesn't match. |
| `package_signatures` | string[] | Recommended | Java package prefixes used to attribute crash-log stack frames to this mod (e.g. `me.jellysquid.mods.sodium`). Use 2+ segments; single top-level like `net` is too broad. |
| `base_categories` | string[] | Recommended | Official curated category tags. Free-form lowercase strings. |
| `community_categories` | string[] | Optional | Freeform community tags. Auto-discovered by the compiler if absent. |
| `curator_note` | string | Recommended | Human-written markdown writeup shown in the UI and used as AI semantic context. |
| `icon_url` | string | Optional | CDN URL for the mod's icon. For `github_release`, provide manually (e.g. point to `raw.githubusercontent.com`). For `modrinth_id`, auto-populated. |
| `gallery_urls` | string[] | Optional | Array of CDN URLs for screenshots. Auto-populated from Modrinth or manually provided. |
| `compatible_versions` | array | Optional (**required for `direct_hash`**) | Array of `{mc_version, loader, mod_version}` objects. If absent, the compiler queries Modrinth for real version data (when a `modrinth_id` is resolvable). Otherwise falls back to `[{mc_version: "1.21", loader: "fabric", mod_version: "latest"}]`. **Do not set this manually unless you need to override** — the hydrator does it for you. The exception is `direct_hash`, where nothing hydrates: it must be supplied by hand, and the `"latest"` placeholder is rejected. |
| `mod_dependencies` | object | Optional | `{required: [...], optional: [...], incompatible: [...]}`. Mod IDs this mod depends on / is incompatible with. If absent, the compiler extracts from the jar's `fabric.mod.json` / `mods.toml` at install time (desktop-side), so this is a curated override only. |
| `mod_jar_aliases` | string[] | Optional | Alternate jar-declared IDs for cross-source matching (e.g. catalog `fabric-api` ↔ jar `fabric` ↔ Modrinth `fabric_api`). Lets the dependency-aware install system match across sources. Only needed when the jar-declared ID differs from the manifest `id`. |
| `governance.immune` | boolean | Optional (default `false`) | If `true`, bypasses all automated triage, vote penalties, and velocity circuit breakers. |
| `governance.override_justification` | string\|null | Required if `immune=true` | Displayed verbatim in the UI. |
| `governance.allow_comments` | boolean | Optional (default `true`) | If `false`, the review section is locked on this mod's page. |

---

## 3. SPDX license identifiers

The `license` field must be a valid SPDX identifier. Common examples:

- `MIT` — permissive
- `Apache-2.0` — permissive with patent grant
- `LGPL-3.0` — weak copyleft (Sodium, Iris, Lithium)
- `GPL-3.0` — strong copyleft
- `MPL-2.0` — weak file-level copyleft
- `LicenseRef-ARR` — All Rights Reserved (closed-source / proprietary; used by Xaero's Minimap)
- `LicenseRef-Proprietary` — for closed-source self-hosted mods
- `LicenseRef-<CustomName>` — any custom license; include a `license_url` or explanation in `curator_note`

Custom or non-open-source licenses MUST use the `LicenseRef-*` prefix. Do NOT invent SPDX-like strings (e.g. `"ARR"` without the prefix is invalid).

---

## 4. Modpack manifest schema (`registry/packs/<id>.json`)

Packs reference mods by ID and declare which loader + MC version the pack targets. A pack can mix mods from the curated registry, Modrinth (referenced by ID), and GitHub releases.

### Full example

```json
{
  "id": "optimized-survival",
  "content_type": "pack",
  "name": "Community Optimized Survival",
  "minecraft_version": "1.21",
  "loader": "fabric",
  "loader_version": "0.15.11",
  "mods": [
    { "id": "sodium", "source": "manifest", "status": "required" },
    { "id": "lithium", "source": "manifest", "status": "required" },
    { "id": "starlight", "source": "manifest", "status": "required" },
    { "id": "fabric-api", "source": "manifest", "status": "required" },
    {
      "id": "iris",
      "source": "manifest",
      "status": "recommended",
      "description": "Enable this if you want to use shader packs."
    },
    {
      "id": "xaeros-minimap",
      "source": "modrinth_id",
      "modrinth_id": "1bokaNcj",
      "version": "24.2.0",
      "status": "optional",
      "description": "Client-side minimap. Disable for a pure vanilla feel."
    }
  ],
  "override_url": null,
  "curator_note": "A curated, performance-focused survival pack for 1.21. Vanilla+ aesthetic with dramatically improved framerates.",
  "governance": {
    "immune": false,
    "override_justification": null,
    "allow_comments": true
  },
  "sha256": "de1d1fc288c327a2980c11dfbb370976f66f309a7dfcd72a746d82bc9623f51b"
}
```

### Pack-specific fields

| Field | Type | Required? | Description |
|---|---|---|---|
| `id` | string | Yes | Unique slug for the pack. |
| `content_type` | string | Yes | Must be `pack`. |
| `minecraft_version` | string | Yes | Target Minecraft version. |
| `loader` | string | Yes | Target loader: `fabric`, `quilt`, `forge`, `neoforge`. |
| `loader_version` | string | Yes | Pinned loader version. |
| `mods` | array | Yes | List of mod entries (see below). |
| `override_url` | string\|null | Optional | URL to a zip of configs / resourcepacks / shaderpacks to apply as overrides when installing the pack. Set to `null` if the pack has no overrides. |
| `sha256` | string | Optional | Hash of the override zip (if `override_url` is set). |

### Pack mod-entry fields

Each entry in `mods[]`:

| Field | Type | Required? | Description |
|---|---|---|---|
| `id` | string | Yes | Mod registry ID (if `source: "manifest"`) or display ID. |
| `source` | string | Yes | `manifest` (lookup in registry.db), `modrinth_id` (query Modrinth API directly), or `github_release`. |
| `modrinth_id` | string | Required when `source: "modrinth_id"` | The Modrinth project ID. |
| `version` | string | Optional | Exact version string. If omitted, the launcher defaults to the latest version compatible with the pack's `minecraft_version` + `loader`. |
| `status` | string | Yes | `required`, `recommended`, or `optional`. Drives the UI badge and whether the pack install flow aborts on failure. |
| `description` | string | Optional | Tooltip shown next to the mod in the pack install UI. |

---

## 5. Other content types

For shaders, resource packs, datapacks, servers, and worlds: the schema is the same as the mod manifest (§2), differing only in `content_type` and the directory:

| Content type | Directory | `content_type` value | Notes |
|---|---|---|---|
| Mod | `registry/mods/` | `"mod"` | See §2. |
| Pack | `registry/packs/` | `"pack"` | Uses the canonical `id` field; see §4. |
| Shader | `registry/shaders/` | `"shader"` | The downloader writes to `<instance>/shaderpacks/`. |
| Resource pack | `registry/resourcepacks/` | `"resourcepack"` | Writes to `<instance>/resourcepacks/`. |
| Server | `registry/servers/` | `"server"` | Server configuration / mod set. |
| Datapack | `registry/datapacks/` | `"datapack"` | Writes to `<instance>/datapacks/`. |
| World | `registry/worlds/` | `"world"` | Pre-built world download. |

**Shaders / resource packs / datapacks** typically use `modrinth_id` or `direct_hash` strategy. They are NOT `.jar` files (usually `.zip`), so:
- `package_signatures` is irrelevant (no Java packages) — leave as `[]` or omit.
- The desktop app routes the download to the correct instance subdirectory based on `content_type` (`shaderpacks/`, `resourcepacks/`, `datapacks/`).
- SHA-256 verification still applies — the hash must match the downloaded file.

---

## 6. Governance files (`registry/governance/`)

These are cross-cutting policy files that affect the whole registry, not a single entry.

### 6.1 Known conflicts (`known_conflicts.json`)

A JSON array of mod-pair conflicts. Used by the crash investigator (signal G in the dynamic scoring algorithm) and by the dependency-aware install system.

```json
[
  {
    "a": "example-renderer-mod",
    "b": "example-shader-mod",
    "severity": "hard",
    "mitigated_by": ["example-compat-shim"],
    "notes": "Hypothetical example -- both mods replace the same rendering pipeline and conflict at startup. Remove when you add real entries."
  }
]
```

> The current `registry/governance/known_conflicts.json` is `[]` -- the example above is illustrative only. Add real entries as mods enter the registry that genuinely conflict with each other.

| Field | Type | Description |
|---|---|---|
| `a` | string | First mod ID (lexicographically smaller). |
| `b` | string | Second mod ID (lexicographically larger). |
| `severity` | string | `"hard"` (will crash) or `"weak"` (may work but not recommended). |
| `mitigated_by` | string[] | Mod IDs that, when present, neutralize the conflict (e.g. `["indium"]` for Sodium+OptiFine). |
| `notes` | string | Free-text explanation. |

### 6.2 Poll blacklist (`poll_blacklist.json`)

A list of GitHub usernames excluded from triage poll vote tallies (bots, known bad actors). Empty by default.

```json
{"usernames": []}
```

### 6.3 Quarantine decisions (`quarantine_decisions.json`)

Curator-authored; the compiler reads but never writes this file. Maps governance event IDs to `accepted` or `rejected`. An `accepted` decision permanently lifts the vote quarantine for that event's reactions. A `rejected` decision permanently excludes those reactions.

```json
{"schema_version": 1, "decisions": []}
```

### 6.4 Governance state (`governance-state.json`)

Compiler-generated in `monitor` mode; records detected vote-surge events with stable IDs, affected GitHub reaction IDs, timestamps, and current status. Production tracks it at `registry/governance/governance-state.json` with `governance_repository: "agora-mc/Agora-Launcher"` and `policy: "production"`. It is public operational history but **not** a release asset because only the compiler consumes it. Validate recovery edits with `python scripts/validate_governance_state.py registry/governance/governance-state.json`; malformed or mismatched production state fails CI.

### 6.5 Audit log (`audit_log.json`)

Compiler-generated; do not edit manually. Appended to on every compile.

---

## 7. Crash signatures (`crash-signatures/<id>.json`)

Not under `registry/` — these live at the repo root in `crash-signatures/`. Each file defines a regex pattern that matches a known crash type + a human-readable fix hint + an optional action button.

### Example

```json
{
  "id": "fabric-api-missing",
  "name": "Missing Fabric API",
  "regex_pattern": "requires \\{fabric @",
  "solution_markdown": "A mod you installed requires **Fabric API**, but it is missing from your mod folder. Click the button below to install it automatically.",
  "action_button": {
    "label": "Install Fabric API",
    "mod_id": "fabric-api"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `id` | string | Unique slug matching the filename. |
| `name` | string | Display name shown in the crash diagnostic UI. |
| `regex_pattern` | string | Rust regex (no backreferences, no unbounded backtracking). Max 256 chars. Test against a ≥100KB crash log before merging. |
| `solution_markdown` | string | Markdown shown to the user explaining the fix. |
| `action_button` | object\|null | Optional `{label, mod_id}` — renders a button that installs the named mod. |
| `action_button.label` | string | Button text. |
| `action_button.mod_id` | string | Registry mod ID to install when clicked. |

### Regex DoS safety rules

1. The Rust `regex` crate is the only engine used — it structurally prevents catastrophic backtracking.
2. Maximum pattern length: 256 characters.
3. Anchor patterns where possible.
4. Prefer plain substring matches for known class names / error strings over regex.
5. Test every new pattern against a 100KB+ crash log before submitting the PR.

---

## 8. SHA-256 hash requirements

The `sha256` field (and `sha256` for packs) must be:
- A string (not a number).
- Exactly 64 lowercase hexadecimal characters.
- The actual SHA-256 of the downloadable file the user will receive.

How to compute it locally before submitting a PR:

**PowerShell (Windows)**
```powershell
(Get-FileHash .\mod-file.jar -Algorithm SHA256).Hash.ToLower()
```

**Python**
```python
import hashlib
print(hashlib.sha256(open("mod-file.jar", "rb").read()).hexdigest())
```

**Bash (Linux / macOS)**
```bash
sha256sum mod-file.jar | cut -d' ' -f1
```

For `github_release` and `modrinth_id` strategies, the compiler populates the hash automatically from API metadata — you don't need to compute it yourself, but the field must still be present (it will be overwritten on compile).

---

## 9. Submitting a new entry (PR workflow)

1. **Create the manifest file** in the appropriate `registry/<type>/` directory. The filename must match the `id` (e.g. `registry/mods/my-cool-mod.json` → `"id": "my-cool-mod"`).
2. **Compute the SHA-256** of the downloadable file (§8) and put it in the `sha256` field.
3. **For mods**: populate `package_signatures` with the Java package prefixes found inside the `.jar` (open it as a zip and look at the top-level directories). Use 2+ segments.
4. **Test locally** (if you have the repo checked out):
   ```bash
   python compiler/compile.py --skip-sign
   ```
   This compiles `registry.db` from the flat files. Verify your entry appears:
   ```bash
   python -c "import sqlite3; print(sqlite3.connect('registry.db').execute('SELECT id, name FROM registry_items').fetchall())"
   ```
5. **Submit a PR** with the new manifest file. The nightly CI compile runs `compile.py` and ships a new signed `registry.db` to GitHub Releases. Your entry appears in the Browse tab after the next nightly compile.

### Curation principles

- **Boutique, not warehoused.** Every entry is community-reviewed. Quality over quantity.
- **List every source a file is genuinely available from, best first.** Most mods are on Modrinth and on GitHub; declaring both means a rate-limited API or an outage costs the user a slower install rather than a failed one. Put Modrinth first for a mod that publishes there — its version API is unauthenticated, ungated, and carries per-file hashes — and keep `github_release` behind it as the developer's own no-intermediary source. Use `direct_hash` only when no API can resolve the file: closed-source, self-hosted, or a download that lives on the project's own site. It is the most manual option and every field must be maintained by hand, including a fresh `sha256` and `compatible_versions` on every version bump.
- **`direct_hash` cannot be used for CurseForge.** Since July 2026 the CurseForge CDN requires API-key authentication; unauthenticated requests to `edge.forgecdn.net` / `mediafilez.forgecdn.net` are refused. A pinned `forgecdn` URL will not download, and Agora has nowhere to hold a key (secrets never enter manifests or source, and a key-holding proxy would mean running a backend). CurseForge-exclusive mods are simply out of scope.
- **Curator notes matter.** The `curator_note` field is shown in the UI and used as semantic context for the AI crash investigator. Write a clear, 1-3 sentence summary of what the mod does and why a user would (or wouldn't) want it.
- **Don't set `compatible_versions` manually** unless you have a specific reason to override. The compiler fetches real version data from Modrinth's API for any mod with a resolvable `modrinth_id` (or whose manifest `id` matches a Modrinth slug). Manual overrides should be rare — `direct_hash` is the one strategy where it is mandatory.
- **Immunity is rare.** `governance.immune: true` should only be set for mods that are foundational and shouldn't be subject to community vote triage (e.g. a core API). Always include `override_justification` when doing this.
- **Archiving, not deleting.** To retire an entry, move its JSON file to `registry/archived/`. The compiler skips that directory entirely, so the entry disappears from the compiled database without losing git history.

---

## 10. Validation checklist

Before submitting a PR, verify:

- [ ] Filename matches `id`.
- [ ] `content_type` matches the directory (mods → `"mod"`, packs → `"pack"`, etc.).
- [ ] `license` is a valid SPDX identifier (or `LicenseRef-*` for custom).
- [ ] Every `download_sources` strategy is one of `github_release`, `modrinth_id`, `direct_hash`, `technic_pack`, `curated_pack`.
- [ ] Each identifier matches its strategy's format (GitHub `owner/repo`, Modrinth ID, or HTTPS URL).
- [ ] The sources are in genuine preference order, and every fallback actually serves this entry's file.
- [ ] `sha256` is 64 lowercase hex chars (compute via §8).
- [ ] `package_signatures` uses 2+ segment prefixes (for mods).
- [ ] `governance.immune` is `false` unless you have an `override_justification`.
- [ ] No file in `registry/archived/` has the same `id`.
- [ ] If adding a pack: every mod in `mods[]` either exists in `registry/mods/` (when `source: "manifest"`) or has a valid `modrinth_id` (when `source: "modrinth_id"`).
- [ ] If adding a known conflict: `a` is lexicographically smaller than `b`.
- [ ] If adding a crash signature: test the regex against a 100KB+ crash log; max 256 chars.

---

## 11. Autopopulated fields (don't set these manually unless overriding)

The nightly compiler hydrates these from the Modrinth API for any mod with a resolvable Modrinth presence (explicit `modrinth_id` OR a manifest `id` that matches a Modrinth slug):

- `icon_url` — from Modrinth project data.
- `gallery_urls` — from Modrinth project's gallery array.
- `description` — short description from Modrinth.
- `body_markdown` — full README-style body from Modrinth.
- `page_url` — constructed from the Modrinth slug.
- `license_id` — from Modrinth's license object.
- `source_updated_at` — from Modrinth's `updated` timestamp.
- `compatible_versions` — fetched from `/v2/project/{id}/version`, deduplicated by `(mc_version, loader)` pair with the latest mod_version per pair.
- `_hydrated_categories` — categories from Modrinth, filtered to remove loader-name noise.

**Manifest values always take precedence** over API-hydrated values. If you set `icon_url` in the manifest, the compiler keeps yours. If you leave it empty, the hydrator fills it from Modrinth. This is the "curator override" principle.

For mods with no Modrinth presence (pure GitHub-release mods whose slug doesn't match a Modrinth project), these fields stay empty unless you provide them manually. `compatible_versions` falls back to `[{mc_version: "1.21", loader: "fabric", mod_version: "latest"}]` — so set it manually if the mod targets a different version.

---

## 12. Common mistakes to avoid

1. **Using `"ARR"` as the license** — it's invalid. Use `"LicenseRef-ARR"` for All Rights Reserved.
2. **Single-segment `package_signatures`** like `["net"]` or `["com"]` — too broad; they match thousands of mods. Use at least 2 segments: `["net.fabricmc.fabric"]`.
3. **Setting `compatible_versions` to a hardcoded list** when you don't need to — the compiler fetches real data from Modrinth. Only override if the hydrator is wrong or the mod isn't on Modrinth.
4. **Uppercase `sha256`** — must be lowercase hex.
5. **Putting a pack manifest in `registry/mods/`** — packs go in `registry/packs/` and use `id` with `content_type: "pack"`.
6. **Deleting a manifest to retire it** — move to `registry/archived/` instead to preserve git history.
7. **Setting `governance.immune: true` without `override_justification`** — the compiler rejects it.
8. **Inventing a strategy** like `"curseforge"` — only `github_release`, `modrinth_id`, `direct_hash`, `technic_pack`, and `curated_pack` are supported.
9. **Using a URL as the identifier for `github_release`** — it must be `owner/repo` format (e.g. `CaffeineMC/sodium`), not a full URL.
10. **Forgetting `sha256` on a `direct_hash` mod** — it's required for all strategies; for `direct_hash` it's the only integrity guarantee and must be manually provided.
