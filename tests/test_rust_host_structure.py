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


def test_engine_and_runtime_share_one_process_supervisor():
    supervisor = HOST / "process/supervisor.rs"
    assert supervisor.is_file()
    assert "struct ManagedProcess" in supervisor.read_text()
    assert "struct ProcessSupervisor" in supervisor.read_text()
    command = HOST / "process/command.rs"
    assert command.is_file()
    command_source = command.read_text()
    assert "fn bundled_sidecar_command" in command_source
    assert "fn configure_background" in command_source
    assert "creation_flags" in command_source

    for relative_path in ("engine/mod.rs", "runtime/mod.rs"):
        source = (HOST / relative_path).read_text()
        assert "ProcessSupervisor" in source
        assert "ManagedProcess" in source
        assert "bundled_sidecar_command" in source
        assert "configure_background" in source
        assert 'target_os = "windows"' not in source
        assert ".exe" not in source
        assert "impl Drop for" not in source
        assert "Mutex<Option<" not in source


def test_runtime_gateway_and_approval_are_separate_capabilities():
    runtime = HOST / "runtime"
    assert {
        "approval.rs",
        "gateway.rs",
        "mod.rs",
        "tool_routes.rs",
    } <= {path.name for path in runtime.glob("*.rs")}

    root = (runtime / "mod.rs").read_text()
    assert "fn resolve_tool_request" not in root
    assert "fn apply_operation_plan" not in root
    assert "static AUTO_SESSIONS" not in root


def test_engine_tests_do_not_hide_the_production_entrypoint():
    engine = HOST / "engine"
    assert (engine / "tests.rs").is_file()
    root = (engine / "mod.rs").read_text()
    assert "mod tests;" in root
    assert "fn operation_inputs_are_strictly_validated" not in root
