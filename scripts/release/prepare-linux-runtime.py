#!/usr/bin/env python3
"""Fetch a reviewed WebKitGTK runtime without modifying the host installation."""
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import urllib.request


def main():
    manifest = Path(__file__).with_name("linux-runtime.json")
    packages = json.loads(manifest.read_text())
    target = Path("target/linux-runtime")
    downloads = target / "downloads"
    root = target / "root"
    downloads.mkdir(parents=True, exist_ok=True)
    root.mkdir(parents=True, exist_ok=True)
    for package in packages:
        archive = downloads / Path(package["filename"]).name
        if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != package["sha256"]:
            url = "https://deb.debian.org/debian/" + package["filename"]
            with urllib.request.urlopen(url, timeout=120) as response:
                archive.write_bytes(response.read())
        if hashlib.sha256(archive.read_bytes()).hexdigest() != package["sha256"]:
            raise RuntimeError(f"checksum mismatch: {archive.name}")
        actual = subprocess.check_output(
            ["dpkg-deb", "-f", str(archive), "Version"], text=True).strip()
        if actual != package["version"]:
            raise RuntimeError(f"version mismatch: {archive.name}")
        subprocess.run(["dpkg-deb", "-x", str(archive), str(root)], check=True)
        print(f"Verified {package['package']} {actual}")
    (target / "linux-runtime.json").write_text(manifest.read_text())
    sources = target / "sources"
    sources.mkdir(exist_ok=True)
    source_manifest = manifest.with_name("linux-runtime-sources.json")
    for entry in json.loads(source_manifest.read_text()):
        archive = sources / Path(entry["filename"]).name
        if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != entry["sha256"]:
            with urllib.request.urlopen(
                    "https://deb.debian.org/debian/" + entry["filename"], timeout=120) as response:
                archive.write_bytes(response.read())
        if hashlib.sha256(archive.read_bytes()).hexdigest() != entry["sha256"]:
            raise RuntimeError(f"source checksum mismatch: {archive.name}")
    (sources / "linux-runtime-sources.json").write_text(source_manifest.read_text())
    with tarfile.open(target / "Quasar-Linux-runtime-sources.tar", "w") as bundle:
        bundle.add(sources, arcname="sources")
    print(f"Runtime library path: {root.resolve() / 'usr/lib/x86_64-linux-gnu'}")


if __name__ == "__main__":
    main()
