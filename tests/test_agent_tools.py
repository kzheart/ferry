from __future__ import annotations

import copy
import json
import os
import sqlite3
from dataclasses import dataclass

import pytest

from engine.adapters.base.plugin import ToolManifest, ToolPlugin
from engine.adapters.opencode import scanner as opencode_scanner
from engine.application import agent_tools
from engine.application import scanning
from engine.application.ports import ApplicationPorts, configure, current
from engine.domain.errors import (
    AgentReferenceError,
    AgentRequestError,
    LocatorStaleError,
)
from engine.domain.model import (
    Block, ImageAsset, Message, Session, ToolCall, text_tool_result,
)
from engine.interfaces.rpc import rpc


class Cache:
    def flush(self):
        pass


class Browser:
    def __init__(self, rows, session, *, identity=False):
        self.rows = rows
        self.session = session
        self.identity = identity
        self.fingerprint_value = "fingerprint-1"

    def scan(self, _cache):
        return list(self.rows)

    def resolve_ref(self, ref):
        return ref

    def read(self, _ref):
        return copy.deepcopy(self.session)

    def read_agent(self, ref):
        return self.read(ref)

    def fingerprint(self, _ref):
        return self.fingerprint_value

    def agent_fingerprint(self, ref):
        return self.fingerprint(ref)


class MigrationSource:
    def __init__(self, browser):
        self.browser = browser

    def export_tree(self, ref):
        return self.browser.read(ref)


class MigrationTarget:
    def plan(self, session):
        return {"lossless": not session.loss, "events": list(session.loss)}

    def preview(self, session, _cwd=None):
        return self.plan(session)

    def write(self, _session, _cwd):
        raise AssertionError("preview must not write")

    def classify_tool_call(self, _tool_call):
        return "native"


@dataclass
class Document:
    revision: str = "revision-1"
    count: int = 4


class Editor:
    name = "claude"

    def __init__(self):
        self.commits = 0
        self.load_calls = 0
        self.preview_load_calls = 0

    def load(self, _ref):
        self.load_calls += 1
        return Document()

    def load_preview(self, _ref):
        self.preview_load_calls += 1
        return Document()

    def stats(self, doc):
        return {"count": doc.count, "size": doc.count * 10}

    def apply_ops(self, doc, ops):
        self.last_ops = copy.deepcopy(ops)
        doc.count -= len(ops)
        return [{"code": "turn.deleted"} for _ in ops]

    def validate(self, _doc):
        pass

    def capabilities(self):
        return {
            "inplace": True,
            "operations": ["delete-turn", "rewrite"],
            "operation_modes": {
                "delete-turn": ["inplace"],
                "rewrite": ["inplace"],
            },
        }

    def commit(self, _doc):
        self.commits += 1
        return {"session_id": "private-id"}

    def snapshot(self, _doc, reason_code="snapshot.before_edit", extra=None):
        return "snapshot-before-agent-edit"

    def restore_snapshot(self, _snapshot, _doc):
        raise AssertionError("successful apply must not restore")

    def saved_revision(self, _result, _doc):
        return "revision-2"


class Verifier:
    def probe(self, _session_id, _cwd, _model=None):
        return {"status": "skipped"}

    def probe_edited(self, _editor, _doc, _result, _model=None):
        return {"status": "skipped"}


class Lifecycle:
    executable = "fake"
    delete_undoable = False

    def resume_descriptor(self, session_id, cwd):
        return {"session_id": session_id, "cwd": cwd,
                "executable": self.executable, "args": [session_id],
                "display_command": f"fake {session_id}"}

    def cleanup(self, _session_id, _dest):
        pass

    def validation_ref(self, session_id, _dest):
        return session_id

    def probe_cwd(self, cwd):
        return cwd

    def delete(self, _plugin, _ref):
        return {"ok": True, "undoable": False}

    def restore_delete(self, _snapshot, _meta):
        return {"ok": True}


class Models:
    def discover(self):
        return [], "fake", None

    def fallback(self):
        return []


def _session():
    return Session(
        source_tool="claude",
        source_id="private-source-id",
        cwd="/Users/private/secret-project",
        title="支付 /Users/private/project",
        messages=[
            Message("user", [Block("text", "第一轮 token=topsecret /tmp/private.txt")]),
            Message("assistant", [
                Block("thinking", "private chain of thought"),
                Block("text", "回答一"),
                Block("tool", tool=ToolCall(
                    "shell", "shell.exec", {"command": "cat /etc/passwd"},
                    text_tool_result("Bearer very-secret-token-value"))),
                Block("image", image=ImageAsset(
                    "image-1", "image/png", "BASE64_PRIVATE",
                    "/tmp/ghp_abcdefghijklmnopqrstuvwxyz.png")),
            ]),
            Message("user", [Block("text", "第二轮 " + "界" * 5000)]),
            Message("assistant", [Block("text", "回答二")]),
        ],
    )


