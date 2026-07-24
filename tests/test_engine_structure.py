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
        "planner.py",
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
    assert "class OperationPlanner" in (operations / "planner.py").read_text()
    assert "def _plan_edit" not in operation_service
    assert "def _plan_migration" not in operation_service
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
    safety = (sessions / "safety.py").read_text()
    assert "def redact(" in safety
    assert "def validate_json_shape(" in safety
    assert "def _redact(" not in catalog
    assert "def _validate_json_shape(" not in catalog


def test_adapter_shared_code_is_not_a_base_layer():
    adapters = ENGINE / "adapters"
    assert (adapters / "shared").is_dir()
    assert not (adapters / "base").exists()
    assert (adapters / "shared/codec.py").is_file()
    assert (adapters / "shared/editing.py").is_file()
    assert (adapters / "shared/migration.py").is_file()


def test_runtime_session_storage_is_its_own_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    runtime_sessions = (ENGINE / "storage/runtime_sessions.py").read_text()
    assert "class RuntimeSessionStore" in runtime_sessions
    assert "def load_runtime_sessions(" not in database
    assert "def commit_runtime_session(" not in database
    assert "def delete_runtime_session(" not in database


def test_metadata_and_history_are_separate_sqlite_capabilities():
    database = (ENGINE / "storage/database.py").read_text()
    assert (ENGINE / "storage/session_metadata.py").is_file()
    assert (ENGINE / "storage/migration_history.py").is_file()
    assert "def list_session_metadata(" not in database
    assert "def append_migration_history(" not in database
    assert (ENGINE / "storage/session_summaries.py").is_file()
    assert "def get_session_summary(" not in database


def test_operation_state_is_a_separate_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    operation_store = (ENGINE / "storage/operation_store.py").read_text()
    assert "class OperationStore" in operation_store
    assert "self.operations = OperationStore" in database
    assert "def store_plan(" not in database
    assert "def store_recovery(" not in database
    assert "def audit(" not in database


def test_organization_transaction_is_a_separate_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    organization_store = (ENGINE / "storage/organization_store.py").read_text()
    assert "class OrganizationStore" in organization_store
    assert "self.organization = OrganizationStore" in database
    assert "def create_or_get(" not in database
    assert "def decide(" not in database
    assert "def invalidate(" not in database
