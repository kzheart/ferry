"""内置 Adapter 静态契约与 turn locator 一致性测试。"""
import copy

import pytest

from engine.adapters.base.codec import select_span
from engine.adapters.base.plugin import (
    MigrationSource, MigrationTarget, ModelCatalog, SessionBrowser,
    SessionEditor, SessionLifecycle, SessionVerifier, ToolManifest, ToolPlugin,
    id_reference,
)
from engine.adapters.registry import AdapterRegistry, create_registry
from engine.adapters.claude.codec import TURN_INDEX as CLAUDE_INDEX
from engine.adapters.codex.codec import TURN_INDEX as CODEX_INDEX
from engine.adapters.opencode.codec import TURN_INDEX as OPENCODE_INDEX
from engine.application.sessions import session_json
from engine.domain.edit import AssistantReply
from engine.domain.errors import ToolUnknownError
from engine.contracts.agents import AGENT_IDS

from test_reply_editing import (
    _editor,
    _document,
    _items,
    _native,
    _roundtrip,
    _validate,
)

TURN_INDEXES = {"claude": CLAUDE_INDEX, "codex": CODEX_INDEX,
                "opencode": OPENCODE_INDEX}


class _FakeBrowser:
    def scan(self, cache):
        return []

    def read(self, ref):
        raise ValueError(f"找不到 fake 会话: {ref}")

    def read_agent(self, ref):
        return self.read(ref)

    def resolve_ref(self, ref):
        return ref

    def fingerprint(self, _ref):
        return "fake-revision"

    def agent_fingerprint(self, ref):
        return self.fingerprint(ref)

    def canonicalize(self, row):
        return id_reference(row)


class _FakeMigrationSource:
    def export_tree(self, ref):
        return _FakeBrowser().read(ref)


class _FakeMigrationTarget:
    def plan(self, _session):
        return {}

    def preview(self, _session, _cwd=None):
        return {}

    def write(self, _session, _cwd):
        raise AssertionError("fake target 不应写入")

    def classify_tool_call(self, _tool_call):
        return "native"


class _FakeEditor:
    name = "fake"

    def capabilities(self):
        return {"inplace": True, "operation_modes": {}}

    def load(self, _ref):
        return object()

    def apply_ops(self, _doc, _ops):
        return []

    def replace_reply(self, _doc, _turn, _reply):
        return []

    def validate(self, _doc):
        pass

    def stats(self, _doc):
        return {}

    def commit(self, _doc):
        return {}

    def snapshot(self, _doc, reason_code=None, extra=None):
        return object()

    def restore_snapshot(self, _snapshot, _doc):
        pass

    def saved_revision(self, _result, _doc):
        return "fake-revision"


class _FakeVerifier:
    def probe(self, _session_id, _cwd, _model=None):
        return {"status": "skipped"}

    def probe_edited(self, _editor, _doc, _result, _model=None):
        return {"status": "skipped"}


class _FakeLifecycle:
    executable = "fake"
    delete_undoable = False

    def resume_descriptor(self, session_id, cwd):
        return {"session_id": session_id, "cwd": cwd}

    def cleanup(self, _session_id, _dest):
        pass

    def validation_ref(self, session_id, _dest):
        return session_id

    def probe_cwd(self, cwd):
        return cwd

    def delete(self, _plugin, _ref):
        raise AssertionError("fake lifecycle 不应删除")

    def restore_delete(self, _snapshot, _meta):
        raise AssertionError("fake lifecycle 不应恢复")


class _FakeModels:
    def discover(self):
        return [], "fake", None

    def fallback(self):
        return []


def _fake_plugin() -> ToolPlugin:
    return ToolPlugin(
        manifest=ToolManifest(id="fake", display_name="Fake Agent", icon="fake",
                              source_path="~/.fake/sessions",
                              executables=("fake",)),
        browser=_FakeBrowser(),
        migration_source=_FakeMigrationSource(),
        migration_target=_FakeMigrationTarget(),
        editor=_FakeEditor(),
        verifier=_FakeVerifier(),
        lifecycle=_FakeLifecycle(),
        models=_FakeModels(),
    )


def test_fake_plugin_satisfies_complete_static_contract():
    plugin = _fake_plugin()
    assert isinstance(plugin.browser, SessionBrowser)
    assert isinstance(plugin.migration_source, MigrationSource)
    assert isinstance(plugin.migration_target, MigrationTarget)
    assert isinstance(plugin.editor, SessionEditor)
    assert isinstance(plugin.verifier, SessionVerifier)
    assert isinstance(plugin.lifecycle, SessionLifecycle)
    assert isinstance(plugin.models, ModelCatalog)
    assert plugin.capabilities() == [
        "browse", "migrate-source", "migrate-target", "edit", "inplace", "verified",
    ]
    described = plugin.describe()
    assert described["id"] == "fake"
    assert described["capabilities"] == plugin.capabilities()
    assert described["executables"] == ["fake"]
    assert plugin.browser.scan(cache=None) == []


def test_registry_accepts_injected_fake_plugin():
    registry = AdapterRegistry((_fake_plugin(),))
    assert registry.ids() == ("fake",)
    assert registry.get("fake").id == "fake"


