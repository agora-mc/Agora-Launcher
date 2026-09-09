# Releasing Agora

Agora has two independent release streams:

| Stream | Typical tag | Contents |
| --- | --- | --- |
| Catalog | `registry-YYYY-MM-DD` | Signed catalog database and web export |
| Desktop | `vX.Y.Z` | Platform installers and application bundles |

Do not treat a successful catalog release as proof that a desktop package was built correctly, or vice versa.

## Desktop release checklist

### Before tagging

- [ ] Version metadata agrees across package files (`python scripts/set_release_version.py --check`; also enforced by the `version-metadata` CI job).
- [ ] Changelog or release notes describe user-visible changes.
- [ ] Required public build variables are present in the release workflow.
- [ ] Signing and updater configuration are available for intended platforms, including Apple Silicon and Intel macOS.
- [ ] The in-app guide and website documentation match current labels.
- [ ] CLI help and `docs/CLI.md` match current commands.
- [ ] Migration from the previous public release has been tested with disposable data.
- [ ] A clean installation has been tested.
- [ ] A packaged upgrade has been tested.

### Checks that gate a release build

No release artifact is built until these three workflows pass **for the commit being
released**. The `Release` workflow calls them with `uses:`, so they are the same
workflows that gate ordinary pushes and pull requests — not a second copy that can
drift — and they run against this run's checkout rather than a previous run's:

| Required check | Covers |
| --- | --- |
| [`CI Enforcement`](../.github/workflows/ci-enforcement.yml) | `cargo fmt`, clippy for core/CLI/desktop, Rust tests on Linux/Windows/macOS, compiler and script tests, TypeScript and Vite build, architecture boundaries, Tauri binding manifest, documentation gates, version-metadata agreement |
| [`Launch Planner CI`](../.github/workflows/launch-planner.yml) | Cross-platform launch-planner integration tests and the `launchMode` delegation-default assertion |
| [`E2E Tests`](../.github/workflows/e2e.yml) | The Playwright browser suite |

Two properties make "the checks passed for the released revision" a fact rather than
an inference:

- Because the checks are *called* rather than looked up, a stale green run on an
  earlier commit cannot stand in for this one, and a missing, pending, failed, or
  cancelled run leaves the build jobs unreachable. Path filters do not apply to a
  called workflow, so a change that no filter matches — a loader-only revision, for
  example — is still fully tested at release time.
- The `release-ref` job asserts that the release tag resolves to the commit this run
  is building. A tag push satisfies this by construction; a manual dispatch from a
  branch whose HEAD is not the tagged commit fails here instead of publishing
  artifacts that no check ever examined. Push the tag first, then dispatch against it.

`loader-manifests/`, `runtime-catalog/`, and `crash-signatures/` are `include_str!`
inputs to `agora-core`, so they are also in `CI Enforcement`'s push and pull-request
path filters. A loader-refresh commit therefore gets the full suite on `master`, not
only `Launch Planner CI`.

The `Web Build` and `Nightly Compiler` workflows are not release gates: they serve the
catalog and website streams on their own schedules.

### Build and inspect

The `Release` workflow in [`.github/workflows/release.yml`](../.github/workflows/release.yml) runs for a pushed `v*` tag or a manual dispatch with a tag. Its first build step rewrites the version from that tag into every file that carries one — the workspace `Cargo.toml`, `desktop/src-tauri/Cargo.toml`, `tauri.conf.json`, `desktop/package.json`, and the `Cargo.lock` workspace entries — via `scripts/set_release_version.py`. The tag is therefore the single source of truth for installer filenames, the version the application shows, and `agora --version`. It rewrites the ephemeral CI checkout only; nothing is committed back. After the required checks above pass, it builds native desktop bundles on Windows, macOS, and Linux, packages standalone CLI archives, generates `SHA256SUMS`, and assembles a draft release. The workflow never publishes the release itself; it stays a draft until a maintainer smoke-tests the artifacts and publishes explicitly.

