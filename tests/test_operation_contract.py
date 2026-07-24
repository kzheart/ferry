import json
from pathlib import Path

from engine.contracts.operations import (
    EDIT_OPERATION_KINDS,
    OPERATION_KINDS,
    OPERATION_PLAN_ID_PREFIX,
    OPERATION_STATUSES,
    OPERATION_SUCCESS_STATUS,
    OPERATION_TERMINAL_STATUSES,
)


ROOT = Path(__file__).resolve().parents[1]


def test_operation_contract_is_generated_for_every_runtime():
    source = json.loads((ROOT / "contracts/operations.json").read_text())
    assert set(source["kinds"]) == OPERATION_KINDS
    assert set(source["edit_operations"]) == EDIT_OPERATION_KINDS
    assert set(source["statuses"]) == OPERATION_STATUSES
    assert set(source["terminal_statuses"]) == OPERATION_TERMINAL_STATUSES
    assert source["plan_id_prefix"] == OPERATION_PLAN_ID_PREFIX == "op_"
    assert source["success_status"] == OPERATION_SUCCESS_STATUS == "applied"
    assert OPERATION_SUCCESS_STATUS in OPERATION_TERMINAL_STATUSES

    for path in (
        "app/src/api/contract/generated/operations.js",
        "app/src-tauri/src/contracts/operations.rs",
        "engine/contracts/operations.py",
        "ferry-runtime/src/server/generated/operations.ts",
    ):
        assert (ROOT / path).is_file()


def test_operation_consumers_use_generated_status_and_identity_contract():
    frontend = (
        ROOT / "app/src/features/operations/operationController.js"
    ).read_text()
    rust_request = (
        ROOT / "app/src-tauri/src/operations/request.rs"
    ).read_text()
    python_store = (ROOT / "engine/operations/plan_store.py").read_text()

    assert "OPERATION_TERMINAL_STATUSES" in frontend
    assert "OPERATION_SUCCESS_STATUS" in frontend
    assert "OPERATION_PLAN_ID_PREFIX" in rust_request
    assert "OPERATION_PLAN_ID_PREFIX" in python_store
