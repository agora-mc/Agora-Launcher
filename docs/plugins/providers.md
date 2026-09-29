# Content providers

A **content provider** is a source of browsable, installable content outside Agora's curated
catalog. Modrinth and Technic are providers. So is any plugin that declares a `contentProviders`
contribution. Browse, the project page and the install flow talk to all of them through one
interface, and none of them is special-cased by name.

> **Providers decide what content is available. Agora decides how that content is safely
> installed.**

A provider answers four questions: *what exists* (`search`), *tell me about one thing*
(`project`), *which versions are there* (`versions`), and *for this instance, what exactly should
be downloaded* (`resolve`). It never downloads, writes, snapshots or records anything. Agora does
all of that, using the same staging, verification, snapshot and rollback as curated content.

Requires plugin API **0.1.1** (`"apiRange": ">=0.1.1, <0.2"`).

## A minimal provider

`examples/plugins/provider/` is a complete, runnable provider that serves three projects from
memory. `crates/agora-core/tests/providers_end_to_end.rs` drives it through the real script host,
Browse and install resolution. Start from it.

```json
{
  "capabilities": { "required": ["content:provide", "network"] },
  "network": { "hosts": ["api.example.org", "files.example.org"] },
  "contributions": {
    "contentProviders": [{
      "id": "shelf",
      "title": "Example Shelf",
      "contentTypes": ["mod", "pack"],
      "sorts": ["relevance", "downloads"],
      "paginates": true,
      "filters": [{ "id": "side", "title": "Side", "options": [
        { "value": "client", "label": "Client" }, { "value": "both", "label": "Both" }
      ] }],
      "exports": { "search": "search", "project": "project", "versions": "versions", "resolve": "resolve" }
    }]
  }
}
```

- `content:provide` is required, and only granted by the user at install time. A manifest that
  declares a provider without it is refused.
- `contentTypes` come from Agora's vocabulary: `mod`, `pack`, `resourcepack`, `shader`,
  `datapack`, and `server` (browse-only: no install plan may carry it).
- `filters` are how a source exposes something only it has, without Agora growing an API named
  after that source. Agora renders them and forwards only values you declared.
- `paginates: false` means your `search` ignores `offset`. Agora then asks once per query and
  pages through the answer itself.
- `project` is optional. Without it, the detail page is built from the search result.
- `categories` appear in Browse's category picker beside every other source's. The chosen id
  comes back to you as `SearchRequest.category`; ignore ids you do not recognise.
- `ranking` says where popularity saturates on *your* site (`downloadsCeiling`,
  `endorsementsCeiling`) and which of your categories mark libraries. Browse merges every
  source into one list, and a download on a small site should not always lose to one on a huge
  site. It defaults to Modrinth's calibration; Technic declares a lower endorsement ceiling
  because its ratings are denser. Curated content keeps its own band above every provider.

The TypeScript shapes for every request and response are in `sdk/index.d.ts` under *Content
providers*. The Rust definitions, with every bound, are in
`crates/agora-plugin-api/src/provider.rs`.

## Install plans

`resolve` returns one of two plans.

**`file`**: one file into an existing instance, plus the dependencies you declare. Dependencies
name projects *in your provider*; Agora resolves each one by asking you again, offers them in the
same review screen as curated dependencies, and flags an installed `incompatible` project as a
blocking conflict.

**`pack`**: a whole modpack, which Agora creates a new instance from. Each file's `path` is
relative, stays inside the instance, and is never a native executable. Paths under `mods/`,
`config/`, `defaultconfigs/`, `resourcepacks/`, `shaderpacks/`, `datapacks/`, `kubejs/`,
`scripts/`, `global_packs/`, `openloader/` or `patchouli_books/` (with `.jar` only in `mods/`)
install normally; anything else, like `options.txt`, installs only when the user has turned on
**Reduced security mode**, and the install prompt lists those files. An optional `overrides` zip
goes through the same sanitiser as `.mrpack` overrides, under the same mode. `kubejs/` and
`scripts/` are not inert (KubeJS and CraftTweaker run what they find there), which is exactly why
trusting the provider is the user's decision.

