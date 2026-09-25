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

The TypeScript shapes for every request and response are in `sdk/index.d.ts` under *Content
providers*. The Rust definitions, with every bound, are in
`crates/agora-plugin-api/src/provider.rs`.

## Install plans

`resolve` returns one of two plans.

**`file`**: one file into an existing instance, plus the dependencies you declare. Dependencies
name projects *in your provider*; Agora resolves each one by asking you again, offers them in the
same review screen as curated dependencies, and flags an installed `incompatible` project as a
blocking conflict.

**`pack`**: a whole modpack, which Agora creates a new instance from. Each file's `path` must sit
under `mods/`, `config/`, `defaultconfigs/`, `resourcepacks/`, `shaderpacks/`, `datapacks/` or
`kubejs/`. Executables are refused everywhere, and `.jar` files only belong in `mods/`. An
optional `overrides` zip goes through the same sanitiser as `.mrpack` overrides. `kubejs/` is not
inert (KubeJS runs the scripts it finds there), which is exactly why trusting the provider is the
user's decision.

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

**One rule decides what counts as verified, for every provider:**

| A planned file… | Counts as |
|---|---|
| over HTTPS, from a host the provider declared, with SHA-256 or SHA-512 | verified |
| from any other host, over plain HTTP, with only MD5/SHA-1, or with no digest | **unverified content** |

Unverified content installs only when the user has turned on **Allow unverified zip packs**. It is
the same setting Technic's bare-zip packs have always needed, because it is the same question.
Downloads always go through Agora's network policy. Lockdown Mode still wins, private and loopback
addresses are still refused, and redirects of in-scope downloads must stay on the declared hosts.
A single `file` plan additionally needs at least SHA-1, since the install pipeline will not
install one file it cannot re-check.

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
| Modrinth single-file install (`resolver.rs`, `SourceType::Modrinth`) | Browse installs of Modrinth mods use the older Modrinth resolver, which reads dependency data from the downloaded jar | That jar-level dependency resolution has no provider-vocabulary equivalent yet. `ModrinthProvider::resolve` exists and is tested |
| Modrinth modpacks (`.mrpack`) | Installed by the mrpack importer | A `.mrpack`'s file list is inside the archive, so a plan cannot be written without downloading it, which a provider must not do |
| Technic pack install (`crate::technic`) | Uses Technic's consent tiers (Solder with `technic_enabled`, zip with `allow_unverified_packs`) | Under the shared rule every Technic pack is unverified content, which is stricter than today. `TechnicProvider::resolve` produces the plan; switching is one line once that is decided |
| `ModDetail.tsx` | The Modrinth and Technic project pages are their own components | Plugin providers use the generic `ProviderDetail` page; the official ones keep their richer pages for now |
| Browse category picker | Offers Modrinth's category tags when Modrinth is on | Presentation only; providers can already declare their own filters |
| Ranking (`browse_cache::ranking_input`) | Technic's likes use a different ceiling than follows | Calibration data, not behaviour |

## Updates

Provider plugins update exactly like every other plugin: through their signed, author-hosted
update document, with the same compatibility check and the same capability-widening consent.
Settings → Software updates → **Check everything** lists Agora and every plugin together, and
**Update all** applies them. It never accepts a permission change on anyone's behalf: a plugin
update that asks for more is left for review in Plugins.