def test_registry_rejects_duplicate_adapter_ids():
    with pytest.raises(ValueError, match="重复的 adapter id"):
        AdapterRegistry((_fake_plugin(), _fake_plugin()))


def test_registry_reports_unknown_adapter():
    registry = AdapterRegistry((_fake_plugin(),))
    with pytest.raises(ToolUnknownError) as excinfo:
        registry.get("missing")
    assert excinfo.value.code == "tool.unknown"


def test_registry_explicitly_composes_all_bundled_adapters():
    assert create_registry().ids() == AGENT_IDS


@pytest.mark.parametrize("tool", ["claude", "codex"])
def test_turn_locator_consistent_across_read_replace_delete(tool, tmp_path):
    """reader 展示的 turn locator == 编辑器替换的原生 turn
    == delete-turn 删除的原生 turn。"""
    data = _native(tool, "case-02-tools")
    dto = session_json(_roundtrip(tool, data, tmp_path))
    doc = _document(tool, data)
    index = TURN_INDEXES[tool]
    spans = index.turns(doc.data)

    # reader DTO 的每一轮 locator 与 TurnIndex 一致
    assert [span.locator for span in spans] == \
        [turn["turn_locator"] for turn in dto["turns"]]

    # 编辑器按 DTO locator 替换的轮次 == TurnIndex 按 ordinal 选中的轮次
    locator = dto["turns"][0]["turn_locator"]
    assert select_span(spans, locator) == select_span(spans, 1)

    # 用 locator 与用 ordinal 做回复替换，产物一致
    reply = AssistantReply.from_dict(
        {"items": [{"kind": "text", "text": "replaced"}]})
    by_locator = _document(tool, copy.deepcopy(data))
    by_ordinal = _document(tool, copy.deepcopy(data))
    _editor(tool).replace_reply(by_locator, locator, reply)
    _editor(tool).replace_reply(by_ordinal, 1, reply)
    dir_a, dir_b = tmp_path / "a", tmp_path / "b"
    dir_a.mkdir()
    dir_b.mkdir()
    assert _items(_roundtrip(tool, by_locator.data, dir_a)) == \
        _items(_roundtrip(tool, by_ordinal.data, dir_b))

    # delete-turn 删除的正是同一原生区间
    codec = __import__(
        f"engine.adapters.{tool}.codec", fromlist=["CODEC"]
    ).CODEC
    span = select_span(spans, locator)
    before = list(doc.data)
    codec.delete_turn(doc, span)
    assert doc.data == before[:span.start] + before[span.end:] or \
        len(doc.data) == len(before) - (span.end - span.start)


@pytest.mark.parametrize("tool", ["claude", "codex", "opencode"])
@pytest.mark.parametrize("role", ["user", "assistant"])
def test_rewrite_supports_visible_user_and_assistant_text(tool, role, tmp_path):
    data = _native(tool, "case-02-tools")
    original = session_json(_roundtrip(tool, data, tmp_path))
    target = next(message for message in original["messages"]
                  if message["role"] == role
                  and any(block["kind"] == "text" for block in message["blocks"]))
    doc = _document(tool, data)
    codec = __import__(f"engine.adapters.{tool}.codec", fromlist=["CODEC"]).CODEC

    codec.rewrite_message(doc, target["locator"], "更委婉的表述")
    _validate(tool, doc.data)
    rewritten = session_json(_roundtrip(tool, doc.data, tmp_path))
    message = next(item for item in rewritten["messages"]
                   if item["locator"] == target["locator"])
    assert [block["text"] for block in message["blocks"]
            if block["kind"] == "text"] == ["更委婉的表述"]


def test_opencode_validation_accepts_terminal_assistant_error():
    data = _native("opencode", "case-02-tools")
    assistant = next(message for message in data["messages"]
                     if message["info"].get("role") == "assistant")
    assistant["info"].pop("finish", None)
    assistant["info"]["error"] = {"name": "ProviderError", "message": "failed"}

    _validate("opencode", data)


def test_codex_rewrite_preserves_user_image_content():
    data = _native("codex", "case-01-plain")
    ordinal, record = next((index, item) for index, item in enumerate(data)
                           if (item.get("payload") or {}).get("role") == "user")
    image = {"type": "input_image", "image_url": "data:image/png;base64,AA=="}
    record["payload"]["content"].append(image)
    from engine.adapters.codex.codec import CODEC

    doc = _document("codex", data)
    CODEC.rewrite_message(doc, f"record:{ordinal}", "更委婉的提问")
    content = doc.data[ordinal]["payload"]["content"]
    assert image in content
    assert {item.get("text") for item in content if item["type"] == "input_text"} \
        == {"更委婉的提问"}
    _validate("codex", doc.data)


def test_resume_descriptor_executable_matches_manifest_whitelist():
    from engine.application import migration, services
    from engine.application.ports import current
    for agent_id in services.adapters():
        adapter = services.adapter(agent_id)
        descriptor = migration.MigrationService(current()).resume_command(
            agent_id, "sid-1", "/work")
        assert descriptor["executable"] in adapter.manifest.executables
        assert descriptor["args"]
        assert descriptor["executable"] in descriptor["display_command"]
        assert "sid-1" in descriptor["display_command"]
