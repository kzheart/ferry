import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "ferry-runtime"


def test_runtime_source_is_grouped_by_responsibility():
    expected = {
        "agents",
        "organizing",
        "providers",
        "roles",
        "runtime",
        "security",
        "server",
        "sessions",
        "tools",
    }
    directories = {
        path.name for path in (RUNTIME / "src").iterdir() if path.is_dir()
    }
    assert expected <= directories
    assert not {
        "application",
        "core",
        "infrastructure",
        "protocol",
        "workflows",
    } & directories
    assert not {
        path.name for path in (RUNTIME / "src").glob("*.ts")
    }
    security_files = {
        path.name for path in (RUNTIME / "src/security").glob("*.ts")
    }
    assert security_files == {"limits.ts"}
    limits = RUNTIME / "src/security/limits.ts"
    assert limits.is_file()
    assert "function truncateText" in limits.read_text()
    assert (
        RUNTIME / "src/providers/provider-config-store.ts"
    ).is_file()
    assert (
        RUNTIME / "src/providers/provider-config-validation.ts"
    ).is_file()
    assert (RUNTIME / "src/providers/provider-models.ts").is_file()
    assert (RUNTIME / "src/providers/provider-service.ts").is_file()
    assert (RUNTIME / "src/providers/commands.ts").is_file()
    assert (RUNTIME / "src/roles/role-service.ts").is_file()
    assert (RUNTIME / "src/roles/role-store.ts").is_file()
    assert not (RUNTIME / "src/roles/role-repository.ts").exists()
    assert (RUNTIME / "src/roles/commands.ts").is_file()
    assert (RUNTIME / "src/runtime/event-bus.ts").is_file()
    assert (RUNTIME / "src/tools/gateway.ts").is_file()
    assert (RUNTIME / "src/agents/delegation-runner.ts").is_file()
    assert (RUNTIME / "src/organizing/coordinator.ts").is_file()
    runtime_session = RUNTIME / "src/sessions/runtime-session.ts"
    assert runtime_session.is_file()
    assert (RUNTIME / "src/sessions/session-store.ts").is_file()
    assert not (RUNTIME / "src/sessions/session-repository.ts").exists()
    assert (RUNTIME / "src/sessions/commands.ts").is_file()
    assert "export class RuntimeSession" in runtime_session.read_text()


def test_runtime_sidecar_name_is_consistent_and_keeps_windows_packaging():
    package = json.loads((RUNTIME / "package.json").read_text())
    assert package["bin"] == {
        "ferry-runtime": "dist/server/server.js",
    }

    tauri = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text())
    assert "binaries/ferry-runtime" in tauri["bundle"]["externalBin"]

    host = (ROOT / "app/src-tauri/src/runtime/mod.rs").read_text()
    assert 'bundled_sidecar_command(resource_dir, "ferry-runtime")' in host
    assert "ferry-runtime/dist/server/server.js" in host
    command = (
        ROOT / "app/src-tauri/src/process/command.rs"
    ).read_text()
    assert 'executable_name_for("ferry-runtime", true)' in command
    assert '"ferry-runtime.exe"' in command

    workflow = (ROOT / ".github/workflows/ci.yml").read_text()
    assert "ferry-runtime-x86_64-pc-windows-msvc.exe" in workflow
    assert "working-directory: ferry-runtime" in workflow