@pytest.fixture
def agent_environment(tmp_path):
    previous = current()
    root = tmp_path / "sessions"
    root.mkdir()
    transcript = root / "session.jsonl"
    transcript.write_text("{}\n")
    editor = Editor()
    rows = [{
        "tool": "claude", "id": "private-id", "path": str(transcript),
        "dir": "/Users/private/secret-project", "title": "支付重构",
        "updated": 2000, "count": 4, "size": transcript.stat().st_size,
        "tokens": {"input": 10, "output": 20, "cache_read": 3, "cache_write": 4},
        "model": "claude-safe",
    }]
    claude_browser = Browser(rows, _session())
    claude = ToolPlugin(
        ToolManifest("claude", "Claude Code", "claude", str(root), "path"),
        claude_browser,
        migration_source=MigrationSource(claude_browser),
        migration_target=MigrationTarget(), editor=editor,
        verifier=Verifier(), lifecycle=Lifecycle(), models=Models(),
    )
    opencode_rows = [{
        "tool": "opencode", "id": "oc-1", "path": "", "dir": "/tmp/project-b",
        "title": "Other", "updated": 1000, "count": 2, "size": 0,
        "tokens": {"input": 1, "output": 2, "cache_read": 0, "cache_write": 0},
        "model": "model-b",
    }]
    opencode_browser = Browser(
        opencode_rows, Session("opencode", "oc-1", "/tmp/project-b"))
    opencode = ToolPlugin(
        ToolManifest("opencode", "OpenCode", "opencode", "/unused", "id"),
        opencode_browser,
        migration_source=MigrationSource(opencode_browser),
        migration_target=MigrationTarget(), editor=Editor(),
        verifier=Verifier(), lifecycle=Lifecycle(), models=Models(),
    )
    plugins = {"claude": claude, "opencode": opencode}
    configure(ApplicationPorts(
        adapter=plugins.__getitem__, adapters=lambda: list(plugins),
        cache_factory=Cache, resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path, version="test",
    ))
    agent_tools.reset_index()
    yield {"root": root, "transcript": transcript, "editor": editor,
           "claude_browser": claude.browser, "opencode_browser": opencode_browser}
    agent_tools.reset_index()
    configure(previous)


def _claude_ref():
    result = agent_tools.search_sessions("支付", limit=20)
    return result["sessions"][0]["ref"]


def test_search_never_exposes_storage_locations(agent_environment):
    result = agent_tools.search_sessions("支付", agents=["claude"], limit=1)
    assert result["returned"] == 1
    item = result["sessions"][0]
    assert item["ref"].startswith("fsr_")
    assert "path" not in item and "dir" not in item and "id" not in item
    assert "/Users/" not in json.dumps(result, ensure_ascii=False)
    with pytest.raises(AgentRequestError):
        agent_tools.search_sessions(limit=51)


def test_library_scan_issues_operation_refs(agent_environment):
    session = next(
        item for item in scanning.scan()["sessions"]
        if item["tool"] == "claude"
    )

    assert session["ref"].startswith("fsr_")
    assert session["revision"]
    assert agent_tools._INDEX.resolve(
        session["tool"], session["ref"],
    ).canonical_ref == str(agent_environment["transcript"])


def test_native_session_id_resolves_to_scoped_reference(agent_environment):
    result = agent_tools.resolve_session("claude", "private-id")
    assert result["tool"] == "claude"
    assert result["ref"].startswith("fsr_")
    assert result["title"] == "支付重构"
    assert "id" not in result and "path" not in result and "dir" not in result
    assert agent_tools.get_session_context("claude", result["ref"])["tool"] == "claude"

    with pytest.raises(AgentReferenceError, match="找不到"):
        agent_tools.resolve_session("claude", "missing-id")
    with pytest.raises(AgentRequestError):
        agent_tools.resolve_session("claude", "bad\nid")


