import json
import re
from pathlib import Path

from engine.contracts.errors import FERRY_ERROR_POLICIES
from engine.errors import DomainError


ROOT = Path(__file__).resolve().parents[1]


def all_subclasses(cls):
    for child in cls.__subclasses__():
        yield child
        yield from all_subclasses(child)


def test_error_contract_is_generated_for_every_runtime():
    source = json.loads((ROOT / "contracts/errors.json").read_text())
    codes = [error["code"] for error in source["errors"]]
    assert codes == sorted(codes)
    assert set(codes) == set(FERRY_ERROR_POLICIES)

    for path in (
        "app/src/shared/contracts/generated/errors.ts",
        "app/src-tauri/src/contracts/errors.rs",
        "engine/contracts/errors.py",
        "ferry-runtime/src/server/generated/errors.ts",
    ):
        assert (ROOT / path).is_file()


def test_engine_error_metadata_comes_from_generated_policy():
    engine_codes = {
        code
        for code, policy in FERRY_ERROR_POLICIES.items()
        if "engine" in policy["sources"]
    }
    declared = {DomainError.code} | {
        error_type.code for error_type in all_subclasses(DomainError)
    }
    assert declared == engine_codes

    for error_type in (DomainError, *all_subclasses(DomainError)):
        error = error_type.__new__(error_type)
        DomainError.__init__(error)
        policy = FERRY_ERROR_POLICIES[error.code]
        assert error.category == policy["category"]
        assert error.retryable == policy["retryable"]


def test_runtime_protocol_errors_are_registered():
    runtime_codes = {
        code
        for code, policy in FERRY_ERROR_POLICIES.items()
        if "runtime" in policy["sources"]
    }
    source = "\n".join(
        path.read_text()
        for path in (ROOT / "ferry-runtime/src").rglob("*.ts")
        if "generated" not in path.parts
    )
    declared = set(re.findall(
        r'new ProtocolError\(\s*"([^"]+)"', source, re.DOTALL,
    ))
    declared.update(re.findall(
        r'failure\(\s*"([^"]+)"', source, re.DOTALL,
    ))
    assert declared == runtime_codes


def test_host_error_reclassification_uses_generated_policy():
    host_codes = {
        code
        for code, policy in FERRY_ERROR_POLICIES.items()
        if "host" in policy["sources"]
    }
    rust = "\n".join(
        path.read_text()
        for path in (ROOT / "app/src-tauri/src/runtime").glob("*.rs")
    )
    for code in host_codes:
        assert f'"{code}"' in rust
    assert "error_policy(code)" in rust
