"""插件契约测试：可选能力、只读 fake 插件与 turn locator 一致性。"""
import copy

import pytest

from engine.adapters.base.codec import select_span
from engine.adapters.base.plugin import ToolManifest, ToolPlugin
from engine.adapters.registry import AdapterRegistry, create_registry
from engine.adapters.claude.codec import TURN_INDEX as CLAUDE_INDEX
from engine.adapters.codex.codec import TURN_INDEX as CODEX_INDEX
from engine.adapters.opencode.codec import TURN_INDEX as OPENCODE_INDEX
from engine.application.sessions import session_json
from engine.domain.edit import AssistantReply
from engine.domain.errors import CapabilityUnsupportedError, ToolUnknownError

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

    def resolve_ref(self, ref):
        return ref


def _fake_plugin() -> ToolPlugin:
    return ToolPlugin(
        manifest=ToolManifest(id="fake", display_name="Fake Agent", icon="fake",
                              source_path="~/.fake/sessions", reference_kind="id",
                              executables=("fake",)),
        browser=_FakeBrowser(),
    )


def test_readonly_fake_plugin_satisfies_contract():
    plugin = _fake_plugin()
    assert plugin.capabilities() == ["browse"]
    described = plugin.describe()
    assert described["id"] == "fake"
    assert described["capabilities"] == ["browse"]
    assert described["executables"] == ["fake"]
    assert plugin.browser.scan(cache=None) == []


@pytest.mark.parametrize("capability", [
    "migration_source", "migration_target", "editor",
    "verifier", "lifecycle", "models",
])
def test_missing_capability_reports_unsupported(capability):
    plugin = _fake_plugin()
    with pytest.raises(CapabilityUnsupportedError) as excinfo:
        plugin.require(capability)
    assert excinfo.value.code == "edit.operation_unsupported"
    assert excinfo.value.params == {"tool": "fake", "capability": capability}


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
    assert create_registry().ids() == ("claude", "codex", "opencode")


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
    from engine.application import services
    for manifest in services.tool_manifests():
        descriptor = services.resume_command(manifest["id"], "sid-1", "/work")
        assert descriptor["executable"] in manifest["executables"]
        assert descriptor["args"]
        assert descriptor["executable"] in descriptor["display_command"]
        assert "sid-1" in descriptor["display_command"]
