import json
from pathlib import Path

from engine.contracts.engine_methods import (
    ENGINE_METHOD_POLICIES,
    PARALLEL_READ_METHOD_NAMES,
)
from engine.server.rpc import RPC_METHODS


ROOT = Path(__file__).resolve().parents[1]


def test_engine_rpc_methods_exactly_match_generated_policy_contract():
    assert set(RPC_METHODS) == set(ENGINE_METHOD_POLICIES)


def test_operations_stay_outside_generic_public_rpc():
    for method in (
        "operation.plan",
        "operation.apply",
        "operation.status",
        "operation.cancel",
    ):
        assert ENGINE_METHOD_POLICIES[method]["exposure"] == "internal"


def test_organization_ui_and_runtime_methods_use_distinct_exposures():
    for method in (
        "organization_proposals_list",
        "organization_proposal_modify",
        "organization_proposal_decide",
    ):
        assert ENGINE_METHOD_POLICIES[method]["exposure"] == "trusted-ui"
    for method in (
        "session_backbone",
        "session_summaries_set",
        "organization_digest_context",
        "organization_propose",
    ):
        assert ENGINE_METHOD_POLICIES[method]["exposure"] == "internal"


def test_operation_enqueue_and_agent_lookup_policies_are_explicit():
    assert ENGINE_METHOD_POLICIES["operation.apply"] == {
        "kind": "mutation",
        "exposure": "internal",
        "timeout": "normal",
        "retry": "never",
        "dispatch": "serial",
    }
    for method in ("agent_search_sessions", "agent_session_read", "agent_get_usage"):
        assert ENGINE_METHOD_POLICIES[method]["exposure"] == "internal"
        assert ENGINE_METHOD_POLICIES[method]["timeout"] == "lookup"
        assert ENGINE_METHOD_POLICIES[method]["retry"] == "never"


def test_only_declared_pure_reads_can_use_parallel_dispatch():
    assert PARALLEL_READ_METHOD_NAMES == {
        "health",
        "version",
        "env",
        "models",
        "history",
        "session_meta_list",
    }
    assert all(
        ENGINE_METHOD_POLICIES[method]["kind"] == "read"
        for method in PARALLEL_READ_METHOD_NAMES
    )


def test_frontend_client_only_accepts_generated_engine_method_unions():
    contract = json.loads(
        (ROOT / "contracts/engine-methods.json").read_text()
    )["methods"]
    generated = (
        ROOT / "app/src/shared/contracts/generated/engine-methods.ts"
    ).read_text()
    client = (ROOT / "app/src/platform/desktop/client.ts").read_text()

    for method in contract:
        if method["exposure"] in {"public", "trusted-ui"}:
            assert json.dumps(method["name"]) in generated
        else:
            assert json.dumps(method["name"]) not in generated
    assert "method: PublicEngineMethod" in client
    assert "method: TrustedUiEngineMethod" in client
