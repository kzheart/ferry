"""静态内置 Adapter 与 JSONL scanner 测试。"""
from engine.adapters.registry import create_registry
from engine.adapters.shared.scanner import scan_jsonl


class _Cache:
    def __init__(self):
        self.values = {}

    def get(self, path, _stat):
        return self.values.get(path)

    def put(self, path, _stat, value):
        self.values[path] = value


def test_scan_jsonl_reuses_cached_adapter_metadata(tmp_path):
    source = tmp_path / "session.jsonl"
    source.write_text('{"message": "one"}\n')
    cache = _Cache()
    calls = []

    def parse(path, _stat):
        calls.append(path)
        return {"id": path.stem, "title": "one", "count": 1}

    first = scan_jsonl(str(tmp_path / "*.jsonl"), cache, parse)
    second = scan_jsonl(str(tmp_path / "*.jsonl"), cache, parse)
    assert first == second
    assert first[0]["id"] == "session"
    assert first[0]["children"] == []
    assert calls == [source]


def test_bundled_adapters_wire_components_declared_by_capabilities():
    registry = create_registry()

    for tool in registry.ids():
        adapter = registry.get(tool)
        assert adapter.supports("browse")
        assert adapter.supports("resume")
        assert adapter.require("migration-source", "migration_source") is not None
        assert adapter.require("migration-target", "migration_target") is not None
        assert adapter.require("edit", "editor") is not None
        assert adapter.require("probe", "verifier") is not None
        assert adapter.require("models", "models") is not None
        assert adapter.require("resume", "lifecycle") is not None
        assert adapter.require("delete", "lifecycle") is not None
        assert adapter.lifecycle.executable == adapter.manifest.executables[0]
