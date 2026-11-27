# Releasing Quasar

Quasar releases are owned by `fordft/Quasar`, not by the upstream Zephium
project. Pushing a `v*` tag runs [`release.yml`](workflows/release.yml) only in
this repository. The workflow checks the tagged source, builds macOS, Windows
and Linux artifacts, signs the updater payloads, attests build provenance and
creates a **draft** GitHub release. Nothing becomes public until a maintainer
publishes that draft.

The in-app updater trusts Quasar's own minisign public key and reads only:
`https://github.com/fordft/Quasar/releases/latest/download/latest.json`.
Upstream Zephium keys, endpoints, accounts and releases are not used.

## Version scheme

Quasar has its own semantic-version line, independent of upstream Zephium.
`desktop/Cargo.toml` `[package] version` is the only version source;
`desktop/tauri.conf.json` must not repeat it. The release tag is exactly
`v` plus that version, for example `v1.2.0` or `v1.2.0-beta.1`. The workflow
rejects a tag that is not valid semver or does not equal the desktop crate
version.

Do not reuse, move or mutate a published tag. Fix forward with a higher Quasar
version.

## Secrets and signing configuration

The updater signing secrets are required for every release:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

Apple signing is optional as one complete set. If all are present, the macOS
release is Developer ID signed and notarized. If none are present, it is
ad-hoc signed and not notarized. Any partial set fails the release gate:

- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_TEAM_ID`
- `APPLE_API_ISSUER`
- `APPLE_API_KEY`
- `APPLE_API_KEY_PATH`

Windows SignPath signing is also optional as one complete set. If all are
present, the installer is Authenticode signed and timestamped before its final
updater signature is made. If none are present, the installer remains unsigned
and the notes say so. Any partial set fails the release gate:

- `SIGNPATH_API_TOKEN` (secret)
- `SIGNPATH_ORGANIZATION_ID` (secret)
- `SIGNPATH_SIGNER_SUBJECT` (variable, because it is the public certificate
  subject rather than a secret)

When configuring SignPath, use project slug `quasar`, signing policy
`release-signing` and artifact configuration `windows-nsis`, restricted to this
repository, `.github/workflows/release.yml`, protected `v*` tags and
GitHub-hosted runners. The workflow verifies publisher trust, the exact subject
and a timestamp before replacing the installer. It then re-signs the final
installer bytes for the updater in both signed and unsigned configurations.

Never print, commit or paste private keys or passwords into issues, pull
requests, logs or documentation.

## Updater key custody

The maintainer keeps the updater private key and its password in offline key
custody, with the GitHub Actions secrets serving only the release workflow.
The public key in `desktop/tauri.conf.json` is not secret.

Losing the private key or password means every installed copy can never accept
another signed update. There is no recovery path that existing installs can
trust. Rotation therefore requires advance custody of the old key: ship one
release whose embedded public key is the new key while its updater payload is
still signed with the old key. Once enough users have that release, later
payloads can be signed with the new key. If the old key is lost before that
transition, users must reinstall manually. Treat a leaked key as a security
incident and rotate it the same way.

## Cutting a release

1. **Refresh the blocker seed** immediately before tagging when it is near its
   freshness boundary. Supply reviewed source files and preserve all license
   and provenance checks:

   ```sh
   sources="${HOME}/.cache/quasar-blocker-seed-sources"
   mkdir -p "${sources}"
   curl -fsSL -o "${sources}/easylist.txt" https://easylist.to/easylist/easylist.txt
   curl -fsSL -o "${sources}/easyprivacy.txt" https://easylist.to/easylist/easyprivacy.txt
   curl -fsSL -o "${sources}/ublock-filters.txt" https://ublockorigin.github.io/uAssets/filters/filters.min.txt
   curl -fsSL -o "${sources}/ublock-quick-fixes.txt" https://ublockorigin.github.io/uAssets/filters/quick-fixes.txt
   curl -fsSL -o "${sources}/ublock-link-shorteners.txt" https://ublockorigin.github.io/uAssets/filters/ubo-link-shorteners.txt
   curl -fsSL -o "${sources}/ublock-privacy.txt" https://ublockorigin.github.io/uAssets/filters/privacy.min.txt
   curl -fsSL -o "${sources}/ublock-resource-abuse.txt" https://ublockorigin.github.io/uAssets/filters/resource-abuse.txt
   curl -fsSL -o "${sources}/ublock-unbreak.txt" https://ublockorigin.github.io/uAssets/filters/unbreak.min.txt
   curl -fsSL -o "${sources}/ublock-badware.txt" https://ublockorigin.github.io/uAssets/filters/badware.min.txt
   cargo xtask update-blocker-seed --sources "${sources}" \
     --license assets/blocker-seed/v1/LICENSE-CC-BY-SA-3.0.txt \
     --gpl-license assets/blocker-seed/v1/LICENSE-GPL-3.0.txt
   cargo xtask check-blocker-seed
   ```

   If the seed changes, update the pinned digest/count evidence described in
   [the adblock maintenance guide](../docs/adblock.md#official-list-maintenance)
   and run the focused blocker tests.

2. **Bump the version in a pull request.** Edit only `desktop/Cargo.toml`'s
   package version and run `cargo check -p zephium-desktop` so `Cargo.lock`
   agrees, then verify the result with `--locked`. Review the updater
   public key and endpoint in `desktop/tauri.conf.json`; they are release
   trust configuration, not routine version-bump edits.

3. **Run the local release security gate**:

   ```sh
   cargo xtask check-release-engine-security
   ```

4. Merge the version bump to `main` and wait for the complete main CI run for
   that exact commit to succeed. Release creation requires it again.

5. **Create and push an annotated tag** for the merged commit:

   ```sh
   git tag -a vX.Y.Z -m "Quasar X.Y.Z"
   git push origin vX.Y.Z
   ```

6. **Let the Release workflow draft the release.** It verifies the tag/version
   match, updater endpoint and public key, required updater secrets, optional
   signing sets, release security policy and successful main CI. It runs the
   WebKit component suite unless an explicit emergency dispatch skips it, then
   builds:

   - macOS Apple Silicon `.app`, DMG and updater archive;
   - Windows x64 NSIS installer and updater signature;
   - Linux x86_64 AppImage plus its runtime source archive, using the same
     runtime preparation, finalization and startup smoke test as the preview
     pipeline.

   The draft receives `SHA256SUMS`, `latest.json` and build-provenance
   attestations. Linux is never listed in `latest.json`.

7. **Review the draft before publishing.** Check the exact assets, hashes,
   provenance, updater manifest, release notes and the notes' statements about
   macOS notarization and Windows Authenticode signing for this release. Install
   and qualify the final DMG and Windows installer on their native platforms.
   The Linux AppImage smoke test is workflow evidence, not full Linux product
   qualification.

8. **Publish.** For a stable `vX.Y.Z`, dispatch
   [`publish-stable.yml`](workflows/publish-stable.yml) with the same tag. It
   downloads the existing draft, verifies the complete asset set, updater
   manifest, signatures, exact candidate CI/Release runs and attestations, then
   marks the release non-draft and non-prerelease. It never rebuilds or replaces
   assets. The newest stable candidate becomes the latest release, so
   `releases/latest/download/latest.json` serves the updater feed; an older
   candidate cannot silently downgrade that feed.

   Prerelease tags are not stable publication input. After a stable release exists, publish betas with "Set as a pre-release"
   checked and "Set as the latest release" unchecked: both the website and the
   updater resolve through `releases/latest`, which skips pre-releases.

To rebuild a tag before publication, dispatch **Release** from that same tag and
pass it in the `tag` input. The workflow refuses a mismatched workflow ref or
checkout commit. An existing draft may be replaced; a published release is never
touched. For an urgent release, the `emergency` dispatch input skips only the
component suite and the generated notes visibly disclose that bypass. Main CI,
security, build and signing checks still gate the draft.

## Published assets

| Asset | Purpose |
| --- | --- |
| `Quasar-macOS-arm64.dmg` | macOS download |
| `Quasar-macOS-arm64.app.tar.gz` + `.sig` | macOS updater payload |
| `Quasar-Windows-x64-setup.exe` + `.sig` | Windows download and updater payload |
| `Quasar-Linux-x86_64.AppImage` | Linux manual download |
| `Quasar-Linux-runtime-sources.tar` | Corresponding Linux runtime source archive |
| `latest.json` | Updater manifest for macOS and Windows only |
| `SHA256SUMS` | Digests of every other asset |

`latest.json` uses the Tauri v2 static updater format and points only to assets
under the same tag in `fordft/Quasar`. It contains the base updater targets
`darwin-aarch64` and `windows-x86_64`; the upstream bundle-specific aliases are
retained for the same two artifacts. Windows updater signatures bind the final
bytes, release version and `windows-x86_64-nsis` target. Renaming a file does
not invalidate a minisign signature, but publication verification requires the
exact asset names above.

## What users receive

- **macOS (Apple Silicon):** automatic updates are enabled for release builds.
  With the complete Apple secret set, the app is Developer ID signed, notarized
  and stapled. Without that set, it is ad-hoc signed and not notarized, so macOS
  may require approval in Privacy & Security. The release notes state which
  configuration was used.
- **Windows (x64):** automatic updates are enabled for release builds. With the
  complete SignPath configuration, the installer is Authenticode signed and
  timestamped. Without it, the installer is unsigned and SmartScreen reputation
  applies. The release notes state which configuration was used.
- **Linux (x86_64):** the AppImage remains a browsing-only manual download. It
  is excluded from `latest.json` and does not automatically update; users install
  a newer AppImage themselves. AI and Work remain unavailable in the Linux
  preview configuration.

Preview workflow `quasar-preview-<run>` builds remain the test channel. They are
prereleases, never marked latest, and do not produce updater payloads. They are
not a substitute for reviewed stable releases.

## Recovery

- **Before publishing**, a draft can be deleted and recreated after the cause is
  fixed. If the tagged source was wrong, delete and recreate the tag before
  publication, or prefer a higher version.
- **After publishing**, never replace assets, edit binaries, delete the release
  or move/delete the tag. Users and the updater may already hold those bytes.
  Fix forward with a higher version.
- **If signing configuration changes**, make the secret set complete or empty
  before starting the release. The gate fails partial sets rather than silently
  changing trust.
