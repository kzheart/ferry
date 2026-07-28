"""内置 Adapter 静态契约与 turn locator 一致性测试。"""
import copy

import pytest

from engine.adapters.shared.codec import select_span
from engine.adapters import registry as adapter_registry
from engine.adapters.contracts import (
    MigrationSource, MigrationTarget, ModelCatalog, SessionBrowser,
    SessionEditor, SessionLifecycle, SessionVerifier, AgentManifest, AgentAdapter,
    NativeSessionReference, id_reference,
)
from engine.adapters.registry import AdapterRegistry, create_registry
from engine.adapters.claude.adapter import ClaudeBrowser
from engine.adapters.codex.adapter import CodexBrowser
from engine.adapters.opencode.adapter import OpenCodeBrowser
from engine.adapters.claude.codec import TURN_INDEX as CLAUDE_INDEX
from engine.adapters.codex.codec import TURN_INDEX as CODEX_INDEX
from engine.adapters.opencode.codec import TURN_INDEX as OPENCODE_INDEX
from engine.sessions.read import session_json
from engine.operations.types import AssistantReply
from engine.errors import AgentReferenceError, ToolUnknownError
from engine.contracts.agents import AGENTS, AGENT_CAPABILITIES, AGENT_IDS

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

    def validate_read_scope(self, _ref):
        pass


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
    operations = ("rewrite",)

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

    def prompt_session(
        self, _session_id, _cwd, _prompt, _model=None, _timeout=360,
    ):
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

    def delete(self, _adapter, _ref):
        raise AssertionError("fake lifecycle 不应删除")

    def restore_delete(self, _snapshot, _meta):
        raise AssertionError("fake lifecycle 不应恢复")


class _FakeModels:
    def discover(self):
        return [], "fake", None

    def fallback(self):
        return []


def _fake_adapter() -> AgentAdapter:
    return AgentAdapter(
        manifest=AgentManifest(id="fake", display_name="Fake Agent", icon="fake",
                              source_path="~/.fake/sessions",
                              capabilities=AGENT_CAPABILITIES,
                              edit_operations=("rewrite",),
                              executables=("fake",)),
        browser=_FakeBrowser(),
        migration_source=_FakeMigrationSource(),
        migration_target=_FakeMigrationTarget(),
        editor=_FakeEditor(),
        verifier=_FakeVerifier(),
        lifecycle=_FakeLifecycle(),
        models=_FakeModels(),
    )


def test_fake_adapter_satisfies_complete_static_contract():
    adapter = _fake_adapter()
    assert isinstance(adapter.browser, SessionBrowser)
    assert isinstance(adapter.migration_source, MigrationSource)
    assert isinstance(adapter.migration_target, MigrationTarget)
    assert isinstance(adapter.editor, SessionEditor)
    assert isinstance(adapter.verifier, SessionVerifier)
    assert isinstance(adapter.lifecycle, SessionLifecycle)
    assert isinstance(adapter.models, ModelCatalog)
    assert adapter.manifest.to_dict() == {
        "id": "fake",
        "display_name": "Fake Agent",
        "icon": "fake",
        "source_path": "~/.fake/sessions",
        "capabilities": list(AGENT_CAPABILITIES),
        "edit_operations": ["rewrite"],
        "executables": ["fake"],
        "fallback_bin_dirs": [],
    }
    assert adapter.browser.scan(cache=None) == []


def test_registry_accepts_injected_fake_adapter():
    registry = AdapterRegistry((_fake_adapter(),))
    assert registry.ids() == ("fake",)
    assert registry.get("fake").id == "fake"


def test_registry_rejects_duplicate_adapter_ids():
    with pytest.raises(ValueError, match="重复的 adapter id"):
        AdapterRegistry((_fake_adapter(), _fake_adapter()))


def test_registry_reports_unknown_adapter():
    registry = AdapterRegistry((_fake_adapter(),))
    with pytest.raises(ToolUnknownError) as excinfo:
        registry.get("missing")
    assert excinfo.value.code == "tool.unknown"


def test_registry_explicitly_composes_all_bundled_adapters():
    registry = create_registry()
    assert registry.ids() == AGENT_IDS
    for agent_id in AGENT_IDS:
        adapter = registry.get(agent_id)
        assert (
            adapter.manifest.edit_operations
            == AGENTS[agent_id]["edit_operations"]
        )
        if adapter.supports("edit"):
            assert adapter.manifest.edit_operations == adapter.editor.operations
        else:
            assert adapter.editor is None


def test_prompt_capability_has_callable_verifier_implementation():
    registry = create_registry()
    prompt_agents = [
        agent_id
        for agent_id in AGENT_IDS
        if "prompt" in AGENTS[agent_id]["capabilities"]
    ]

    assert prompt_agents == list(AGENT_IDS)
    for agent_id in prompt_agents:
        adapter = registry.get(agent_id)
        verifier = adapter.require("prompt", "verifier")
        assert callable(verifier.prompt_session)


def test_adapter_rejects_manifest_editor_operation_mismatch():
    adapter = _fake_adapter()
    manifest = AgentManifest(
        id="fake",
        display_name="Fake Agent",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=AGENT_CAPABILITIES,
        edit_operations=("delete-turn", "rewrite"),
    )
    with pytest.raises(ValueError, match="编辑操作契约不一致"):
        AgentAdapter(
            manifest=manifest,
            browser=adapter.browser,
            migration_source=adapter.migration_source,
            migration_target=adapter.migration_target,
            editor=adapter.editor,
            verifier=adapter.verifier,
            lifecycle=adapter.lifecycle,
            models=adapter.models,
        )


