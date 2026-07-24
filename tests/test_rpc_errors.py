"""结构化 RPC envelope 与错误码测试。"""
import json

from engine.server.rpc import PROTOCOL, RpcDispatcher, rpc


def request(method: str, params: dict | None = None, request_id: str = "req-1"):
    return rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": request_id,
        "method": method,
        "params": params or {},
    }))


def test_success_envelope_carries_protocol_and_id():
    response = request("health")
    assert response["protocol"] == PROTOCOL == "ferry-ipc/1"
    assert response["ok"] is True
    assert response["id"] == "req-1"
    assert response["result"]["status"] == "ready"
    assert response["result"]["service"] == "engine"
    assert response["result"]["contract_hash"].startswith("sha256:")


def test_invalid_json_is_structured():
    response = rpc("{not json")
    assert response["ok"] is False
    assert response["protocol"] == PROTOCOL
    assert response["id"] == "unknown"
    assert response["error"]["code"] == "rpc.invalid_json"
    assert response["error"]["category"] == "validation"


def test_old_or_extended_envelopes_are_rejected():
    old = rpc(json.dumps({
        "protocol": 2,
        "id": "old",
        "method": "health",
        "params": {},
    }))
    assert old["id"] == "old"
    assert old["error"]["code"] == "rpc.unsupported_protocol"

    extended = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "extended",
        "method": "health",
        "params": {},
        "request_id": "legacy",
    }))
    assert extended["id"] == "extended"
    assert extended["error"]["code"] == "rpc.invalid_request"


def test_unknown_method_is_structured():
    response = request("nope")
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.unknown_method"
    assert response["error"]["params"] == {"method": "nope"}


def test_metadata_write_is_not_a_generic_rpc_method():
    response = request("session_meta_set", {
        "id": "session", "patch": {"name": "direct write"},
    })
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.unknown_method"


def test_missing_param_is_structured():
    response = request("models")
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.missing_param"
    assert response["error"]["params"] == {"param": "tool"}


def test_unknown_tool_is_structured():
    response = request("models", {"tool": "nope"})
    assert response["ok"] is False
    assert response["error"]["code"] == "tool.unknown"
    assert response["error"]["params"] == {"tool": "nope"}
    assert response["error"]["category"] == "not-found"


def test_invalid_reply_maps_to_edit_code():
    response = request("operation.plan", {"input": {
            "kind": "edit", "tool": "claude", "ref": "fsr_missing",
            "ops": [{
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": []},
            }],
        }})
    assert response["ok"] is False
    assert response["error"]["code"] == "edit.invalid_reply"


def test_environment_and_pricing_use_the_engine_capability_facade():
    class Application:
        def environment(self):
            return {"environment": "current"}

        def pricing(self, force=False):
            return {"forced": force}

    dispatcher = RpcDispatcher(Application())
    environment = dispatcher.handle(json.dumps({
        "protocol": PROTOCOL,
        "id": "env",
        "method": "env",
        "params": {},
    }))
    pricing = dispatcher.handle(json.dumps({
        "protocol": PROTOCOL,
        "id": "pricing",
        "method": "pricing",
        "params": {"force": True},
    }))

    assert environment["result"] == {"environment": "current"}
    assert pricing["result"] == {"forced": True}
