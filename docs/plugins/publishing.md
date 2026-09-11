# Publishing a plugin, and shipping updates

You do not need Agora's permission to publish a plugin, and you do not need a server. A plugin is
a `.zip` and a JSON file on any host that serves HTTPS — GitHub Pages, a release asset, your own
box. Nobody reviews it, nobody can take it down, and nothing here phones home.

That freedom is the point, and it is also why the trust model below is narrower than it might
first appear. Read that section before you decide to rely on it.

> **Status:** API 0.1 is experimental and both plugin switches ship off. See
> [implementation-status.md](implementation-status.md) for what is actually built and what is not.

## The short version

1. Generate a signing key. Keep the private half safe; if you lose it you cannot ship updates to
   existing installs, ever.
2. Put `agora-plugin-update.json` in your package, naming your update URL and your public key.
3. Publish the package.
4. Publish a signed update document at that URL listing the release.
5. For each later release: add it to the document, increment `sequence`, re-sign, re-upload.

## Two files

### `agora-plugin-update.json` — inside the package

```json
{
  "schema": 1,
  "url": "https://plugins.example.com/acme.dashboard.json",
  "keys": [
    { "id": "2026-09", "algorithm": "ed25519", "publicKey": "<base64 of 32 raw bytes>" }
  ]
}
```

This is read **once**, when someone installs your plugin, and the user sees the host and the key
fingerprint as part of what they are agreeing to. Agora then records it and never reads it from a
downloaded file again.

It is a separate file from `agora-plugin.json` on purpose. Where a plugin gets its updates is a
different question from what a plugin is allowed to do, and keeping them apart means the manifest
contract does not move every time distribution grows a feature.

A plugin with no such file simply has no updates. That is a perfectly reasonable thing to ship.

### The update document — hosted at `url`

```json
{
  "schema": 1,
  "id": "acme.dashboard",
  "sequence": 4,
  "releases": [
    {
      "version": "1.2.0",
      "url": "https://github.com/you/acme-dashboard/releases/download/v1.2.0/acme.dashboard.zip",
      "sha256": "<64 lowercase hex characters>",
      "size": 18342,
      "apiRange": ">=0.1, <0.2",
      "notes": "Fixes the mod count on instances with no mods.",
      "published": "2026-09-11T12:00:00Z"
    }
  ],
  "signatures": [
    { "keyId": "2026-09", "algorithm": "ed25519", "value": "<base64 of 64 raw bytes>" }
  ]
}
```

Notes on the fields that are easy to get wrong:

- **`sequence` must increase on every publication.** It is not a version number; it is a counter
  for the document itself. Agora records the highest it has verified and refuses anything lower,
  which is what stops someone serving an old copy of your document back to your users to keep them
  on a release you have since fixed. If you forget to increment it, clients that already saw the
  higher number will refuse the new document.
- **`sha256` is lowercase hex of the package bytes**, and it is what actually authenticates the
  download. The `url` is only where to look. Because the hash is inside the signed document, the
  package itself needs no signature and can live on a completely different host.
- **`size` is checked before the body is read**, so an oversized response is abandoned rather than
  downloaded and then rejected.
- **`apiRange` must match the `apiRange` in that release's own manifest.** It is carried here so
  Agora can skip a release it cannot run without downloading it first.
- **A version may appear exactly once.** One version means one set of bytes. If you need to change
  the bytes, publish a new version.

### What gets signed

The signature covers the document with the `signatures` field removed, canonicalised (object keys
sorted, no insignificant whitespace), prefixed with the literal context string:

```
agora-plugin-update:v1:
```

The prefix is a domain separator, so a signature over an update document can never be replayed as a
signature over something else. Canonicalising means reserialising or reindenting the document on
the way through a CDN does not break a valid signature, while changing anything a client acts on
does.

## What the signature proves — and what it does not

**It proves continuity, not identity.** A verified update came from whoever published the version
you already have. That is genuinely useful: it means nobody can push you a modified build of
someone else's plugin.

It is **not** proof of who that publisher is, and it is not a review. Specifically:

- **Whoever gives you the first package chooses the key and the URL.** A package obtained from a
  bad source is bad forever — every "update" after it will verify perfectly. The first install is
  the decision that matters; everything after it is just consistency with that decision.
- **The `publisher.plugin` id is not an authorship claim.** Anyone can call themselves anything.
- **A fingerprint on its own tells you nothing.** It is worth something only if you compare it
  against a key the author published somewhere you already trust — their repository, their site.
  Agora shows it so that comparison is *possible*, not because looking at it is a security step.
- **A plugin you granted capabilities to can misuse them**, and a signature does not change that.
  The install prompt is the real control.

This is roughly what an Android signing key gives you, with the same limits.

## Keys

Generate an Ed25519 key and keep the private half out of your repository, your package, and your
CI logs. It is the one secret in this system.

**Rotation.** `keys` is a list. To rotate: publish a release, signed with your *current* key, whose
package names both the current and the new key. Anyone who installs that release pins both, and
your next release can be signed with the new one. Leave the old key in place until you are
confident your users have moved forward — there is no way to know, so err on the side of longer.

**Loss.** If you lose your only key, that is the end of the line for existing installs. There is no
authority that can vouch for a replacement key and nowhere to publish a revocation, because there
are no servers. Your users will have to uninstall and install the new plugin deliberately. This is
a real cost of the $0.00/month design, and it is why rotating *early* — publishing a second key
before you need it — is worth the five minutes.

**Theft.** Same answer, and it is worse: someone with your key can publish anything to anyone who
has your plugin installed. There is no revocation. Tell your users through whatever channel you
have, and expect that some of them will not hear you.

If those consequences are unacceptable for what your plugin does, do not ship an update source.
Distribute by hand, and let each install be a deliberate decision.

## Updates a user has to agree to again

Agora compares each update against what was granted:

- More capabilities than before, or a new host in `network.hosts`, and the update stops for
  explicit consent naming exactly what is new.
- The same or fewer, and it applies without nagging.
- Updating a plugin the user switched off leaves it switched off.

So adding a capability is a decision your users make, not one you make for them. Expect a lower
uptake on releases that widen permissions, and say why in `notes`.

## Checking your work

```bash
agora plugin preview package ./acme.dashboard.zip --json
```

shows exactly what a user would be agreeing to, including the update host and key fingerprints,
without installing anything.

## What does not exist yet

No catalog, no search, no publisher verification, no download counts, no revocation, and no
automatic install of updates without the user asking. Background *checking* is opt-in and off by
default; nothing contacts your host until someone turns it on or presses the button.

If any of that changes it will be written here and in
[implementation-status.md](implementation-status.md), which is kept deliberately blunt.