def test_session_read_merges_resolve_context_and_content(agent_environment):
    # native session_id 直接读:内部完成 resolve → ref,默认走上下文分页
    by_id = agent_tools.session_read("claude", session_id="private-id")
    assert by_id["tool"] == "claude"
    assert by_id["mode"] == "context"
    assert by_id["resolved_from_session_id"] is True
    assert by_id["ref"].startswith("fsr_")

    # 显式 ref + terms:切到正文搜索,返回 matches
    ref = by_id["ref"]
    searched = agent_tools.session_read("claude", ref=ref, terms=["支付"])
    assert searched["mode"] == "search"
    assert "matches" in searched
    assert "resolved_from_session_id" not in searched

    # ref/session_id 二选一约束
    with pytest.raises(AgentRequestError):
        agent_tools.session_read("claude")
    with pytest.raises(AgentRequestError):
        agent_tools.session_read("claude", ref=ref, session_id="private-id")


def test_only_current_index_refs_are_accepted(agent_environment):
    ref = _claude_ref()
    with pytest.raises(AgentReferenceError):
        agent_tools.get_session_context("claude", str(agent_environment["transcript"]))
    with pytest.raises(AgentReferenceError):
        agent_tools.get_session_context("opencode", ref)
    with pytest.raises(AgentReferenceError):
        agent_tools.get_session_context("opencode", "oc-arbitrary")


def test_id_backed_ref_rejects_changed_or_recreated_session(agent_environment):
    ref = agent_tools.search_sessions("Other")["sessions"][0]["ref"]
    agent_environment["opencode_browser"].fingerprint_value = "fingerprint-2"
    with pytest.raises(AgentReferenceError, match="扫描后已变化"):
        agent_tools.get_session_context("opencode", ref)


def test_stale_and_symlink_escape_are_rejected(agent_environment, tmp_path):
    ref = _claude_ref()
    old_stat = agent_environment["transcript"].stat()
    agent_environment["transcript"].write_text("[]\n")
    os.utime(agent_environment["transcript"],
             ns=(old_stat.st_atime_ns, old_stat.st_mtime_ns))
    with pytest.raises(AgentReferenceError, match="扫描后已变化"):
        agent_tools.get_session_context("claude", ref)
    refreshed = _claude_ref()
    assert refreshed != ref
    with pytest.raises(AgentReferenceError, match="扫描索引"):
        agent_tools.get_session_context("claude", ref)

    agent_tools.reset_index()
    ref = _claude_ref()
    child_dir = agent_environment["root"] / "session" / "subagents"
    child_dir.mkdir(parents=True)
    outside = tmp_path / "outside.jsonl"
    outside.write_text("{}\n")
    (child_dir / "agent-escape.jsonl").symlink_to(outside)
    with pytest.raises(AgentReferenceError, match="超出"):
        agent_tools.get_session_context("claude", ref)


def test_context_is_turn_bounded_redacted_and_byte_bounded(agent_environment):
    ref = _claude_ref()
    result = agent_tools.get_session_context(
        "claude", ref, from_message=1, limit=4, max_bytes=2048)
    encoded = json.dumps(result, ensure_ascii=False).encode()
    assert len(encoded) <= 2048
    text = encoded.decode()
    assert "topsecret" not in text
    assert "/tmp/" not in text and "/Users/" not in text
    assert "private chain of thought" not in text
    assert "cat /etc/passwd" not in text
    assert "very-secret-token-value" not in text
    assert "BASE64_PRIVATE" not in text
    assert "ghp_" not in text
    assert '"input": "[omitted]"' in text
    assert result["truncation"]["truncated"] is True
    assert result["turn_count"] == 2
    assert result["message_count"] == 4
    assert result["next_from_message"] is not None
    assert result["messages"]
    assert result["message_range"]["to"] == result["messages"][-1]["message"]
    assert all(item["locator"].startswith("fml_")
               for item in result["messages"] if item["editable"])

    with_output = agent_tools.get_session_context(
        "claude", ref, from_message=1, limit=2,
        include_tool_outputs=True, max_bytes=4096)
    output_text = json.dumps(with_output, ensure_ascii=False)
    assert "very-secret-token-value" not in output_text
    assert "[REDACTED]" in output_text


def test_context_locator_is_required_for_agent_rewrite(agent_environment):
    ref = _claude_ref()
    context = agent_tools.get_session_context(
        "claude", ref, from_message=1, limit=2, max_bytes=4096)
    locator = next(item["locator"] for item in context["messages"]
                   if item["role"] == "user" and item["editable"])

    preview = agent_tools.preview_edit(
        "claude", ref,
        ops=[{"op": "rewrite", "locator": locator, "text": "委婉文本"}])
    assert preview["mode"] == "edit"
    assert agent_environment["editor"].last_ops[0]["locator"] == "index:0"
    assert not agent_environment["editor"].last_ops[0]["locator"].startswith("fml_")

    with pytest.raises(AgentReferenceError, match="Engine 签发") as error:
        agent_tools.preview_edit(
            "claude", ref,
            ops=[{"op": "rewrite", "locator": "turn:1", "text": "错误定位"}])
    assert "messages[].locator" in error.value.params["hint"]

    with pytest.raises(LocatorStaleError) as stale:
        agent_tools.preview_edit(
            "claude", ref,
            ops=[{"op": "rewrite", "locator": "fml_missing", "text": "错误定位"}])
    assert stale.value.retryable is True
    assert "messages[].locator" in stale.value.params["hint"]


