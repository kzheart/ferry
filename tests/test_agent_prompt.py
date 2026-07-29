import json
from types import SimpleNamespace

import pytest

from engine.adapters.contracts import filesystem_reference
from engine.app import EngineService
from engine.context import EngineContext
from engine.errors import AgentReferenceError, AgentRequestError
from engine.server.rpc import PROTOCOL, RpcDispatcher
from engine.sessions.index import AgentSessionIndex


class _Verifier:
    def __init__(self):
        self.calls = []

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        self.calls.append(
            (session_id, cwd, prompt, model, timeout),
        )
        return {
            "status": "completed",
            "params": {"tool": "claude", "exit_code": 0},
            "diagnostic": {
                "stdout": "done",
                "stderr": "",
                "truncated": False,
            },
            "text": "done",
            "text_truncated": False,
        }


class _Adapter:
    id = "claude"

    def __init__(self, verifier, *, supports=True, browser=None):
        self.verifier = verifier
        self._supports = supports
        self.browser = browser

    def supports(self, capability):
        return self._supports and capability == "prompt"

    def require(self, capability, component):
        if capability != "prompt" or component != "verifier":
            raise ValueError("unsupported component")
        return self.verifier


class _Index:
    def __init__(self):
        self.current_ref = "fsr_current"
        self.resolve_calls = []
        self.refresh_calls = 0

    def _record(self):
        return SimpleNamespace(
            opaque_ref=self.current_ref,
            tool="claude",
            row={"id": "session-1", "dir": "/tmp/project"},
        )

    def resolve(self, tool, ref, *, pin_content=True):
        self.resolve_calls.append((tool, ref, pin_content))
        if tool != "claude" or ref != self.current_ref:
            raise AgentReferenceError("stale ref")
        return self._record()

    def refresh(self):
        self.refresh_calls += 1
        self.current_ref = "fsr_refreshed"
        return [self._record()]


class _Operations:
    def shutdown(self):
        pass


class _Cache:
    def flush(self):
        pass


class _FileBrowser:
    def __init__(self, root, source):
        self.root = root
        self.source = source

    def scan(self, _cache):
        stat = self.source.stat()
        return [
            {
                "path": str(self.source),
                "id": "session-1",
                "dir": "/tmp/project",
                "updated": stat.st_mtime_ns,
                "size": stat.st_size,
            },
        ]

    def canonicalize(self, row):
        return filesystem_reference(
            row,
            str(self.root),
            self.resolve_ref,
            kind="file",
        )

    def resolve_ref(self, ref):
        return ref

    def fingerprint(self, _ref):
        return "session-1"


class _WritingVerifier(_Verifier):
    def __init__(self, source):
        super().__init__()
        self.source = source

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        self.source.write_text(
            self.source.read_text() + prompt + "\n",
        )
        return super().prompt_session(
            session_id,
            cwd,
            prompt,
            model,
            timeout,
        )


def _application(tmp_path, *, supports=True):
    verifier = _Verifier()
    adapter = _Adapter(verifier, supports=supports)
    ports = EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ("claude",),
        cache_factory=lambda *_: None,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )
    index = _Index()
    return (
        EngineService(ports, index, _Operations()),
        verifier,
        index,
    )


def test_agent_prompt_resolves_pinned_ref_and_returns_fresh_ref(tmp_path):
    application, verifier, index = _application(tmp_path)

    result = application.agent_prompt(
        "claude",
        "fsr_current",
        "finish the task",
        model="model-a",
        timeout_sec=45,
    )

    assert verifier.calls == [
        (
            "session-1",
            "/tmp/project",
            "finish the task",
            "model-a",
            45,
        ),
    ]
    assert index.resolve_calls == [
        ("claude", "fsr_current", True),
    ]
    assert index.refresh_calls == 1
    assert result["next_ref"] == "fsr_refreshed"
    assert result["params"]["session_id"] == "session-1"
    assert result["params"]["model"] == "model-a"
    with pytest.raises(AgentReferenceError):
        index.resolve("claude", "fsr_current", pin_content=True)


