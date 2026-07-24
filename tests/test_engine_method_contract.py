from engine.contracts.engine_methods import (
    ENGINE_METHOD_POLICIES,
    PARALLEL_READ_METHOD_NAMES,
)
from engine.interfaces.rpc import RPC_METHODS


def test_engine_rpc_methods_exactly_match_generated_policy_contract():
    assert set(RPC_METHODS) == set(ENGINE_METHOD_POLICIES)


def test_operations_stay_outside_generic_public_rpc():
    for method in (
        "operation.plan",
        "operation.apply",
        "operation.status",
        "operation.cancel",
    ):
        assert ENGINE_METHOD_POLICIES[method]["public"] is False


def test_operation_enqueue_and_agent_lookup_policies_are_explicit():
    assert ENGINE_METHOD_POLICIES["operation.apply"] == {
        "kind": "mutation",
        "public": False,
        "timeout": "normal",
        "retry": "never",
        "dispatch": "serial",
    }
    for method in ("agent_search_sessions", "agent_session_read", "agent_get_usage"):
        assert ENGINE_METHOD_POLICIES[method]["timeout"] == "lookup"
        assert ENGINE_METHOD_POLICIES[method]["retry"] == "never"


def test_only_declared_pure_reads_can_use_parallel_dispatch():
    assert PARALLEL_READ_METHOD_NAMES == {
        "health",
        "version",
        "env",
        "models",
        "history",
        "edit_capabilities",
        "session_meta_list",
    }
    assert all(
        ENGINE_METHOD_POLICIES[method]["kind"] == "read"
        for method in PARALLEL_READ_METHOD_NAMES
    )
