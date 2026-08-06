# Releasing Agora

Agora has two independent release streams:

| Stream | Typical tag | Contents |
| --- | --- | --- |
| Registry | `registry-YYYY-MM-DD` | Signed registry database and web export |
| Desktop | `vX.Y.Z` | Platform installers and application bundles |

Do not treat a successful registry release as proof that a desktop package was built correctly, or vice versa.

## Desktop release checklist

### Before tagging

- [ ] Version metadata agrees across desktop package files.
- [ ] Changelog or release notes describe user-visible changes.
- [ ] Required public build variables are present in the release workflow.
- [ ] Signing and updater configuration are available for intended platforms.
- [ ] Unit, integration, frontend, and end-to-end tests pass.
- [ ] The in-app guide and website documentation match current labels.
- [ ] CLI help and `docs/CLI.md` match current commands.
- [ ] Migration from the previous public release has been tested with disposable data.
- [ ] A clean installation has been tested.
- [ ] A packaged upgrade has been tested.

### Build and inspect

The `Release` workflow in [`.github/workflows/release.yml`](../.github/workflows/release.yml) runs for a pushed `v*` tag or a manual dispatch with a tag. It tests core and CLI code, builds native desktop bundles on Windows, macOS, and Linux, packages standalone CLI archives, generates `SHA256SUMS`, assembles a draft release, and then publishes it after all build jobs succeed.

Public desktop build variables and secret boundaries are documented once in [DEVELOPMENT.md](./DEVELOPMENT.md). Confirm the workflow has every required public value and protected signing credential without copying their values into release notes or logs.

Do not rely on fixed installer filenames or package sizes in documentation. Tauri and platform tooling can change both.

The current workflow does not contain a manual approval gate: `assemble-release` changes the release from draft to published after the desktop and CLI jobs succeed. Inspect the draft while the workflow is running when practical, and cancel the run if any of these are wrong:

- platform and architecture coverage;
- version shown by the application;
- installer identity;
- checksums or signatures where provided;
- updater metadata;
- release notes;
- accidental debug artifacts.

### Packaged smoke test

Use the actual release artifact, not `tauri dev`.

Minimum test:

1. install or run the packaged build;
2. complete first-run setup on a clean disposable profile;
3. synchronize and verify the registry;
4. confirm Browse returns curated content;
5. confirm loader and Java catalogs are available;
6. create or import a disposable instance;
7. run health;
8. launch through the default delegated mode;
9. launch directly with a test Microsoft account when that platform is supported;
10. restart and verify settings and instances persist;
11. exercise update detection from the previous release.

The registry public-key check is release-critical. A package that builds successfully but lacks the expected verification key can fail only after installation. Always test registry synchronization in the packaged artifact.

### Publish

The workflow currently publishes automatically, before a human packaged smoke test can be recorded. Treat that as a release-process gap: verify artifacts immediately, stop promotion when validation fails, and correct the workflow before claiming a mandatory pre-publication approval.

After publishing:

- confirm the website download control selects the intended release;
- perform one clean public download;
- confirm update checks can see the release;
- monitor support channels for migration, signing, and installer failures.

## Registry release checklist

- [ ] Curated manifests validate.
- [ ] Governance inputs validate.
- [ ] Loader/runtime catalog inputs are current and pinned.
- [ ] Compiler tests pass.
- [ ] The registry is signed with the production key.
- [ ] The public key expected by released clients matches the signing key.
- [ ] Database and web export signatures verify.
- [ ] Release assets use the expected names.
- [ ] A current packaged desktop client can download and open the new registry.

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

### Registry

When a registry release is invalid:

1. do not weaken signature verification;
2. retain the invalid release for audit unless policy requires removal;
3. restore or republish the Last Known Good signed registry;
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
