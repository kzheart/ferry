import json
from pathlib import Path

from engine.contracts.ipc import FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL


ROOT = Path(__file__).resolve().parent.parent


def test_all_runtimes_use_the_generated_ferry_ipc_protocol():
    source = json.loads((ROOT / "contracts/ipc.json").read_text())
    assert source["protocol"] == FERRY_IPC_PROTOCOL == "ferry-ipc/1"

    generated = (
        ROOT / "app/src/api/contract/generated/ipc.js",
        ROOT / "app/src-tauri/src/contracts/ipc.rs",
        ROOT / "ferry-runtime/src/server/generated/ipc.ts",
    )
    for path in generated:
        text = path.read_text()
        assert FERRY_IPC_PROTOCOL in text
        assert FERRY_CONTRACT_HASH in text
        assert "ferry-agent/v1" not in text
        assert "ferry-runtime/v1" not in text
    assert FERRY_CONTRACT_HASH.startswith("sha256:")
    assert len(FERRY_CONTRACT_HASH) == len("sha256:") + 64


def test_ipc_envelope_fields_are_exact():
    source = json.loads((ROOT / "contracts/ipc.json").read_text())
    assert source["request"]["required"] == [
        "protocol", "id", "method", "params",
    ]
    assert source["response"]["success_required"] == [
        "protocol", "id", "ok", "result",
    ]
    assert source["response"]["failure_required"] == [
        "protocol", "id", "ok", "error",
    ]
    assert source["error"]["required"] == [
        "code", "category", "retryable", "params",
    ]
    assert source["event"] == {
        "required": ["protocol", "type", "payload"],
        "optional": ["correlation_id", "context"],
        "additional_properties": False,
    }
    assert all(
        value["additional_properties"] is False
        for key, value in source.items()
        if key != "protocol"
    )
