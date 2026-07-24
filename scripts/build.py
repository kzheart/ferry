#!/usr/bin/env python3
"""Build both sidecars and the native Ferry desktop bundle."""
from __future__ import annotations

import argparse
import platform
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "ferry-runtime"
APP = ROOT / "app"
TARGETS = {
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Windows", "AMD64"): "x86_64-pc-windows-msvc",
    ("Windows", "x86_64"): "x86_64-pc-windows-msvc",
}
NODE_MINIMUM = (22, 19, 0)


def native_target(system: str, machine: str) -> str:
    try:
        return TARGETS[(system, machine)]
    except KeyError as error:
        raise ValueError(
            f"不支持的构建主机: {system}/{machine}"
        ) from error


def parse_node_version(value: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)", value.strip())
    if match is None:
        raise ValueError(f"无法识别 Node 版本: {value.strip()}")
    return tuple(map(int, match.groups()))


def run(command: list[str], *, cwd: Path = ROOT) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def verify_toolchain(target: str) -> None:
    host = native_target(platform.system(), platform.machine())
    if target != host:
        raise ValueError(
            f"sidecar 必须原生构建: 请求 {target}, 当前主机 {host}"
        )
    if sys.version_info[:2] != (3, 12):
        raise ValueError("Python 3.12 is required")
    node = subprocess.run(
        ["node", "--version"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    if parse_node_version(node) < NODE_MINIMUM:
        raise ValueError("Node.js 22.19.0 or newer is required")
    subprocess.run(
        ["cargo", "--version"],
        check=True,
        capture_output=True,
        text=True,
    )


def build(target: str, *, install: bool = True) -> None:
    verify_toolchain(target)
    if install:
        run(["npm", "ci"], cwd=RUNTIME)
        run(["npm", "ci"], cwd=APP)
    run(["npm", "run", "build:sea", "--", target], cwd=RUNTIME)
    run([
        sys.executable,
        str(ROOT / "scripts/build-sidecar.py"),
        "--clean",
        "--target",
        target,
    ])
    run(["npm", "run", "tauri", "--", "build"], cwd=APP)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Build Ferry sidecars and desktop bundle",
    )
    parser.add_argument("--target", choices=sorted(set(TARGETS.values())))
    parser.add_argument(
        "--skip-install",
        action="store_true",
        help="reuse existing npm dependencies",
    )
    args = parser.parse_args()
    target = args.target or native_target(
        platform.system(),
        platform.machine(),
    )
    build(target, install=not args.skip_install)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"build error: {error}", file=sys.stderr)
        raise SystemExit(1)