@pytest.mark.parametrize(
    ("changes", "field"),
    [
        ({"tool": ""}, "tool"),
        ({"tool": "unknown"}, "tool"),
        ({"ref": ""}, "ref"),
        ({"ref": 1}, "ref"),
        ({"prompt": ""}, "prompt"),
        ({"prompt": "x" * 100_001}, "prompt"),
        ({"model": ""}, "model"),
        ({"model": "x" * 513}, "model"),
        ({"model": "bad\nmodel"}, "model"),
        ({"timeout_sec": True}, "timeout_sec"),
        ({"timeout_sec": 0}, "timeout_sec"),
        ({"timeout_sec": 361}, "timeout_sec"),
        ({"timeout_sec": 1.5}, "timeout_sec"),
    ],
)
def test_agent_prompt_rejects_invalid_inputs(tmp_path, changes, field):
    application, verifier, index = _application(tmp_path)
    params = {
        "tool": "claude",
        "ref": "fsr_current",
        "prompt": "continue",
        "model": None,
        "timeout_sec": 360,
    }
    params.update(changes)

    with pytest.raises(AgentRequestError) as raised:
        application.agent_prompt(**params)

    assert raised.value.params["field"] == field
    assert verifier.calls == []
    assert index.resolve_calls == []


def test_agent_prompt_rpc_dispatches_to_service(tmp_path):
    application, verifier, _index = _application(tmp_path)
    dispatcher = RpcDispatcher(application)
    request = json.dumps(
        {
            "protocol": PROTOCOL,
            "id": "prompt-1",
            "method": "agent_prompt",
            "params": {
                "tool": "claude",
                "ref": "fsr_current",
                "prompt": "continue",
                "timeout_sec": 12,
            },
        },
    )

    response = dispatcher.handle(request)

    assert response["ok"] is True
    assert response["result"]["text"] == "done"
    assert response["result"]["next_ref"] == "fsr_refreshed"
    assert verifier.calls[0][-1] == 12


def test_agent_prompt_allows_multiline_prompt(tmp_path):
    application, verifier, _index = _application(tmp_path)

    application.agent_prompt(
        "claude",
        "fsr_current",
        "first line\nsecond line",
    )

    assert verifier.calls[0][2] == "first line\nsecond line"


def test_agent_prompt_refreshes_before_rejecting_invalid_report(
    tmp_path,
    monkeypatch,
):
    application, verifier, index = _application(tmp_path)
    monkeypatch.setattr(
        verifier,
        "prompt_session",
        lambda *_args, **_kwargs: "invalid",
    )

    with pytest.raises(RuntimeError, match="返回值必须是 object"):
        application.agent_prompt(
            "claude",
            "fsr_current",
            "continue",
        )

    assert index.refresh_calls == 1


def test_agent_prompt_preserves_report_when_ref_refresh_fails(
    tmp_path,
    monkeypatch,
):
    application, _verifier, index = _application(tmp_path)

    def fail_refresh():
        raise RuntimeError("scan failed")

    monkeypatch.setattr(index, "refresh", fail_refresh)

    result = application.agent_prompt(
        "claude",
        "fsr_current",
        "continue",
    )

    assert result["status"] == "completed"
    assert result["text"] == "done"
    assert "next_ref" not in result
    assert result["params"]["ref_refresh_failed"] is True


def test_agent_prompt_write_keeps_stable_ref_resolvable(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    source = root / "session.jsonl"
    source.write_text("before\n")
    browser = _FileBrowser(root, source)
    verifier = _WritingVerifier(source)
    adapter = _Adapter(verifier, browser=browser)
    ports = EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ("claude",),
        cache_factory=_Cache,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )
    index = AgentSessionIndex(ports)
    old_ref = index.refresh()[0].opaque_ref
    application = EngineService(ports, index, _Operations())

    result = application.agent_prompt(
        "claude",
        old_ref,
        "after",
    )

    # ref 是稳定句柄:写入后重扫不换发,同一 ref 直接解析到新记录。
    assert result["next_ref"] == old_ref
    assert index.resolve("claude", result["next_ref"]).row["id"] == "session-1"
    assert index.resolve("claude", old_ref).row["id"] == "session-1"
