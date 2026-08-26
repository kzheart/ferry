#!/usr/bin/env python3
"""Build both sidecars and the native Ferry desktop bundle."""
from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "ferry-runtime"
APP = ROOT / "app"
# Rust 引擎是独立 package（自带 Cargo.lock），产物落在自己的 target/ 下。
ENGINE_CRATE = ROOT / "crates/ferry-engine"
ENGINE_MANIFEST = ENGINE_CRATE / "Cargo.toml"
BINARIES = APP / "src-tauri/binaries"
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


def executable_suffix(target: str) -> str:
    return ".exe" if target.endswith("windows-msvc") else ""


def engine_build_command(target: str) -> list[str]:
    return [
        "cargo",
        "build",
        "--release",
        "--locked",
        "--target",
        target,
        "--manifest-path",
        str(ENGINE_MANIFEST),
    ]


def tauri_build_command() -> list[str]:
    command = ["npm", "run", "tauri", "--", "build"]
    if not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
        # 本地构建没有发布私钥时仍应产出可安装的原生包。正式发布流水线注入
        # 私钥后沿用 tauri.conf.json，继续生成带签名的 updater artifacts。
        override = json.dumps(
            {"bundle": {"createUpdaterArtifacts": False}},
            separators=(",", ":"),
        )
        command.extend(["--config", override])
    return command


def engine_binary_paths(target: str) -> tuple[Path, Path]:
    """Rust 引擎产物路径与 Tauri externalBin 要求的落地路径。"""
    suffix = executable_suffix(target)
    source = ENGINE_CRATE / "target" / target / "release" / f"ferry-engine{suffix}"
    destination = BINARIES / f"ferry-engine-{target}{suffix}"
    return source, destination


def install_engine_binary(target: str) -> Path:
    """命名必须是 ferry-engine-<target>[.exe]：tauri.conf.json 的 externalBin 依赖它。"""
    source, destination = engine_binary_paths(target)
    if not source.is_file():
        raise ValueError(f"未找到引擎产物: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    print(destination)
    return destination


def run(command: list[str], *, cwd: Path = ROOT) -> None:
    # Windows 的 npm 入口是 npm.cmd。subprocess 在 shell=False 时不会可靠地按
    # PATHEXT 解析批处理文件,先展开成绝对路径后再执行。
    executable = shutil.which(command[0]) or command[0]
    subprocess.run([executable, *command[1:]], cwd=cwd, check=True)


def verify_rust_target(target: str) -> None:
    """rustup 缺目标会在 cargo build 阶段才报错，提前给出可执行的修复提示。"""
    try:
        installed = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        # 非 rustup 管理的工具链（发行版 cargo）无法枚举，交给 cargo 自己报错。
        return
    if target not in installed.split():
        raise ValueError(
            f"缺少 Rust target {target}: 请先运行 rustup target add {target}"
        )


def verify_toolchain(target: str) -> None:
    host = native_target(platform.system(), platform.machine())
    if target != host:
        raise ValueError(
            f"sidecar 必须原生构建: 请求 {target}, 当前主机 {host}"
        )
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
    verify_rust_target(target)


def build(target: str, *, install: bool = True) -> None:
    verify_toolchain(target)
    if install:
        run(["npm", "ci"], cwd=RUNTIME)
        run(["npm", "ci"], cwd=APP)
    run(["npm", "run", "build:sea", "--", target], cwd=RUNTIME)
    run(engine_build_command(target))
    install_engine_binary(target)
    run(tauri_build_command(), cwd=APP)


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
