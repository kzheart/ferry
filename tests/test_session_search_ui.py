"""session_search:⌘K 全局搜索的 public RPC。

自建环境而不复用 test_agent_tools 的 agent_environment:那个 fixture 把
search_sessions 换成了绑好 index/content_index 的 partial,而这里要走
production 调用路径(自己传 index),两者会在关键字参数上打架。
"""
from __future__ import annotations

import json

import pytest

from test_agent_tools import (
    Browser, Cache, Editor, Lifecycle, MigrationSource, MigrationTarget,
    Models, Verifier,
)

from engine.adapters.contracts import AgentAdapter, AgentManifest
from engine.app import EngineService
from engine.context import EngineContext
from engine.contracts.agents import AGENT_CAPABILITIES
from engine.errors import AgentRequestError
from engine.server.rpc import PROTOCOL, RpcDispatcher
from engine.sessions.cleanup import CleanupService
from engine.sessions.content_index import ContentIndex
from engine.sessions.index import AgentSessionIndex
from engine.sessions.model import Block, Message, Session
from engine.sessions.search import search_sessions_for_ui


# 片段窗口(120 前 + 240 后)必须撑破 UI 的 160 上限,截断才测得到。
_PADDING = "填" * 400


def _adapter(name: str, root: str, browser: Browser) -> AgentAdapter:
    return AgentAdapter(
        AgentManifest(
            name, name, name, root, AGENT_CAPABILITIES,
            ("delete-turn", "rewrite"),
        ),
        browser,
        migration_source=MigrationSource(browser),
        migration_target=MigrationTarget(), editor=Editor(),
        verifier=Verifier(), lifecycle=Lifecycle(), models=Models(),
    )


class _Operations:
    def shutdown(self):
        pass