Public desktop build variables and secret boundaries are documented once in [DEVELOPMENT.md](./DEVELOPMENT.md). Confirm the workflow has every required public value and protected signing credential without copying their values into release notes or logs.

Do not rely on fixed installer filenames or package sizes in documentation. Tauri and platform tooling can change both.

The workflow leaves the release as a draft with all artifacts uploaded and `SHA256SUMS` generated. It never publishes automatically. Inspect the draft after the workflow finishes before publishing, and cancel or re-run the workflow if any of these are wrong:

- platform and architecture coverage;
- both Apple Silicon and Intel macOS installers, with architecture-bearing bundle names;
- version shown by the application;
- installer identity;
- checksums or signatures where provided;
- updater metadata contains signed `darwin-aarch64` and `darwin-x86_64` entries;
- release notes;
- accidental debug artifacts.

### Packaged smoke test

Everything in this section is **manual and not enforced by CI**. The required checks
above run against source; nothing in them installs or launches a packaged build, so a
release that compiles and tests clean can still fail on first run. The one automated
exception is the macOS launch check inside `build-desktop`, which starts the packaged
`.app` for a few seconds to catch bundle and startup failures; it is not a substitute
for the steps below, and there is no Windows or Linux equivalent.

Use the actual release artifact, not `tauri dev`.

Minimum test:

1. install or run the packaged build;
   - On macOS, launch both the Apple Silicon and Intel `.app` packages on their matching hardware.
2. complete first-run setup on a clean disposable profile;
3. synchronize and verify the catalog;
4. confirm Browse returns curated content;
5. confirm loader and Java catalogs are available;
6. create or import a disposable instance;
7. run health;
8. launch through the default delegated mode;
9. launch directly with a test Microsoft account when that platform is supported;
10. restart and verify settings and instances persist;
11. exercise update detection from the previous release.

The catalog public-key check is release-critical. A package that builds successfully but lacks the expected verification key can fail only after installation. Always test catalog synchronization in the packaged artifact.

### Publish

The workflow leaves the release as a draft after all build jobs succeed. The desktop and CLI artifacts and `SHA256SUMS` are already attached; publication is the one manual step a maintainer performs after a packaged smoke test.

After the packaged smoke test passes, publish the draft:

- click **Publish release** on the draft's GitHub page, or
- run `gh release edit "$TAG" --draft=false --repo agora-mc/Agora-Launcher`.

After publishing:

- confirm the website download control selects the intended release;
- perform one clean public download;
- confirm update checks can see the release;
- monitor support channels for migration, signing, and installer failures.

## Catalog release checklist

- [ ] Curated manifests validate.
- [ ] Governance inputs validate.
- [ ] Loader/runtime catalog inputs are current and pinned.
- [ ] Compiler tests pass.
- [ ] The catalog is signed with the production key.
- [ ] The public key expected by released clients matches the signing key.
- [ ] Catalog database and web export signatures verify.
- [ ] Release assets use the expected names.
- [ ] A current packaged desktop client can download and open the new catalog.

The private signing key belongs only in the protected CI environment. Never place it in documentation, issue comments, artifacts, or local shell history.

## Rollback

### Desktop

When a desktop release is broken:

1. stop promoting the release;
2. document the affected platforms and user-visible failure;
3. preserve the failed artifacts for investigation;
4. publish or restore a known-good release according to updater behavior;
5. test migration from both the broken and previous good versions;
6. explain whether users must take manual action.

### Catalog

When a catalog release is invalid:

1. do not weaken signature verification;
2. retain the invalid release for audit unless policy requires removal;
3. restore or republish the Last Known Good signed catalog;
4. verify client fallback behavior;
5. correct source manifests or compiler logic through review;
6. publish a new signed release.

## Release documentation

Release notes should lead with player impact:

- what changed;
- whether migration is automatic;
- known limitations;
- recovery advice;
- platform-specific concerns.

Detailed compiler, signing, or governance internals belong in maintainer documents, not the first screen of player release notes.