## The trust model

This is the part to read carefully, because it is the reason providers were first declined and
what changed.

**A matching hash does not prove a provider is trustworthy.** The provider supplies both the URL
and the digest, so a match proves only that the bytes are the ones the provider meant. That is
worth having (it catches corruption, truncation and a swapped file on a mirror), but it is not
an endorsement. It is the same guarantee Modrinth's API has always given Agora.

**Trust is the user's decision, made once and kept visible.** The user grants `content:provide`
when they install the plugin, sees its `network.hosts` at the same time, and can switch the
provider off in Settings → Content sources. Every file a provider installs is recorded with the
provider's id, project and version in the instance manifest, so the decision stays attributable
after the fact.

**One rule, for every provider**, and it is the one Technic has always used:

| A planned file… | What happens |
|---|---|
| over HTTPS, from a host the provider declared, with SHA-256 or SHA-512 | installs; nothing to warn about |
| from another host, over plain HTTP, or with only MD5/SHA-1 | **reduced security**: the user is warned and may continue |
| with no digest at all | **low security**: hidden and not installable unless the user turned on **Allow low security downloads** |

**Reduced security mode** is the one switch for limits that are reasonable to lift for a source
the user trusts: plugins reaching any host (`"network": {"hosts": ["*"]}`) or more than 10, and
packs placing files outside the usual folders. It never allows executables, paths outside the
instance, or skipping digest checks. See MASTER_SPEC §21.3.

Agora warns and asks; it does not decide for the user. A provider can mark a search result
`lowSecurity: true` so it stays out of Browse for users who have not opted in, without filtering
for them itself. Downloads always go through Agora's network policy: Lockdown Mode still wins,
private and loopback addresses are still refused, and redirects of declared-host downloads must
stay on the declared hosts. Every digest a provider publishes is checked, including MD5.

## Official providers are not privileged

Modrinth and Technic are compiled into Agora rather than shipped as JavaScript plugins. That was
a deliberate choice, not an oversight. What "just another plugin" is meant to guarantee is
that Agora's own providers have no capability a community provider lacks, and that holds
structurally: both implement the same `ContentProvider` trait as the plugin bridge, return the
same `InstallPlan` type, and are judged by the same rule above. Keeping them in Rust keeps
~2,400 lines of tested code, keeps the plugin runtime off for people who only want Modrinth, and
avoids needing an official signing key. Porting either to a plugin later is a matter of registering
a different implementation.

### Migration debt

These paths still name a source. Each is listed so it gets retired rather than forgotten.

| Where | What is still source-specific | Why it has not moved yet |
|---|---|---|
| Modrinth single-file install (`resolver.rs`, `SourceType::Modrinth`) | Browse installs of Modrinth mods use the older Modrinth resolver, which reads dependency data from the downloaded jar | See *Modrinth's single-file install on the provider path* below: a follow-up, deliberately not part of the provider change. `ModrinthProvider::resolve` exists and is tested |
| Modrinth modpacks (`.mrpack`) | Installed by the mrpack importer | A `.mrpack`'s file list is inside the archive, so a plan cannot be written without downloading it, which a provider must not do |
| Technic pack install (`crate::technic`) | Still installs through its own importer | The rules now match (Solder warns, zip needs low security downloads); what remains is routing the button through `TechnicProvider::resolve` |
| `ModDetail.tsx` | The Modrinth and Technic project pages are their own components | Plugin providers use the generic `ProviderDetail` page; the official ones keep their richer pages for now |
| Browse category picker | Offers Modrinth's category tags when Modrinth is on | Presentation only; providers can already declare their own filters |
| Ranking (`browse_cache::ranking_input`) | Technic's likes use a different ceiling than follows | Calibration data, not behaviour |

