# Plugin implementation status

What is actually built, what is prototype, and what is not there at all. Kept separate from
`README.md` so the author guide can describe the feature while this file stays blunt about its
limits.

API version: **0.1.0**. Manifest schema: **1**. Both `plugins_enabled` and
`network_plugins_enabled` ship **off**.

## Milestones

| Milestone | State | Evidence |
|---|---|---|
| **P0** — script host and runtime selection | Done | `crates/agora-plugin-host/tests/host.rs` (21 tests) |
| **P1** — host, management, recovery | Done | `crates/agora-core/src/plugins/` + 63 unit tests |
| **P2** — extension surfaces | Done | `crates/agora-core/tests/plugins_end_to_end.rs` (27 tests) against the real QuickJS host |
| **P3** — first public release materials | Mostly done; see gaps below | `sdk/`, `examples/plugins/`, `docs/plugins/`, `fixtures/` |
| **P4** — author-hosted signed updates | Built; unexercised against a real host | `crates/agora-plugin-api/src/distribution.rs`, `crates/agora-core/src/plugins/updates.rs`, `crates/agora/tests/signing_round_trip.rs` |
| **P5** — deeper customization | Closed: home replacement built, the rest declined with reasons | `crates/agora-core/tests/plugins_end_to_end.rs`, `desktop/src/features/plugins/PluginSurface.test.tsx`, `BACKLOG.md` |

## What works

- Install from a `.zip` package or load a development folder in place. Path traversal,
  decompression ratio, entry count and total size are all bounded before anything is written,
  and a failed extraction leaves nothing behind.
- Enable, disable, uninstall. Discarding plugin data is a **separate** question from removing
  the plugin, and a reinstall of the same id adopts the retained settings.
- A grant is made once and compared afterwards. An update that asks for a capability — or for a
  network host — the installed version did not have is refused until it is accepted again, and the
  prompt names *what changed* rather than re-listing everything. An update that asks for the same
  or less installs without asking again, and updating a plugin the user switched off leaves it
  off.
- Dependency resolution with version ranges, cycle detection, and a deterministic activation
  order. Unresolvability propagates up a chain rather than one level.
- Pages, instance panels, palette and instance-context commands, declared settings, themes,
  diagnostics with typed repairs, and bounded pre-launch checks.
- Per-plugin logs, event subscriptions with origin and depth tracking, and a
  **Turn all off** recovery path that does not require plugins to cooperate.
- Activation events are honoured rather than decorative: a plugin declaring only `onView:` or
  `onCommand:` is not started at launch, and one declaring `onEvent:` is started when that
  event first fires.
- A CLI surface (`agora plugin …`) reaching the same core services as the GUI.
- Author-hosted updates: a plugin ships an `agora-plugin-update.json` naming its update URL and
  Ed25519 keys, which are pinned when the user agrees to install and read only from the database
  afterwards. Updates come from a signed, author-hosted document; the package is authenticated by
  the SHA-256 inside it. `agora plugin keygen` and `agora plugin sign` are the author side, and a
  round-trip test drives the real binary and verifies the result with the real verifier, because
  the two halves silently breaking apart is the failure nothing else would catch.
- Automatic checking is a separate opt-in (`plugin_updates_enabled`, off) from letting plugins
  reach the network (`network_plugins_enabled`, off). Neither implies the other. With it off,
  nothing contacts a publisher unless you press the button.

- A plugin can offer to render Agora's **home screen**. Offering is not taking: the offer joins a
  list in Settings and Agora's own screen renders until someone chooses otherwise. A chosen plugin
  that is disabled, removed or unable to run falls back to the built-in and says why, and the
  choice survives so re-enabling restores it.

## Known limits — read before relying on any of this

**This is a capability boundary, not a security sandbox.** Plugins run in bounded QuickJS
runtimes on host-owned threads in the launcher's own process. That contains bugs — infinite
loops, exceptions, runaway allocation — and it does not contain malice beyond the capability
checks. A plugin you granted `content:write` can disable your mods, because that is what you
agreed to. **The install prompt is the real control.** Nothing here protects against a defect
in the engine itself.

**Rollback is partial, and only covers the install itself.** During a replacement the previous
package is set aside and restored if *extraction* fails, so a failed install never leaves you
with a broken plugin. Once the new package is in place the old bytes are deleted — there is no
"go back to the previous version" command. When `dataVersion` changes, the plugin's stored data
is checkpointed first, and `restore_latest_checkpoint` exists, but nothing currently invokes it
automatically after a plugin's own migration goes wrong. Restoring package bytes would not undo
writes a plugin already made anyway; that is what the data checkpoint is for.

