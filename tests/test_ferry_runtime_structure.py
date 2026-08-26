import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "ferry-runtime"


def test_runtime_sidecar_name_is_consistent_and_keeps_windows_packaging():
    package = json.loads((RUNTIME / "package.json").read_text(encoding="utf-8"))
    assert package["bin"] == {
        "ferry-runtime": "dist/server/server.js",
    }

    tauri = json.loads(
        (ROOT / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8")
    )
    assert "binaries/ferry-runtime" in tauri["bundle"]["externalBin"]

    host = (ROOT / "app/src-tauri/src/runtime/mod.rs").read_text(encoding="utf-8")
    assert 'bundled_sidecar_command(resource_dir, "ferry-runtime")' in host
    assert "ferry-runtime/dist/server/server.js" in host
    command = (
        ROOT / "app/src-tauri/src/process/command.rs"
    ).read_text(encoding="utf-8")
    assert 'executable_name_for("ferry-runtime", true)' in command
    assert '"ferry-runtime.exe"' in command

    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    assert "ferry-runtime-x86_64-pc-windows-msvc.exe" in workflow
    assert "working-directory: ferry-runtime" in workflow
