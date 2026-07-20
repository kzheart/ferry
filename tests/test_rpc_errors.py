"""结构化 RPC envelope 与错误码测试。"""
import json

from engine.interfaces.rpc import PROTOCOL, rpc


def test_success_envelope_carries_protocol_and_request_id():
    response = rpc(json.dumps({"method": "health", "request_id": "req-1"}))
    assert response["protocol"] == PROTOCOL == 2
    assert response["ok"] is True
    assert response["request_id"] == "req-1"
    assert response["result"]["status"] == "ok"
    assert response["result"]["protocol"] == 2


def test_invalid_json_is_structured():
    response = rpc("{not json")
    assert response["ok"] is False
    assert response["protocol"] == 2
    assert response["error"]["code"] == "rpc.invalid_json"
    assert response["error"]["category"] == "validation"


def test_unknown_method_is_structured():
    response = rpc(json.dumps({"method": "nope"}))
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.unknown_method"
    assert response["error"]["params"] == {"method": "nope"}


def test_missing_param_is_structured():
    response = rpc(json.dumps({"method": "models", "params": {}}))
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.missing_param"
    assert response["error"]["params"] == {"param": "tool"}


def test_unknown_tool_is_structured():
    response = rpc(json.dumps({"method": "models", "params": {"tool": "nope"}}))
    assert response["ok"] is False
    assert response["error"]["code"] == "tool.unknown"
    assert response["error"]["params"] == {"tool": "nope"}
    assert response["error"]["category"] == "not-found"


def test_error_params_are_semantic_fields_not_sentences():
    response = rpc(json.dumps({
        "method": "authoring_preview",
        "params": {"tool": "claude", "ref": "no-such-session", "turn": 1,
                   "reply": {"items": [{"kind": "text", "text": "x"}]}}}))
    assert response["ok"] is False
    assert response["error"]["code"] == "session.not_found"
    assert response["error"]["params"] == {"tool": "claude",
                                           "ref": "no-such-session"}


def test_invalid_reply_maps_to_authoring_code():
    response = rpc(json.dumps({
        "method": "authoring_preview",
        "params": {"tool": "claude", "ref": "x", "turn": 1,
                   "reply": {"items": []}}}))
    assert response["ok"] is False
    assert response["error"]["code"] == "authoring.invalid_reply"


def test_tools_rpc_returns_manifests():
    response = rpc(json.dumps({"method": "tools"}))
    assert response["ok"] is True
    manifests = response["result"]
    ids = [m["id"] for m in manifests]
    assert ids == ["claude", "codex", "opencode"]
    for manifest in manifests:
        assert manifest["display_name"]
        assert manifest["icon"]
        assert manifest["executables"]
        assert "browse" in manifest["capabilities"]
