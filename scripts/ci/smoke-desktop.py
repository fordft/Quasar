#!/usr/bin/env python3
"""Verify that this checkout's packaged app reveals a native window."""
import json
import http.server
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time


def smoke_linux_browsing(window):
    """Navigate through the real omnibox and execute JS in an untrusted page."""
    ready = threading.Event()
    result = {}

    class Page(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.startswith("/ready?ipc="):
                result["page_javascript_executed"] = True
                result["trusted_ipc_exposed"] = self.path != "/ready?ipc=0"
                ready.set()
                self.send_response(204)
                self.end_headers()
            elif self.path == "/browse":
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.end_headers()
                self.wfile.write(b'''<!doctype html><title>Quasar browsing smoke</title>
<h1>Quasar can browse</h1><p id="result"></p><script>
document.getElementById('result').textContent = 'JavaScript works';
fetch('/ready?ipc=' + Number('__TAURI_INTERNALS__' in window || '__TAURI__' in window));
</script>''')
            else:
                self.send_error(404)

        def log_message(self, *args):
            pass

    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page) as server:
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            subprocess.run(["xdotool", "windowfocus", "--sync", window], check=True)
            # A fresh Linux profile has five onboarding steps and no AI setup.
            # Enter advances their primary action, including Skip Import.
            for _ in range(7):
                subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], check=True)
                time.sleep(2)
            subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+l"], check=True)
            subprocess.run([
                "xdotool", "type", "--clearmodifiers", "--delay", "15",
                f"http://127.0.0.1:{server.server_port}/browse",
            ], check=True)
            subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], check=True)
            if not ready.wait(20):
                raise RuntimeError("browser did not load and execute the test page")
            if result["trusted_ipc_exposed"]:
                raise RuntimeError("trusted application IPC was exposed to the test page")
            return result
        finally:
            server.shutdown()
            worker.join(timeout=5)


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
                browsing = smoke_linux_browsing(window) if sys.platform == "linux" else {}
                (evidence / "result.json").write_text(json.dumps({
                    "binary": str(binary), "platform": sys.platform,
                    "pid": app.pid, "window": window, "visible": True,
                    **browsing,
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
