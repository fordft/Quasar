#!/usr/bin/env python3
"""Keep the AppImage's WebKit helpers paired with its bundled engine."""
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    appdir = Path(sys.argv[1]).resolve(strict=True)
    sdk = Path("target/linux-runtime/root").resolve(strict=True)
    # Always ship the complete reviewed library cohort. The first linuxdeploy
    # pass can use host libraries when a cached SDK contains stale symlinks;
    # replacing only WebKit would then leave its newer codec dependencies out.
    for library in sorted((sdk / "usr/lib/x86_64-linux-gnu").glob("lib*.so.*")):
        if library.is_symlink():
            continue
        soname = subprocess.check_output(
            ["patchelf", "--print-soname", str(library)], text=True).strip()
        if not soname or Path(soname).name != soname:
            raise RuntimeError(f"invalid SDK library SONAME: {library.name}")
        destination = appdir / "usr/lib" / soname
        destination.unlink(missing_ok=True)
        shutil.copy2(library, destination)
        subprocess.run([
            "patchelf", "--set-rpath", "$ORIGIN", str(destination),
        ], check=True)
    helpers = Path("usr/lib/x86_64-linux-gnu/webkit2gtk-4.1")
    # Tauri copies helpers from /usr, even when linuxdeploy resolves the
    # shared engine from LD_LIBRARY_PATH. Replace that set as one unit.
    shutil.copytree(sdk / helpers, appdir / helpers, dirs_exist_ok=True)
    for helper in ("WebKitWebProcess", "WebKitNetworkProcess"):
        subprocess.run([
            "patchelf", "--set-rpath", "$ORIGIN/..:$ORIGIN/../..",
            str(appdir / helpers / helper),
        ], check=True)
    # Tauri's GTK plugin rewrites every /usr string, including sandbox mount
    # points. Start from the verified original and relocate only WebKit's own
    # helper/injected-bundle directory. Preserve all sandbox paths and rules.
    engine = appdir / "usr/lib/libwebkit2gtk-4.1.so.0"
    original = (sdk / "usr/lib/x86_64-linux-gnu/libwebkit2gtk-4.1.so.0").read_bytes()
    helper_path = b"/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1"
    if helper_path not in original:
        raise RuntimeError("cannot locate WebKit helper directory")
    engine.write_bytes(original.replace(
        helper_path, b"././/lib/x86_64-linux-gnu/webkit2gtk-4.1"))
    subprocess.run(["patchelf", "--set-rpath", "$ORIGIN", str(engine)], check=True)
    licenses = appdir / "usr/share/doc/quasar-linux-runtime"
    shutil.copytree(sdk / "usr/share/doc", licenses, dirs_exist_ok=True)
    shutil.copy2("scripts/release/linux-runtime.json", licenses)
    # Debian copyright files refer to these full license texts.
    shutil.copytree("/usr/share/common-licenses", appdir / "usr/share/common-licenses",
                    dirs_exist_ok=True)
    (licenses / "README.txt").write_text(
        "WebKitGTK 2.54.2 and its matching JavaScriptCore and subprocesses.\n"
        "Built from Debian binary packages pinned by SHA-256 in linux-runtime.json.\n"
        "Helper RPATHs and the WebKit helper/injected-bundle directory are "
        "relocated. Sandbox paths and rules remain unchanged.\n"
        "Corresponding source: https://deb.debian.org/debian/pool/main/w/webkit2gtk/\n"
        "webkit2gtk_2.54.2.orig.tar.xz and webkit2gtk_2.54.2-2.debian.tar.xz\n"
        "Codec source directories: pool/main/liba/libavif/, "
        "pool/main/libj/libjpeg-turbo/, pool/main/libw/libwebp/.\n"
        "All package versions and copyright/license notices are included here.\n")

    # Build from Tauri's existing AppDir after replacing the helpers; do not
    # run its GTK plugin again (it has already relocated the engine paths).
    cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    linuxdeploy = cache / "tauri/linuxdeploy-x86_64.AppImage"
    images = list(appdir.parent.glob("app.quasar_*.AppImage"))
    if len(images) != 1:
        raise RuntimeError(f"expected one Quasar AppImage, found {len(images)}")
    output = images[0]
    output.unlink()
    env = os.environ.copy()
    env.update(OUTPUT=str(output), ARCH="x86_64", APPIMAGE_EXTRACT_AND_RUN="1")
    env["LD_LIBRARY_PATH"] = str(appdir / "usr/lib")
    subprocess.run([
        str(linuxdeploy), "--appimage-extract-and-run", "--appdir", str(appdir),
        "--output", "appimage",
    ], env=env, check=True)
    print(f"Finalized {output}")


if __name__ == "__main__":
    main()
