# Releasing Quasar

Pushing a `v*` tag runs [`release.yml`](workflows/release.yml). It builds and
verifies macOS (Apple Silicon) and Windows (x64), then creates a **draft**
GitHub release. Nothing is public until a maintainer publishes the draft.

Quasar's in-app updater is disabled. Configure Quasar's own verification key
and endpoint in `desktop/tauri.conf.json`, then enable the platform support gate
in `desktop/src/updates.rs` only after signed updates have been qualified.
The upstream Zephium signing key and update feed are not used by this fork.

## One-time setup

### Secrets

Add these under **Settings → Secrets and variables → Actions** as repository
secrets. The release gate fails with the names of any that are missing.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the Developer ID Application `.p12` (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Name> (<TEAMID>)` |
| `APPLE_TEAM_ID` | The 10-character team ID; the workflow checks the app is signed by it |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID (Users and Access → Integrations → Keys) |
| `APPLE_API_KEY` | That API key's key ID |
| `APPLE_API_KEY_PATH` | The **contents** of `AuthKey_<KEYID>.p8`; the workflow writes it to disk |
| `TAURI_SIGNING_PRIVATE_KEY` | The updater private key (file contents) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password |

The API key needs the Developer role to notarize.

### Updater key

Generate the minisign key pair once and keep the private key and password in a
password manager:

```sh
pnpm --dir desktop tauri signer generate -w ~/.tauri/zephium-updater.key
```

The public key (the `.pub` file contents) is embedded in
`desktop/tauri.conf.json` at `plugins.updater.pubkey`. Installed copies trust
only that key: losing the private key or changing the public key strands every
existing install on its current version.

### Repository

- The repository must be public: build provenance attestations and the
  `releases/latest/download/…` links depend on it.
- Protect `v*` tags with a ruleset so only maintainers can create them.

## Cutting a release

1. **Refresh the blocker seed.** EasyList asks to be refreshed every four days,
   and the release gate rejects a seed past that point, so do this right
   before tagging:

   ```sh
   curl -fsSL -o /tmp/easylist.txt https://easylist.to/easylist/easylist.txt
   curl -fsSL -o /tmp/easyprivacy.txt https://easylist.to/easylist/easyprivacy.txt
   cargo xtask update-blocker-seed --easylist /tmp/easylist.txt \
     --easyprivacy /tmp/easyprivacy.txt \
     --license assets/blocker-seed/v1/LICENSE-CC-BY-SA-3.0.txt
   ```

   Then copy the new WebKit digest into the native seed probe and its policy
   test, which pin it (`scripts/ci/probe_macos_blocker_seed.swift` and
   `scripts/ci/test_macos_blocker_seed_probe.sh`):

   ```sh
   jq -r '.compilers[] | select(.target == "webkit") | .native_artifact_sha256' \
     assets/blocker-seed/v1/compile-report.json
   ```

2. **Bump the version** in `desktop/Cargo.toml` (`[package] version`, e.g.
   `1.0.0-beta.1`). It is the only version source: `tauri.conf.json` has none.
   Run `cargo check -p zephium-desktop` so `Cargo.lock` follows.
3. **Run the release gate locally.** It is the same check the workflow runs
   first: strict engine-floor and advisory reviews, seed freshness, and
   outstanding vendor fixes.

   ```sh
   cargo xtask check-release-engine-security
   ```

4. Merge to `main` with CI green.
5. **Tag and push** the merged commit:

   ```sh
   git tag -a v1.0.0-beta.1 -m "Quasar 1.0.0-beta.1"
   git push origin v1.0.0-beta.1
   ```

6. **Review the draft** once the workflow finishes: install the DMG and the
   Windows installer, check the notes, then **Publish**. Keep "Set as the
   latest release" checked and leave "Set as a pre-release" unchecked, even for
   betas: `releases/latest` skips pre-releases, and both the website and the
   updater resolve through it.

To rebuild a tag (for example after a flaky runner), run **Actions → Release →
Run workflow** with the tag, or `gh workflow run release.yml -f tag=v1.0.0-beta.1`.
An existing draft for that tag is replaced; a published release is never
touched.

## Published assets

The website links `releases/latest/download/<name>`, so these names are a
contract:

| Asset | Purpose |
| --- | --- |
| `Quasar-macOS-arm64.dmg` | macOS download |
| `Quasar-macOS-arm64.app.tar.gz` + `.sig` | macOS updater payload |
| `Quasar-Windows-x64-setup.exe` + `.sig` | Windows download and updater payload |
| `latest.json` | Updater manifest (`darwin-aarch64[-app]`, `windows-x86_64[-nsis]`) |
| `SHA256SUMS` | Digests of every other asset |

Renaming does not invalidate an updater signature: minisign signs the bytes,
and the build-time file name in its trusted comment is not checked by the
updater.

## What the workflow verifies

- The tag is `v` + the `zephium-desktop` version, valid semver, and
  `tauri.conf.json` carries no competing version.
- `cargo xtask check-release-engine-security` passes.
- macOS: `codesign --verify --deep --strict`, the signing team is
  `APPLE_TEAM_ID`, Gatekeeper (`spctl`) accepts the app, the app and the DMG
  are notarized and stapled, the main executable's entitlements are exactly
  camera and microphone, and no other Mach-O carries any.
- Both updater signatures verify against the public key in
  `desktop/tauri.conf.json`, and the asset set is exactly the one above.
- Build provenance is attested for the DMG, the updater archive and the
  installer (`gh attestation verify <file> --repo fordft/Quasar`).

The macOS dSYM and Windows PDB are kept as workflow artifacts for 90 days.
Download them for any release you need to symbolicate later.

## Windows signing

The Windows installer is not Authenticode-signed yet, so SmartScreen warns on
first run. When SignPath is added, signing goes in the marked spot in the
Windows job and must come **before** the updater signature: Authenticode
changes the installer's bytes, so sign it first, then replace Tauri's `.sig`
by running `pnpm tauri signer sign` over the signed file.

Until then, SmartScreen judges each installer by its own download reputation.
After publishing, submit `Quasar-Windows-x64-setup.exe` at
<https://www.microsoft.com/wdsi/filesubmission> as a software developer
("Incorrectly detected as malware/malicious" when it is flagged). Keep the
asset name and download URL unchanged between releases, and keep the bundle's
`publisher` at `Quasar`: Windows keys the per-user install by it.

## Recovery

- **Before publishing**, anything can be redone: delete the draft, fix the
  cause, and rerun. If the tagged source was wrong, delete and recreate the
  tag, or simply bump the version.
- **After publishing**, never replace assets, edit binaries, or delete the
  release or tag. Users and the updater may already hold those bytes. Fix
  forward with a higher version (`1.0.0-beta.2`).
- If the updater private key leaks, ship a release whose embedded key is new,
  signed with the old key, and treat it as a security incident.
