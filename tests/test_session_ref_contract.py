import json
from pathlib import Path

from engine.contracts.session_ref import is_opaque_session_ref


ROOT = Path(__file__).resolve().parents[1]


def test_session_ref_contract_is_generated_for_every_runtime():
    contract = json.loads((ROOT / "contracts/session-ref.json").read_text())
    assert contract == {
        "opaque_prefix": "fsr_",
        "minimum_length": 8,
        "maximum_length": 128,
        "allowed_suffix": "ascii-alphanumeric-underscore-hyphen",
    }
    for path in (
        "app/src/api/contract/generated/session-ref.ts",
        "app/src-tauri/src/contracts/session_ref.rs",
        "engine/contracts/session_ref.py",
        "ferry-runtime/src/server/generated/session-ref.ts",
    ):
        assert (ROOT / path).is_file()


def test_opaque_session_ref_uses_one_strict_shape():
    assert is_opaque_session_ref("fsr_valid")
    assert is_opaque_session_ref("fsr_a-b_C9")
    assert not is_opaque_session_ref("native-session-id")
    assert not is_opaque_session_ref("fsr_bad/path")
    assert not is_opaque_session_ref("fsr_\nsecret")
    assert not is_opaque_session_ref("fsr_" + "a" * 125)