**Signatures prove continuity, not identity, and there is no revocation.** A verified update came
from whoever published the version you already have — nothing more. Whoever hands you the *first*
package chooses the key and the update URL, so a package from a bad source is bad forever and
every subsequent "update" will verify perfectly. There is no catalog, no publisher verification,
and no way to revoke a stolen key: an author whose key is taken has no channel to say so, and an
author who loses theirs cannot ship to existing installs again. `docs/plugins/publishing.md` says
this to authors in the same words. Replay is handled — a document carries a monotonic sequence and
an older one is refused — but that is a narrower guarantee than it sounds.

**Nothing about the update path has been exercised against a real host.** The format, the
verifier, the trust store and the install path are covered by tests, and a document signed by the
CLI is proven to verify in the launcher. What has *not* happened is a real fetch over the real
network from a real static host: the network gate rejects loopback by design, so a local server
would only prove the gate refusing it. Treat "it works end to end" as unproven until someone
publishes a plugin and updates it.

**Custom views were withdrawn, not deferred.** The `data:`-iframe prototype is gone: renderer,
bridge, read command, example and manifest support. A manifest declaring one is refused by name
with a migration message, pinned by a compatibility fixture.

The decisive reason was not the accessibility cost, which was known. It was that the script in
that frame ran in the WebView, outside every bound the plugin runtime exists to impose — no memory
ceiling, no interrupt handler, no deadline — so a 512 KiB document could hang the launcher with a
loop while the QuickJS budget it was supposedly subject to looked on. Two further gaps: the frame's
isolation could not be demonstrated for the packaged app (Tauri documents that on some platforms
IPC from an embedded frame is indistinguishable from IPC from its window), and accessibility and
controller behaviour inside the frame were the author's problem in an app where controller support
is first-class.

It was removed rather than repaired because repairing it buys a feature with no demonstrated user
and a permanent compatibility obligation. API 0.1 is experimental, unreleased and off by default,
so withdrawing now cost one example; withdrawing later would have been a breaking change. The
design history is in the commit that removed it. Reconsider only when a specific plugin needs an
interaction a reasonable host-rendered component cannot provide.

**Windows/MSVC is the only platform actually exercised.** The runtime is portable in principle
and `rquickjs` builds cleanly elsewhere, but macOS and Linux packaging of the plugin host has
not been verified here. Treat cross-platform as an open gate, not a claim.

**Replacement views cover one surface.** `home`, and nothing else. The instance overview is the
obvious next one and is deliberately not offered: it is an inline region inside `InstanceEditor`
rather than a component, and naming a surface a plugin can declare but never render would be worse
than not offering it. Only host-rendered views may replace a surface — a replacement is the whole
screen, and the custom-frame prototype owns its own accessibility and controller behaviour, so
standing one in for a built-in is exactly the thing "do not build a product on it" rules out.

**API 0.1 is experimental.** Pin `>=0.1, <0.2`. There is no deprecation window yet, because
there has not yet been anything to deprecate. Before v1 is advertised as stable, the items
under "Remaining before a stable v1" must be resolved.

## Compatibility fixtures

`docs/plugins/fixtures/v0.1/` pins the shipped surface: manifests that must keep loading, and
manifests that must keep being refused *for the same reason and at the same stage*.
`fixtures/v0.1/distribution/` does the same for `agora-plugin-update.json`, kept separate because
how a plugin is published and what it may do are separate agreements that change on separate
schedules. Both are enforced by `crates/agora-plugin-api/tests/compatibility.rs`.

When API 0.2 arrives, this directory becomes the "older supported plugin" set the plan asks
for. Adding a fixture after fixing a contract bug is cheap and correct; **changing** one is a
deliberate compatibility decision and should be reviewed as exactly that.

## Remaining before a stable v1

- [ ] Verified behaviour in the packaged desktop app on macOS and Linux, not only Windows.
- [ ] A real deprecation and support-window policy, tested by running v0.1 fixtures against a
      v0.2 host.
- [ ] Honest, complete update rollback — or documentation that stops implying one exists.
- [ ] A real publish-and-update cycle against an author-hosted static file, not only tests.
- [ ] A revocation story, or an explicit decision that there will not be one.
- [ ] A curated catalog, if there is ever to be one. There is none today and installing does not
      need one.
- [x] ~~A decision on whether the custom-view prototype becomes supported or is withdrawn.~~
      Withdrawn.

Content-source providers, install hooks and generic import/export hooks were **declined** rather
than deferred; `BACKLOG.md` records why. The short version: an extension point earns its
maintenance cost by enabling something the existing API cannot express, and none of the three has
a plugin that needs it. Install *observation* is already served by the ten lifecycle events;
install *participation* is a transaction problem — ordering, veto, rollback, crash recovery — that
should not be invented speculatively. A content source must never be able to certify its own
artifacts, so it needs a provenance design before it needs an interface: a matching SHA-256 proves
the bytes match an expected digest, and says nothing about whether that digest was trustworthy.
