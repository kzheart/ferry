from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "engine"


def test_adapter_contract_has_no_plugin_layer():
    assert (ENGINE / "adapters/contracts.py").is_file()
    for agent in ("claude", "codex", "opencode"):
        assert (ENGINE / f"adapters/{agent}/adapter.py").is_file()
        assert not (ENGINE / f"adapters/{agent}/plugin.py").exists()
    assert not (ENGINE / "adapters/base/plugin.py").exists()


def test_opencode_store_is_separate_from_session_codec():
    opencode = ENGINE / "adapters/opencode"
    assert (opencode / "store.py").is_file()
    session = (opencode / "session.py").read_text()
    assert "sqlite3.connect" not in session
    assert "subprocess.run" not in session
    assert "def load_native_payload" not in session
    assert "def import_payload" not in session


def test_business_capabilities_live_in_top_level_packages():
    organization = ENGINE / "organization"
    assert {
        path.name for path in organization.glob("*.py")
    } == {"__init__.py", "proposals.py", "summaries.py"}
    operations = ENGINE / "operations"
    assert {
        path.name for path in operations.glob("*.py")
    } == {
        "__init__.py",
        "delete.py",
        "edit.py",
        "history.py",
        "metadata.py",
        "migrate.py",
        "service.py",
        "types.py",
        "verification.py",
    }
    directories = {
        path.name for path in ENGINE.iterdir() if path.is_dir()
    }
    assert not {
        "application",
        "domain",
        "infrastructure",
        "interfaces",
    } & directories
