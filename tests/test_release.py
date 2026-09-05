import json

import pytest

from scripts import release


def test_release_workflow_builds_and_publishes_windows_nsis():
    workflow = (release.ROOT / ".github/workflows/release.yml").read_text(
        encoding="utf-8"
    )
    assert "build-windows:" in workflow
    assert "runs-on: windows-latest" in workflow
    assert "--targets nsis" in workflow
    assert "bundle/nsis/*-setup.exe" in workflow
    assert "bundle/nsis/*-setup.exe.sig" in workflow
    assert "needs: [validate, build-macos, build-windows]" in workflow
    assert "release-assets/**/*.exe" in workflow


def test_release_config_accepts_windows_nsis(tmp_path):
    output = tmp_path / "release.json"
    release.release_config(output, "owner/ferry", "public-key", ["nsis"])

    config = json.loads(output.read_text(encoding="utf-8"))
    assert config["bundle"] == {
        "createUpdaterArtifacts": True,
        "targets": ["nsis"],
    }


def test_latest_manifest_contains_macos_and_windows(tmp_path, monkeypatch):
    assets = tmp_path / "assets"
    assets.mkdir()
    files = {
        "Ferry.app.tar.gz": "mac-app",
        "Ferry.app.tar.gz.sig": "mac-signature\n",
        "Ferry_0.9.0_x64-setup.exe": "windows-installer",
        "Ferry_0.9.0_x64-setup.exe.sig": "windows-signature\n",
    }
    for name, content in files.items():
        (assets / name).write_text(content, encoding="utf-8")
    output = tmp_path / "latest.json"
    monkeypatch.setenv("RELEASE_PUB_DATE", "2026-08-27T00:00:00+00:00")

    release.latest(assets, output, "owner/ferry", "0.9.0", "Windows support")

    manifest = json.loads(output.read_text(encoding="utf-8"))
    assert manifest["platforms"] == {
        "darwin-aarch64": {
            "signature": "mac-signature",
            "url": "https://github.com/owner/ferry/releases/download/v0.9.0/Ferry.app.tar.gz",
        },
        "windows-x86_64": {
            "signature": "windows-signature",
            "url": "https://github.com/owner/ferry/releases/download/v0.9.0/Ferry_0.9.0_x64-setup.exe",
        },
    }


def test_latest_manifest_rejects_missing_windows_artifact(tmp_path, monkeypatch):
    assets = tmp_path / "assets"
    assets.mkdir()
    (assets / "Ferry.app.tar.gz").write_text("mac-app", encoding="utf-8")
    (assets / "Ferry.app.tar.gz.sig").write_text("mac-signature", encoding="utf-8")
    monkeypatch.setenv("RELEASE_PUB_DATE", "2026-08-27T00:00:00+00:00")

    with pytest.raises(ValueError, match="windows-x86_64"):
        release.latest(assets, tmp_path / "latest.json", "owner/ferry", "0.9.0", "")