def test_content_search_returns_editable_message_locators(agent_environment):
    ref = _claude_ref()
    result = agent_tools.search_session_content(
        "claude", ref, ["第二轮", "回答一"], roles=["user", "assistant"])
    assert result["total_matches"] == 2
    assert result["returned"] == 2
    assert result["turn_count"] == 2
    assert {item["role"] for item in result["matches"]} == {"user", "assistant"}
    assert all(item["locator"].startswith("fml_") for item in result["matches"])
    assert all("topsecret" not in item["snippet"] for item in result["matches"])
    assert all(isinstance(item["complete"], bool) for item in result["matches"])

    user = next(item for item in result["matches"] if item["role"] == "user")
    agent_tools.preview_edit(
        "claude", ref,
        ops=[{"op": "rewrite", "locator": user["locator"], "text": "新的第二轮"}])
    assert agent_environment["editor"].last_ops[0]["locator"] == "index:2"

    with pytest.raises(AgentRequestError):
        agent_tools.search_session_content("claude", ref, [], limit=20)


def test_context_paginates_long_single_turn_by_message(agent_environment):
    session = agent_environment["claude_browser"].session
    session.messages = [Message("user", [Block("text", "问题")])] + [
        Message("assistant", [Block("text", f"片段 {index}")])
        for index in range(1, 6)
    ]
    ref = _claude_ref()

    first = agent_tools.get_session_context(
        "claude", ref, from_message=1, limit=2, max_bytes=4096)
    assert first["turn_count"] == 1
    assert [item["message"] for item in first["messages"]] == [1, 2]
    assert first["next_from_message"] == 3

    second = agent_tools.get_session_context(
        "claude", ref, from_message=first["next_from_message"],
        limit=2, max_bytes=4096)
    assert [item["message"] for item in second["messages"]] == [3, 4]
    assert all(item["turn"] == 1 for item in second["messages"])


def test_redaction_covers_cross_platform_paths_and_common_credentials():
    private_key = (
        "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----")
    value = (
        "/root/a /Volumes/drive/a /mnt/a C:/Users/alice/private D:\\private\\a "
        "\\\\server\\share\\a "
        "file:///root/a ~/secret ghp_abcdefghijklmnopqrstuvwxyz "
        "github_pat_abcdefghijklmnopqrstuvwxyz gho_abcdefghijklmnopqrstuvwxyz "
        "ghu_abcdefghijklmnopqrstuvwxyz ghs_abcdefghijklmnopqrstuvwxyz "
        "ghr_abcdefghijklmnopqrstuvwxyz AKIAABCDEFGHIJKLMNOP "
        "AWS_SECRET_ACCESS_KEY=supersecretvalue xoxb-1234567890123456 "
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nenc\n"
        "-----END ENCRYPTED PRIVATE KEY----- "
        "-----BEGIN DSA PRIVATE KEY-----\ndsa\n-----END DSA PRIVATE KEY----- "
        + private_key)
    redacted = agent_tools._redact(value)
    for secret in ("/root", "/Volumes", "/mnt", "C:/", "D:\\", "server", "ghp_",
                   "github_pat_", "gho_", "ghu_", "ghs_", "ghr_", "AKIA",
                   "abc123", "enc", "dsa"):
        assert secret not in redacted
    assert "supersecretvalue" not in redacted and "xoxb-" not in redacted


def test_usage_is_aggregated_without_raw_session_data(agent_environment):
    result = agent_tools.get_usage(agents=["claude"])
    assert result == {
        "sessions": 1,
        "tokens": {"input": 10, "output": 20, "cache_read": 3, "cache_write": 4},
        "by_agent": {
            "claude": {"input": 10, "output": 20, "cache_read": 3, "cache_write": 4}
        },
        "cost": None,
        "currency": "USD",
        "filters": {
            "agents": ["claude"],
            "projects": None,
            "time_range": {"from": None, "to": None},
        },
    }
    assert "private-id" not in json.dumps(result)


