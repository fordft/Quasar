#!/usr/bin/env python3
"""Verify that this checkout's packaged app reveals a native window."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


def main():
    binary = Path(sys.argv[1]).resolve(strict=True)
    evidence = Path("target/preview-smoke")
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="quasar-smoke-") as scratch:
        scratch = Path(scratch)
        env = os.environ.copy()
        if sys.platform == "linux":
            for key, directory in [
                ("XDG_DATA_HOME", "data"),
                ("XDG_CONFIG_HOME", "config"),
                ("XDG_CACHE_HOME", "cache"),
            ]:
                location = scratch / directory
                location.mkdir(mode=0o700)
                env[key] = str(location)
            env["GDK_BACKEND"] = "x11"
            env["WEBKIT_DISABLE_DMABUF_RENDERER"] = "1"
        else:
            swift = scratch / "windows.swift"
            swift.write_text('''import CoreGraphics
import Foundation
let pid = Int(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let found = windows.contains { ($0[kCGWindowOwnerPID as String] as? Int) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
exit(found ? 0 : 1)
''')
            probe = scratch / "windows"
            subprocess.run(["swiftc", str(swift), "-o", str(probe)], check=True)
        with (evidence / "startup.log").open("w") as log:
            app = subprocess.Popen([str(binary)], env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
            try:
                deadline = time.monotonic() + 60
                window = None
                while time.monotonic() < deadline:
                    if app.poll() is not None:
                        raise RuntimeError(f"app exited before revealing its window: {app.returncode}")
                    if sys.platform == "linux":
                        result = subprocess.run(
                            ["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)],
                            text=True, capture_output=True, check=False)
                        if result.returncode == 0 and result.stdout.strip():
                            window = result.stdout.splitlines()[0]
                            break
                    elif subprocess.run([str(probe), str(app.pid)], check=False).returncode == 0:
                        window = "native-window"
                        break
                    time.sleep(1)
                if window is None:
                    raise RuntimeError("no visible application window within 60 seconds")
                time.sleep(3)
                if app.poll() is not None:
                    raise RuntimeError("app exited immediately after startup")
                (evidence / "result.json").write_text(json.dumps({
                    "binary": str(binary), "platform": sys.platform,
                    "pid": app.pid, "window": window, "visible": True,
                }, indent=2) + "\n")
                print(f"Native startup passed: {binary.name} revealed its own window.")
            finally:
                if app.poll() is None:
                    os.killpg(app.pid, signal.SIGTERM)
                    try:
                        app.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(app.pid, signal.SIGKILL)
                        app.wait(timeout=5)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"Desktop smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
