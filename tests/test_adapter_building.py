"""Reusable adapter assembly and JSONL scanner tests."""
from engine.adapters.base.builder import BrowserAdapter, ModelCatalogAdapter, build_plugin
from engine.adapters.base.plugin import ToolManifest
from engine.adapters.base.scanner import scan_jsonl


class _Cache:
    def __init__(self):
        self.values = {}

    def get(self, path, _stat):
        return self.values.get(path)

    def put(self, path, _stat, value):
        self.values[path] = value


class _Lifecycle:
    executable = ""


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


def test_build_plugin_wires_shared_browser_models_and_lifecycle():
    manifest = ToolManifest(
        id="test-agent", display_name="Test Agent", icon="test",
        source_path="~/.test-agent", reference_kind="id", executables=("test-agent",))
    browser = BrowserAdapter(lambda _cache: [], lambda ref: {"ref": ref}, lambda ref: ref)
    lifecycle = _Lifecycle()

    plugin = build_plugin(
        manifest, browser, lifecycle=lifecycle,
        models=ModelCatalogAdapter(lambda: [{"id": "model"}], lambda: []))

    assert plugin.browser.read("session") == {"ref": "session"}
    assert plugin.models.discover() == [{"id": "model"}]
    assert lifecycle.executable == "test-agent"
    assert plugin.capabilities() == ["browse", "migrate-source"]
