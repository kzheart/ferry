import importlib.util
import json
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "generate_contracts", ROOT / "scripts/generate-contracts.py"
)
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


def test_dto_revision_changes_the_handshake_without_renaming_the_method():
    methods = GENERATOR.load_engine_methods()
    old_methods = [dict(method) for method in methods]
    for method in old_methods:
        method.pop("wire_revision", None)

    def digest(items):
        return GENERATOR.contract_hash([], items, [], {}, {}, {}, [], [])

    assert digest(methods) != digest(old_methods)


@pytest.mark.parametrize("revision", [0, -1, True, "2", None])
def test_invalid_wire_revisions_are_rejected(tmp_path, monkeypatch, revision):
    methods = GENERATOR.load_engine_methods()
    methods[0]["wire_revision"] = revision
    path = tmp_path / "engine-methods.json"
    path.write_text(json.dumps({"methods": methods}))
    monkeypatch.setattr(GENERATOR, "ENGINE_METHODS_SOURCE", path)
    with pytest.raises(ValueError, match="wire_revision"):
        GENERATOR.load_engine_methods()
