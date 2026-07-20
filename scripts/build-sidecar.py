#!/usr/bin/env python3
"""Build and place the native PyInstaller sidecar for Tauri."""
import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {("Darwin", "arm64"): "aarch64-apple-darwin", ("Windows", "AMD64"): "x86_64-pc-windows-msvc"}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=TARGETS.values())
    parser.add_argument("--clean", action="store_true")
    args = parser.parse_args()
    target = args.target or TARGETS.get((platform.system(), platform.machine()))
    if target is None:
        raise SystemExit("only native macOS aarch64 and Windows x64 builds are supported")
    if sys.version_info[:2] != (3, 12):
        raise SystemExit("Python 3.12 is required")
    command = [sys.executable, "-m", "PyInstaller", "--noconfirm"]
    if args.clean:
        command.append("--clean")
    command += ["--distpath", str(ROOT / "dist"), "--workpath", str(ROOT / "build/pyinstaller"), str(ROOT / "ferry-sidecar.spec")]
    subprocess.run(command, check=True)
    extension = ".exe" if target.endswith("windows-msvc") else ""
    source = ROOT / "dist" / f"ferry-engine{extension}"
    destination = ROOT / "app/src-tauri/binaries" / f"ferry-engine-{target}{extension}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    print(destination)


if __name__ == "__main__":
    main()
