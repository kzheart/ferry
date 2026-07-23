from engine.contracts.engine_methods import ENGINE_METHOD_POLICIES
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


def test_commit_and_agent_lookup_policies_are_explicit():
    assert ENGINE_METHOD_POLICIES["operation.apply"]["timeout"] == "commit"
    for method in ("agent_search_sessions", "agent_session_read", "agent_get_usage"):
        assert ENGINE_METHOD_POLICIES[method]["timeout"] == "lookup"
        assert ENGINE_METHOD_POLICIES[method]["retry"] == "never"