@pytest.fixture
def search_environment(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    transcript = root / "session.jsonl"
    transcript.write_text("{}\n")
    claude_rows = [{
        "tool": "claude", "id": "private-claude-id", "path": str(transcript),
        "dir": "/Users/private/secret-project", "title": "支付重构",
        "updated": 2000, "count": 4, "size": transcript.stat().st_size,
        "model": "claude-safe",
    }]
    claude_browser = Browser(
        claude_rows,
        Session(
            source_tool="claude", source_id="private-claude-id",
            cwd="/Users/private/secret-project", title="支付重构",
            messages=[
                Message("user", [Block("text", "帮我看看结算链路")]),
                Message("assistant", [Block(
                    "text", _PADDING + "缓存穿透" + _PADDING,
                )]),
            ],
        ),
        source_path=str(root),
    )
    opencode_rows = [{
        "tool": "opencode", "id": "oc-1", "path": "", "dir": "/tmp/project-b",
        "title": "缓存穿透排查", "updated": 1000, "count": 2, "size": 0,
        "model": "model-b",
    }]
    opencode_browser = Browser(
        opencode_rows,
        Session(
            source_tool="opencode", source_id="oc-1", cwd="/tmp/project-b",
            messages=[Message("user", [Block("text", "另一个会话的正文")])],
        ),
        identity=True,
    )
    adapters = {
        "claude": _adapter("claude", str(root), claude_browser),
        "opencode": _adapter("opencode", "/unused", opencode_browser),
    }
    ports = EngineContext(
        adapter=adapters.__getitem__, adapters=lambda: list(adapters),
        cache_factory=Cache, resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path, version="test",
    )
    index = AgentSessionIndex(ports)
    content = ContentIndex(tmp_path / "content-index.sqlite3")
    yield {
        "ports": ports, "index": index, "content_index": content,
        "transcript": transcript, "claude_browser": claude_browser,
    }
    content.close()


def _search(environment, query="", **params):
    return search_sessions_for_ui(
        query,
        index=environment["index"],
        content_index=environment["content_index"],
        **params,
    )


def test_ui_search_returns_flat_rows_with_best_snippet(search_environment):
    result = _search(search_environment, "缓存穿透")
    assert result["query"] == "缓存穿透"
    assert result["scope"] == "any"
    assert result["content_index"]["ready"] is True
    assert result["returned"] == result["total_matches"] == 2
    assert result["has_more"] is False

    content_hit = next(
        item for item in result["sessions"] if item["tool"] == "claude"
    )
    assert content_hit["matched_in"] == ["content"]
    assert content_hit["match_count"] == 1
    assert (content_hit["message"], content_hit["role"]) == (2, "assistant")
    assert "缓存穿透" in content_hit["snippet"]
    assert content_hit["title"] == "支付重构"
    assert content_hit["project"] == "/Users/private/secret-project"

    title_hit = next(
        item for item in result["sessions"] if item["tool"] == "opencode"
    )
    assert title_hit["matched_in"] == ["metadata"]
    assert (title_hit["snippet"], title_hit["message"]) == ("", None)


def test_ui_search_never_leaks_native_ids_or_paths(search_environment):
    result = _search(search_environment, "缓存穿透")
    for item in result["sessions"]:
        assert item["ref"].startswith("fsr_")
        assert not {"path", "dir", "id", "session_id"} & set(item)
    encoded = json.dumps(result, ensure_ascii=False)
    assert "private-claude-id" not in encoded
    assert str(search_environment["transcript"]) not in encoded


def test_ui_snippet_is_shorter_than_the_agent_facing_one(search_environment):
    snippet = next(
        item["snippet"]
        for item in _search(search_environment, "缓存穿透")["sessions"]
        if item["tool"] == "claude"
    )
    assert len(snippet) == 160


def test_title_scope_skips_content_index_entirely(search_environment):
    result = _search(search_environment, "缓存穿透", scope="title")
    assert "content_index" not in result
    assert [item["tool"] for item in result["sessions"]] == ["opencode"]
    assert result["sessions"][0]["matched_in"] == ["metadata"]
    # metadata 是同一档位的底层名,两个都收。
    assert _search(
        search_environment, "缓存穿透", scope="metadata",
    )["total_matches"] == 1


def test_content_scope_drops_title_only_matches(search_environment):
    result = _search(search_environment, "缓存穿透", scope="content")
    assert [item["tool"] for item in result["sessions"]] == ["claude"]


def test_tools_filter_narrows_to_one_agent(search_environment):
    result = _search(search_environment, "缓存穿透", tools=["opencode"])
    assert [item["tool"] for item in result["sessions"]] == ["opencode"]


def test_limit_is_clamped_instead_of_rejected(search_environment):
    # UI 的翻页参数越界不该让搜索框整个空掉,钳到边界继续出结果。
    assert _search(search_environment, "缓存穿透", limit=999)["returned"] == 2
    assert _search(search_environment, "缓存穿透", limit=0)["returned"] == 1
    assert _search(search_environment, "缓存穿透", limit=None)["returned"] == 2
    with pytest.raises(AgentRequestError):
        _search(search_environment, "缓存穿透", limit="30")


def test_query_and_scope_are_validated(search_environment):
    for query in ("", "   ", None):
        with pytest.raises(AgentRequestError):
            _search(search_environment, query)
    with pytest.raises(AgentRequestError):
        _search(search_environment, "缓存穿透", scope="everything")
    with pytest.raises(AgentRequestError):
        _search(search_environment, "x" * 501)


def _rpc(environment, params, request_id="ui-search-1"):
    application = EngineService(
        environment["ports"], environment["index"], _Operations(),
        environment["content_index"],
        cleanup=CleanupService(environment["index"], environment["ports"]),
    )
    return RpcDispatcher(application).handle(json.dumps({
        "protocol": PROTOCOL,
        "id": request_id,
        "method": "session_search",
        "params": params,
    }))


def test_rpc_exposes_session_search_to_the_ui(search_environment):
    response = _rpc(search_environment, {"query": "缓存穿透", "limit": 5})
    assert response["ok"] is True
    assert response["result"]["returned"] == 2
    assert {item["tool"] for item in response["result"]["sessions"]} == {
        "claude", "opencode",
    }


def test_rpc_reports_missing_query_as_a_parameter_error(search_environment):
    response = _rpc(search_environment, {})
    assert response["ok"] is False
    assert response["error"]["code"] == "rpc.missing_param"
    assert response["error"]["params"] == {
        "param": "query", "message": "缺少参数: query",
    }


def test_rpc_surfaces_scope_validation_as_a_domain_error(search_environment):
    response = _rpc(
        search_environment, {"query": "缓存穿透", "scope": "everything"},
    )
    assert response["ok"] is False
    assert response["error"]["category"] == "validation"
