from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "engine"


def test_adapter_contract_has_no_plugin_layer():
    assert (ENGINE / "adapters/contracts.py").is_file()
    for agent in ("claude", "codex", "opencode"):
        assert (ENGINE / f"adapters/{agent}/adapter.py").is_file()
        assert not (ENGINE / f"adapters/{agent}/plugin.py").exists()
    assert not (ENGINE / "adapters/base/plugin.py").exists()


def test_opencode_adapter_has_explicit_reader_writer_and_store():
    opencode = ENGINE / "adapters/opencode"
    assert (opencode / "store.py").is_file()
    assert (opencode / "reader.py").is_file()
    assert (opencode / "writer.py").is_file()
    assert not (opencode / "session.py").exists()
    writer = (opencode / "writer.py").read_text()
    assert "sqlite3.connect" not in writer
    assert "subprocess.run" not in writer
    assert "def load_native_payload" not in writer
    assert "def import_payload" not in writer
    assert "def parse_session" not in writer
    assert "def read(" not in writer


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
        "plan_store.py",
        "service.py",
        "types.py",
        "validation.py",
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
    operation_service = (operations / "service.py").read_text()
    assert "class OperationPlan:" not in operation_service
    assert "class OperationState:" not in operation_service
    assert "hashlib.sha256" not in operation_service
    assert "def _validate_edit_input" not in operation_service
    assert "def _validate_migration_input" not in operation_service
    assert "def validate_edit_input" in (operations / "validation.py").read_text()
    assert "def _resolve_ops" not in operation_service
    assert "def _preview_edit" not in operation_service
    assert "class EditOperationHandler" in (operations / "edit.py").read_text()


def test_session_reference_index_is_isolated_from_query_catalog():
    sessions = ENGINE / "sessions"
    index = (sessions / "index.py").read_text()
    catalog = (sessions / "catalog.py").read_text()
    assert "class AgentSessionIndex" in index
    assert "class IndexedSession" in index
    assert "class AgentSessionIndex" not in catalog
    assert "class IndexedSession" not in catalog