def test_adapter_supports_partial_capabilities_with_matching_components():
    manifest = AgentManifest(
        id="browse-only",
        display_name="Browse Only",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=("browse", "resume"),
        edit_operations=(),
    )
    adapter = AgentAdapter(
        manifest=manifest,
        browser=_FakeBrowser(),
        lifecycle=_FakeLifecycle(),
    )

    assert adapter.supports("browse") is True
    assert adapter.supports("edit") is False
    assert adapter.require("browse", "browser") is adapter.browser
    with pytest.raises(ValueError, match="映射无效"):
        adapter.require("resume", "browser")
    with pytest.raises(ValueError, match="不支持能力"):
        adapter.require("edit", "editor")


def test_adapter_rejects_capability_component_mismatch():
    manifest = AgentManifest(
        id="invalid",
        display_name="Invalid",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=("browse",),
        edit_operations=(),
    )
    with pytest.raises(ValueError, match="capability/component"):
        AgentAdapter(
            manifest=manifest,
            browser=_FakeBrowser(),
            migration_source=_FakeMigrationSource(),
        )


@pytest.mark.parametrize(
    "capabilities",
    [
        ("browse", "browse"),
        ("resume", "browse"),
        ("unknown",),
    ],
)
def test_adapter_rejects_invalid_capability_sequences(capabilities):
    manifest = AgentManifest(
        id="invalid-capabilities",
        display_name="Invalid",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=capabilities,
        edit_operations=(),
    )
    with pytest.raises(ValueError, match="capability 契约无效"):
        AgentAdapter(manifest=manifest)


def test_adapter_requires_edit_capability_operations_and_editor_together():
    without_edit = AgentManifest(
        id="invalid-edit-ops",
        display_name="Invalid",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=(),
        edit_operations=("rewrite",),
    )
    with pytest.raises(ValueError, match="未声明 edit"):
        AgentAdapter(manifest=without_edit)

    without_operations = AgentManifest(
        id="invalid-empty-edit",
        display_name="Invalid",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=("edit",),
        edit_operations=(),
    )
    with pytest.raises(ValueError, match="声明 edit 但未包含"):
        AgentAdapter(manifest=without_operations, editor=_FakeEditor())


def test_lifecycle_can_support_resume_without_delete():
    manifest = AgentManifest(
        id="resume-only",
        display_name="Resume Only",
        icon="fake",
        source_path="~/.fake/sessions",
        capabilities=("resume",),
        edit_operations=(),
    )
    adapter = AgentAdapter(manifest=manifest, lifecycle=_FakeLifecycle())
    assert adapter.require("resume", "lifecycle") is adapter.lifecycle
    assert adapter.supports("delete") is False


@pytest.mark.parametrize(
    ("builders", "missing", "extra"),
    [
        (
            {AGENT_IDS[0]: lambda: None},
            sorted(set(AGENT_IDS[1:])),
            [],
        ),
        (
            {
                **{agent_id: (lambda: None) for agent_id in AGENT_IDS},
                "extra": lambda: None,
            },
            [],
            ["extra"],
        ),
    ],
)
def test_registry_reports_builder_contract_mismatch(
        monkeypatch, builders, missing, extra):
    monkeypatch.setattr(adapter_registry, "ADAPTER_BUILDERS", builders)
    with pytest.raises(ValueError) as excinfo:
        create_registry()
    message = str(excinfo.value)
    assert f"missing={missing}" in message
    assert f"extra={extra}" in message


def test_claude_browser_rejects_subagent_symlink_escape(tmp_path):
    root = tmp_path / "claude"
    root.mkdir()
    session = root / "session.jsonl"
    session.write_text("{}\n")
    ref = NativeSessionReference(str(session), str(root), "file")
    ClaudeBrowser().validate_read_scope(ref)

    child_root = root / "session" / "subagents"
    child_root.mkdir(parents=True)
    outside = tmp_path / "outside.jsonl"
    outside.write_text("{}\n")
    (child_root / "agent-escape.jsonl").symlink_to(outside)
    with pytest.raises(AgentReferenceError, match="超出"):
        ClaudeBrowser().validate_read_scope(ref)


def test_codex_browser_rejects_rollout_symlink_escape(tmp_path):
    root = tmp_path / "codex"
    root.mkdir()
    session = root / "rollout-main.jsonl"
    session.write_text("{}\n")
    ref = NativeSessionReference(str(session), str(root), "file")
    CodexBrowser().validate_read_scope(ref)

    outside = tmp_path / "rollout-outside.jsonl"
    outside.write_text("{}\n")
    (root / "rollout-escape.jsonl").symlink_to(outside)
    with pytest.raises(AgentReferenceError, match="超出"):
        CodexBrowser().validate_read_scope(ref)


def test_opencode_browser_only_accepts_id_backed_read_scope():
    browser = OpenCodeBrowser()
    browser.validate_read_scope(
        NativeSessionReference("session-id", None, "id"),
    )
    with pytest.raises(AgentReferenceError, match="原生 id"):
        browser.validate_read_scope(
            NativeSessionReference("/tmp/session", "/tmp", "file")
        )


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


def test_resume_descriptor_executable_matches_manifest_whitelist(ports):
    from engine.operations import migrate as migration
    for agent_id in ports.adapters():
        adapter = ports.adapter(agent_id)
        descriptor = migration.MigrationService(ports).resume_command(
            agent_id, "sid-1", "/work")
        assert descriptor["executable"] in adapter.manifest.executables
        assert descriptor["args"]
        assert descriptor["executable"] in descriptor["display_command"]
        assert "sid-1" in descriptor["display_command"]
