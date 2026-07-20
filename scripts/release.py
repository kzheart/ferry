#!/usr/bin/env python3
"""Version and release helpers used locally and by GitHub Actions."""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "app/package.json"
PACKAGE_LOCK = ROOT / "app/package-lock.json"
CARGO = ROOT / "app/src-tauri/Cargo.toml"
CARGO_LOCK = ROOT / "app/src-tauri/Cargo.lock"
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")


def package_version() -> str:
    return json.loads(PACKAGE.read_text(encoding="utf-8"))["version"]


def cargo_version(path: Path = CARGO) -> str:
    return tomllib.loads(path.read_text(encoding="utf-8"))["package"]["version"]


def replace_package_version(path: Path, version: str) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    data["version"] = version
    if path == PACKAGE_LOCK and data.get("packages", {}).get("") is not None:
        data["packages"][""]["version"] = version
    path.write_text(json.dumps(data, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def replace_cargo_package_version(path: Path, version: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(
        r"(?ms)(^\[package\]\s.*?^version\s*=\s*)\"[^\"]+\"",
        rf'\g<1>"{version}"', text, count=1,
    )
    if count != 1:
        raise ValueError(f"cannot locate [package] version in {path}")
    path.write_text(updated, encoding="utf-8")


def replace_cargo_lock_version(path: Path, version: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(
        r'(?ms)(^\[\[package\]\]\s+name\s*=\s*"ferry"\s+version\s*=\s*)"[^"]+"',
        rf'\g<1>"{version}"', text, count=1,
    )
    if count != 1:
        raise ValueError(f"cannot locate ferry package version in {path}")
    path.write_text(updated, encoding="utf-8")


def check(tag: str | None = None) -> None:
    expected = package_version()
    if not SEMVER.fullmatch(expected):
        raise ValueError(f"invalid package version: {expected}")
    actual = cargo_version()
    if actual != expected:
        raise ValueError(f"Cargo.toml version {actual} != package.json version {expected}")
    lock = tomllib.loads(CARGO_LOCK.read_text(encoding="utf-8"))
    own = next((p for p in lock["package"] if p["name"] == "ferry"), None)
    if not own or own["version"] != expected:
        raise ValueError("Cargo.lock ferry version is not synchronized")
    package_lock = json.loads(PACKAGE_LOCK.read_text(encoding="utf-8"))
    if package_lock["version"] != expected or package_lock["packages"][""]["version"] != expected:
        raise ValueError("package-lock.json version is not synchronized")
    if tag is not None and tag != f"v{expected}":
        raise ValueError(f"tag {tag} != v{expected}")
    print(expected)


def bump(version: str) -> None:
    if not SEMVER.fullmatch(version):
        raise ValueError(f"invalid version: {version}")
    replace_package_version(PACKAGE, version)
    replace_package_version(PACKAGE_LOCK, version)
    replace_cargo_package_version(CARGO, version)
    replace_cargo_lock_version(CARGO_LOCK, version)
    check()


def release_config(output: Path, repository: str, pubkey: str, targets: list[str], thumbprint: str | None) -> None:
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("repository must be owner/name")
    if not pubkey.strip():
        raise ValueError("updater public key is required")
    config: dict = {
        "plugins": {"updater": {
            "endpoints": [f"https://github.com/{repository}/releases/latest/download/latest.json"],
            "pubkey": pubkey.strip(),
        }},
        "bundle": {"createUpdaterArtifacts": True, "targets": targets},
    }
    if thumbprint:
        config["bundle"]["windows"] = {
            "certificateThumbprint": thumbprint,
            "digestAlgorithm": "sha256",
        }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")


def latest(assets: Path, output: Path, repository: str, version: str, notes: str) -> None:
    base = f"https://github.com/{repository}/releases/download/v{version}"
    specs = {
        "darwin-aarch64": ("*.app.tar.gz", "*.app.tar.gz.sig"),
        "windows-x86_64": ("*.nsis.zip", "*.nsis.zip.sig"),
    }
    platforms = {}
    for platform, (artifact_glob, sig_glob) in specs.items():
        artifacts = list(assets.rglob(artifact_glob))
        signatures = list(assets.rglob(sig_glob))
        if len(artifacts) != 1 or len(signatures) != 1:
            raise ValueError(f"expected one artifact and signature for {platform}")
        platforms[platform] = {
            "signature": signatures[0].read_text(encoding="utf-8").strip(),
            "url": f"{base}/{artifacts[0].name}",
        }
    manifest = {"version": version, "notes": notes, "pub_date": os.environ.get("RELEASE_PUB_DATE"), "platforms": platforms}
    if not manifest["pub_date"]:
        raise ValueError("RELEASE_PUB_DATE is required for deterministic manifest generation")
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    check_parser = commands.add_parser("check")
    check_parser.add_argument("--tag")
    bump_parser = commands.add_parser("bump")
    bump_parser.add_argument("version")
    config_parser = commands.add_parser("config")
    config_parser.add_argument("--output", type=Path, required=True)
    config_parser.add_argument("--repository", required=True)
    config_parser.add_argument("--pubkey", required=True)
    config_parser.add_argument("--targets", nargs="+", required=True, choices=("app", "dmg", "nsis"))
    config_parser.add_argument("--windows-thumbprint")
    latest_parser = commands.add_parser("latest")
    latest_parser.add_argument("--assets", type=Path, required=True)
    latest_parser.add_argument("--output", type=Path, required=True)
    latest_parser.add_argument("--repository", required=True)
    latest_parser.add_argument("--version", required=True)
    latest_parser.add_argument("--notes", default="")
    args = parser.parse_args()
    if args.command == "check": check(args.tag)
    elif args.command == "bump": bump(args.version)
    elif args.command == "config": release_config(args.output, args.repository, args.pubkey, args.targets, args.windows_thumbprint)
    elif args.command == "latest": latest(args.assets, args.output, args.repository, args.version, args.notes)


if __name__ == "__main__":
    try:
        main()
    except (KeyError, ValueError, StopIteration) as exc:
        print(f"release error: {exc}", file=sys.stderr)
        raise SystemExit(1)
