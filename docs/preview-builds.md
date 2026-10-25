# Quasar preview builds

The Preview workflow builds the fork's current `main` and publishes experimental
downloads when manually dispatched with `platform: both` and `publish: true`.
Ordinary pushes build artifacts without creating a release. The production
Release workflow remains separate and requires signing credentials.

## macOS

`Quasar-macOS-arm64.dmg` is for Apple Silicon Macs. Drag Quasar into Applications.
These previews use an ad-hoc signature and are not Apple-notarized. If macOS
blocks the first launch, use System Settings → Privacy & Security → Open Anyway
after attempting to open the app. Keep Gatekeeper enabled.

The browser uses the system WebKit and targets macOS 14 or later. Install
available macOS and Safari updates before testing; the app reports outdated
supported runtimes and rejects unsupported major versions. Intel Macs are not
included in this first preview.

## Linux

`Quasar-Linux-x86_64.AppImage` targets Ubuntu 26.04 or newer, with glibc 2.43 or
newer. Other distributions with compatible libraries may work but are not yet
qualified. Ubuntu 22.04/24.04 are not supported by this preview artifact.
The host must have `bubblewrap` and `xdg-dbus-proxy` installed for WebKit's
sandbox; the package uses their normal distro security profiles.

```sh
chmod +x Quasar-Linux-x86_64.AppImage
./Quasar-Linux-x86_64.AppImage
```

If FUSE is unavailable, extract the AppImage and run its launcher:

```sh
./Quasar-Linux-x86_64.AppImage --appimage-extract
./squashfs-root/AppRun
```

This is a browsing preview: AI and Work are disabled, including their settings
switches. Notes, tasks and the normal browser interface remain available.
WebKitGTK 2.54.2, JavaScriptCore and matching WebKit subprocesses are included;
the AppImage does not replace the host's WebKit installation. It retains the
engine's sandbox and runtime security checks. X11 startup is checked in CI;
Wayland and distro compatibility still need wider manual testing.

Debian runtime packages are pinned by version and SHA-256 in
`scripts/release/linux-runtime.json`. Their checksums were reviewed against the
signed Debian archive metadata. Copyright notices are included in the AppImage;
`Quasar-Linux-runtime-sources.tar` contains the corresponding WebKit and codec
source archives and Debian packaging patches.

## Verification and development

Each published preview includes `SHA256SUMS`. Compare the checksum of the file
you downloaded before opening it. Automatic updates are disabled; install each
new preview manually.

CI runs frontend checks, production asset builds, Rust formatting, core tests
and the emitted style gate before packaging. It then requires the packaged app
to reveal its own native window; macOS also requires signature verification.
Linux navigates through the real address bar to a loopback test page, requires
its JavaScript to run and verifies that trusted application IPC is absent there.
These focused checks do not establish complete browser or AI qualification.

Linux `pnpm dev` and `pnpm -C desktop build` automatically forward
`--no-default-features` to Cargo, leaving the unfinished AI/Work product off.
Development still requires Tauri's native dependencies and a matching installed
WebKitGTK/JavaScriptCore/process package set at version 2.54.0 or newer. A host
with only WebKitGTK 2.52 cannot run the native development binary. The preview
packaging scripts prepare a private runtime instead; do not bypass the runtime
floor or disable the sandbox to run a development build.
