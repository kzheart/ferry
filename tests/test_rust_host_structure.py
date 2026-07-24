from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HOST = ROOT / "app/src-tauri/src"


def test_rust_host_uses_capability_packages():
    assert {
        "contracts",
        "desktop",
        "engine",
        "operations",
        "process",
        "runtime",
    } <= {
        path.name for path in HOST.iterdir() if path.is_dir()
    }
    for legacy in (
        "agent.rs",
        "sidecar.rs",
        "sidecar_policy.rs",
        "operation_commands.rs",
        "operation_input.rs",
        "operation_request.rs",
        "operation_validation.rs",
        "platform",
        "terminal.rs",
        "reveal.rs",
        "window.rs",
    ):
        assert not (HOST / legacy).exists()


def test_desktop_package_keeps_windows_platform_boundary():
    platform = HOST / "desktop/platform"
    assert (platform / "macos.rs").is_file()
    assert (platform / "windows.rs").is_file()
    assert (platform / "unsupported.rs").is_file()
