from pathlib import Path

from engine.contracts.agents import AGENTS, AGENT_IDS


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "engine"


def test_adapter_contract_has_no_plugin_layer():
    assert (ENGINE / "adapters/contracts.py").is_file()
    for agent in AGENT_IDS:
        assert (ENGINE / f"adapters/{agent}/adapter.py").is_file()
        assert not (ENGINE / f"adapters/{agent}/plugin.py").exists()
    assert not (ENGINE / "adapters/base/plugin.py").exists()


def test_opencode_adapter_has_explicit_reader_writer_and_store():
    opencode = ENGINE / "adapters/opencode"
    assert (opencode / "store.py").is_file()
    assert (opencode / "reader.py").is_file()
    assert (opencode / "writer.py").is_file()
    assert (opencode / "payload.py").is_file()
    assert not (opencode / "session.py").exists()
    writer = (opencode / "writer.py").read_text()
    assert "sqlite3.connect" not in writer
    assert "subprocess.run" not in writer
    payload = (opencode / "payload.py").read_text()
    assert "def canonical_payload(" in payload
    assert "def remap_payload(" in payload
    assert "def ensure_task_links(" in payload
    tool_calls = (opencode / "tool_calls.py").read_text()
    assert "OP_WRITERS" in tool_calls


def test_codex_reader_keeps_rollout_topology_in_its_own_capability():
    codex = ENGINE / "adapters/codex"
    reader = (codex / "reader.py").read_text()
    topology = (codex / "topology.py").read_text()
    assert "def read_tree" in topology
    assert "def rollout_index" in topology
    assert "thread_spawn_edges" in topology
    assert "sqlite3.connect" not in reader
    assert (codex / "tool_results.py").is_file()
    assert "def parse_result" in (codex / "tool_results.py").read_text()
    tool_calls = (codex / "tool_calls.py").read_text()
    assert "def parse_custom_call" in tool_calls
    assert "def parse_function_call" in tool_calls


def test_business_capabilities_live_in_top_level_packages():
    organization = ENGINE / "organization"
    assert {
        path.name for path in organization.glob("*.py")
    } == {
        "__init__.py",
        "proposals.py",
        "store.py",
        "summaries.py",
        "summary_store.py",
    }
    operations = ENGINE / "operations"
    assert {
        path.name for path in operations.glob("*.py")
    } == {
        "__init__.py",
        "delete.py",
        "edit.py",
        "executor.py",
        "history.py",
        "history_store.py",
        "metadata.py",
        "metadata_store.py",
        "migrate.py",
        "planner.py",
        "plan_store.py",
        "service.py",
        "snapshots.py",
        "state_store.py",
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
    assert "def validate_edit_input" in (operations / "validation.py").read_text()
    assert "class OperationPlanner" in (operations / "planner.py").read_text()
    assert "class OperationExecutor" in (operations / "executor.py").read_text()
    assert "class EditOperationHandler" in (operations / "edit.py").read_text()


def test_session_reference_index_is_isolated_from_query_catalog():
    sessions = ENGINE / "sessions"
    index = (sessions / "index.py").read_text()
    assert "class AgentSessionIndex" in index
    assert "class IndexedSession" in index
    assert not (sessions / "catalog.py").exists()
    assert (sessions / "search.py").is_file()
    assert "def search_sessions" in (sessions / "search.py").read_text()
    assert "def get_usage" in (sessions / "usage.py").read_text()
    assert (sessions / "agent_read.py").is_file()
    assert "def session_read" in (sessions / "agent_read.py").read_text()
    assert "def preview(self, record" in (
        ENGINE / "operations/edit.py"
    ).read_text()
    safety = (sessions / "safety.py").read_text()
    assert "def truncate_text(" in safety
    assert "def bounded_json(" in safety
    assert "def validate_json_shape(" in safety
    assert (sessions / "scan_cache.py").is_file()
    assert not (ENGINE / "storage/scan_cache.py").exists()


def test_storage_package_only_owns_database_composition():
    storage = ENGINE / "storage"
    assert {
        path.name for path in storage.glob("*.py")
    } == {"__init__.py", "database.py"}


def test_adapter_shared_code_is_not_a_base_layer():
    adapters = ENGINE / "adapters"
    assert (adapters / "shared").is_dir()
    assert not (adapters / "base").exists()
    assert (adapters / "shared/codec.py").is_file()
    assert (adapters / "shared/editing.py").is_file()
    assert (adapters / "shared/migration.py").is_file()


def test_runtime_session_storage_is_its_own_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    runtime_sessions = (ENGINE / "runtime/store.py").read_text()
    assert "class RuntimeSessionStore" in runtime_sessions
    assert not (ENGINE / "storage/runtime_sessions.py").exists()
    assert "def load_runtime_sessions(" not in database
    assert "def commit_runtime_session(" not in database
    assert "def delete_runtime_session(" not in database


def test_metadata_and_history_are_separate_sqlite_capabilities():
    database = (ENGINE / "storage/database.py").read_text()
    assert (ENGINE / "operations/metadata_store.py").is_file()
    assert (ENGINE / "operations/history_store.py").is_file()
    assert not (ENGINE / "storage/session_metadata.py").exists()
    assert not (ENGINE / "storage/migration_history.py").exists()
    assert not (ENGINE / "storage/snapshots.py").exists()
    assert "def list_session_metadata(" not in database
    assert "def append_migration_history(" not in database
    assert (ENGINE / "organization/summary_store.py").is_file()
    assert not (ENGINE / "storage/session_summaries.py").exists()
    assert "def get_session_summary(" not in database


def test_operation_state_is_a_separate_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    operation_store = (ENGINE / "operations/state_store.py").read_text()
    assert "class OperationStore" in operation_store
    assert "self.operations = OperationStore" in database
    assert not (ENGINE / "storage/operation_store.py").exists()
    assert "def store_plan(" not in database
    assert "def store_recovery(" not in database
    assert "def audit(" not in database


def test_organization_transaction_is_a_separate_sqlite_capability():
    database = (ENGINE / "storage/database.py").read_text()
    organization_store = (ENGINE / "organization/store.py").read_text()
    assert "class OrganizationStore" in organization_store
    assert "self.organization = OrganizationStore" in database
    assert not (ENGINE / "storage/organization_store.py").exists()
    assert "def create_or_get(" not in database
    assert "def decide(" not in database
    assert "def invalidate(" not in database


CAPABILITY_MODULES = {
    "browse": {"native_schema", "reader", "scanner"},
    "resume": {"lifecycle"},
    "migration-source": set(),
    "migration-target": {"migration", "writer"},
    "edit": {"codec", "editor"},
    "delete": {"lifecycle"},
    "probe": {"probe"},
    "models": {"models"},
}


def test_every_adapter_provides_modules_required_by_its_capabilities():
    for agent in AGENT_IDS:
        package = ENGINE / f"adapters/{agent}"
        modules = {path.stem for path in package.glob("*.py")} - {"__init__"}
        required = {"adapter"}
        for capability in AGENTS[agent]["capabilities"]:
            required.update(CAPABILITY_MODULES[capability])
        missing = required - modules
        assert not missing, f"{agent} Adapter 缺少模块: {sorted(missing)}"


def test_shared_adapter_layer_imports_no_concrete_agent():
    for path in (ENGINE / "adapters/shared").glob("*.py"):
        source = path.read_text(encoding="utf-8")
        for agent in AGENT_IDS:
            assert f"from ..{agent}" not in source, f"{path.name} 依赖了 {agent}"
            assert f"import {agent}" not in source, f"{path.name} 依赖了 {agent}"