## Updates

Provider plugins update exactly like every other plugin: through their signed, author-hosted
update document, with the same compatibility check and the same capability-widening consent.
Settings → Software updates → **Check everything** lists Agora and every plugin together, and
**Update all** applies them. It never accepts a permission change on anyone's behalf: a plugin
update that asks for more is left for review in Plugins.

## Where this is going

The direction agreed so far. Each part says whether it is built.

### One detail page

`ModDetail` and `ProviderDetail` merge into a single page built from the provider vocabulary
(project, versions, install), with Agora's curated layer (votes, curator notes, governance) added
when a catalog entry exists, and a provider able to contribute extra blocks through the existing
host-rendered view model.

### Modrinth's single-file install on the provider path (next, as its own change)

The older path downloads the jar while planning and reads the mod's own metadata
(`fabric.mod.json` and friends), because Modrinth's declared dependencies are sometimes wrong for
the chosen loader. It then maps the jar's mod ids back to Modrinth projects (via the catalog's
aliases, then a Modrinth search), handles several roots installed together, collapses a jar id
and a Modrinth project that turn out to be the same mod, and can fall back to the closest
Minecraft version. That is about 700 lines of tuned behaviour on the most-used install path.

The plan is to **parameterize that engine over `ContentProvider`** rather than rewrite it: its
Modrinth calls become trait calls (list versions, resolve, project details, and a new optional
"which of your projects provides mod id X?"), and jar-metadata checking becomes a core step every
provider's file plans get. Then Browse's Modrinth installs switch to `provider:modrinth:<id>`
items. It lands as its own pull request so the change to that path gets its own review.

Already provider-neutral: update checks (provider-installed content is checked against its
provider, and **Update all** applies it through the same resolver), and the installed-content
view labels each item with the provider it came from.

### `.mrpack` and curated content

- `.mrpack` files already install without the Modrinth provider being switched on: the importer
  checks every file's SHA-1 against the index and only downloads from Modrinth's and GitHub's CDNs.
  Routing it through a plugin would add nothing, so it stays as is.
- Curated entries whose sources are Agora-native (`github_release`, `direct_hash`,
  `curated_pack`) keep resolving in core. Entries sourced from a provider (`modrinth_id`,
  `technic_pack`) resolve through that provider once the official providers are plugins.
- **Decided:** curated entries whose only usable source needs a plugin the user does not have are
  shown by default when that plugin is official (with a button to turn it on), and behind a
  setting for community plugins. Not built yet.

### Curating a provider's pack as it is (built)

A catalog entry with `download_strategy: "provider_pack"` pins one version of any provider's pack
(`technic:tekkit@3.1.2`, or a plugin source's pack) and its **plan digest**: the SHA-256 of the
plan the curator reviewed, printed by `agora provider plan-digest`. At install time Agora resolves
the same version, recomputes the digest and, if the provider now serves something different,
tells the user it is no longer the reviewed version and installs it only if they accept it as
uncurated content. This is how Technic Solder packs become curatable: the digest covers every
mod's URL and MD5, so a build that changes after review is caught. `.mrpack` packs keep using
`modrinth_id`. See `REGISTRY_CURATION_REFERENCE.md`.

### Native plugins: not planned

A native (compiled) plugin would need its own program built for every platform Agora runs on, for
every plugin. That is a real cost for every author, for a benefit (speed, other languages) that
providers do not need: they spend their time waiting on the network. So plugins stay JavaScript
on QuickJS, and Modrinth and Technic stay Rust built-ins behind the same trait as plugins (see
*Official providers are not privileged*).

Considered and set aside: a companion process per plugin speaking the host protocol over stdin/
stdout (the per-platform build cost above), loading a native library into Agora's process (no
stable Rust ABI, and a plugin crash is an Agora crash), and WebAssembly (sandboxed and
multi-language, but a 10–20 MB engine; it could be added as another `ScriptHost` later).