def test_agent_rpc_returns_stable_structured_errors(agent_environment):
    response = rpc(json.dumps({
        "method": "agent_session_read", "request_id": "agent-1",
        "params": {"tool": "claude", "ref": "/tmp/not-issued.jsonl"},
    }))
    assert response["ok"] is False
    assert response["error"] == {
        "code": "agent.reference_invalid",
        "params": {},
        "category": "validation",
        "retryable": False,
        "request_id": "agent-1",
    }


def test_engine_revalidates_limits_without_relying_on_sidecar(agent_environment):
    with pytest.raises(AgentRequestError):
        agent_tools.search_sessions(agents=[f"tool-{index}" for index in range(9)])
    with pytest.raises(AgentRequestError):
        agent_tools.search_sessions(time_range={"from": float("nan")})
    ref = _claude_ref()
    with pytest.raises(AgentRequestError):
        agent_tools.preview_edit(
            "claude", ref,
            ops=[{"op": "delete-turn", "turn": 1}] * 51)
    nested = {}
    cursor = nested
    for _ in range(10):
        cursor["next"] = {}
        cursor = cursor["next"]
    with pytest.raises(AgentRequestError):
        agent_tools.preview_edit(
            "claude", ref,
            ops=[{"op": "rewrite", "locator": "fml_invalid", "text": nested}])


def test_opencode_fingerprint_detects_update_and_delete(tmp_path, monkeypatch):
    database_path = tmp_path / "opencode.db"
    with sqlite3.connect(database_path) as database:
        database.execute("CREATE TABLE session (id TEXT PRIMARY KEY, data TEXT)")
        database.execute("CREATE TABLE message (id TEXT, session_id TEXT, data TEXT)")
        database.execute("CREATE TABLE part (id TEXT, session_id TEXT, data TEXT)")
        database.execute("INSERT INTO session VALUES ('s1', '{}')")
        database.execute("INSERT INTO message VALUES ('m1', 's1', '{}')")
        database.execute("INSERT INTO part VALUES ('p1', 's1', '{\"text\":\"a\"}')")
    monkeypatch.setattr(opencode_scanner, "OPENCODE_DB", database_path)
    first = opencode_scanner.fingerprint("s1")
    with sqlite3.connect(database_path) as database:
        database.execute(
            "UPDATE part SET data = '{\"text\":\"b\"}' WHERE id = 'p1'")
    assert opencode_scanner.fingerprint("s1") != first
    with sqlite3.connect(database_path) as database:
        database.execute("DELETE FROM session WHERE id = 's1'")
    assert opencode_scanner.fingerprint("s1") is None


def test_path_ref_rejects_changed_child_tree(agent_environment):
    child_dir = agent_environment["root"] / "session" / "subagents"
    child_dir.mkdir(parents=True)
    child = child_dir / "agent-child.jsonl"
    child.write_text("{}\n")
    ref = _claude_ref()
    agent_environment["claude_browser"].fingerprint_value = "fingerprint-2"
    with pytest.raises(AgentReferenceError, match="扫描后已变化"):
        agent_tools.get_session_context("claude", ref)


def test_search_dto_is_byte_bounded(agent_environment):
    browser = agent_environment["claude_browser"]
    browser.rows.clear()
    for index in range(50):
        transcript = agent_environment["root"] / f"session-{index}.jsonl"
        transcript.write_text("{}\n")
        browser.rows.append({
            "tool": "claude", "id": f"private-{index}",
            "path": str(transcript), "dir": "/tmp/" + "项" * 256,
            "title": "标题" * 200, "updated": 2000 - index,
            "count": 4, "size": transcript.stat().st_size,
            "model": "模型" * 120,
        })
    result = agent_tools.search_sessions(limit=50)
    assert len(json.dumps(result, ensure_ascii=False).encode("utf-8")) <= 64 * 1024
    assert result["returned"] < 50
    assert result["has_more"] is True
    assert result["truncation"]["reason"] == "byte_budget"


def test_previews_are_narrow_and_do_not_write(agent_environment):
    ref = _claude_ref()
    migration = agent_tools.preview_migration("claude", ref, "opencode", max_turn=1)
    assert migration["source_tool"] == "claude"
    assert "cwd" not in migration and "source_id" not in migration

    edit = agent_tools.preview_edit(
        "claude", ref, ops=[{"op": "delete-turn", "turn": 1}])
    assert edit["mode"] == "edit"
    assert edit["revision"] == "revision-1"
    assert agent_environment["editor"].commits == 0
    assert agent_environment["editor"].load_calls == 0
    assert agent_environment["editor"].preview_load_calls == 1
    assert "saved_as" not in edit
