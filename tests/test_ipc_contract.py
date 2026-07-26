import json
from pathlib import Path

from engine.contracts.ipc import FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL


ROOT = Path(__file__).resolve().parent.parent


def test_all_runtimes_use_the_generated_ferry_ipc_protocol():
    source = json.loads((ROOT / "contracts/ipc.json").read_text())
    assert source["protocol"] == FERRY_IPC_PROTOCOL == "ferry-ipc/1"

    generated = (
        ROOT / "app/src/shared/contracts/generated/ipc.ts",
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
