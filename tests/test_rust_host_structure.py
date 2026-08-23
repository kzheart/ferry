import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_bundle_targets_stay_in_platform_configs():
    tauri = ROOT / "app/src-tauri"
    common = json.loads((tauri / "tauri.conf.json").read_text())
    macos = json.loads((tauri / "tauri.macos.conf.json").read_text())
    windows = json.loads(
        (tauri / "tauri.windows.conf.json").read_text()
    )

    assert "targets" not in common["bundle"]
    assert macos["bundle"]["targets"] == ["app", "dmg"]
    assert windows["bundle"]["targets"] == ["nsis", "msi"]
    assert windows["app"]["windows"][0]["transparent"] is False
