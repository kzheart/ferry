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


def test_metadata_write_is_not_a_generic_rpc_method():
    response = rpc(json.dumps({"method": "session_meta_set", "params": {
        "id": "session", "patch": {"name": "direct write"},
    }}))
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.unknown_method"


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


def test_invalid_reply_maps_to_edit_code():
    response = rpc(json.dumps({
        "method": "operation.plan",
        "params": {"input": {
            "kind": "edit", "tool": "claude", "ref": "fsr_missing",
            "ops": [{
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": []},
            }],
        }}}))
    assert response["ok"] is False
    assert response["error"]["code"] == "edit.invalid_reply"
