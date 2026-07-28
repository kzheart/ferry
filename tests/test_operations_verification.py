"""operations/verification.py 的探针入口回归网。

覆盖 harness/probe.py 依赖的三种出口：正常返回、能力缺失、探针超时。
"""
import pytest

from engine.context import EngineContext
from engine.errors import AgentCapabilityError
from engine.operations.verification import (
    ProbeTimeout,
    run_agent_prompt,
    run_probe,
)


class _Verifier:
    def __init__(self, result=None, error=None):
        self.result = result if result is not None else (True, "ok")
        self.error = error
        self.calls = []

    def probe(self, session_id, dirpath, model):
        self.calls.append((session_id, dirpath, model))
        if self.error is not None:
            raise self.error
        return self.result

    def prompt_session(
        self, session_id, dirpath, prompt, model, timeout,
    ):
        self.calls.append(
            (session_id, dirpath, prompt, model, timeout),
        )
        if self.error is not None:
            raise self.error
        return self.result


class _Adapter:
    def __init__(self, verifier, *, supports=True, require_fails=False):
        self.id = "claude"
        self.verifier = verifier
        self._supports = supports
        self._require_fails = require_fails

    def supports(self, _capability):
        return self._supports

    def require(self, _capability, _component):
        if self._require_fails:
            raise ValueError("组件未装配")
        return self.verifier


def _ports(adapter, tmp_path):
    return EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ["claude"],
        cache_factory=lambda *_: None,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )


class _NamedProbeTimeout(RuntimeError):
    """名字与 adapter 侧超时异常一致，但不是同一个类。"""


_NamedProbeTimeout.__name__ = "ProbeTimeout"


def test_run_probe_delegates_to_adapter_verifier(tmp_path):
    verifier = _Verifier(result=(True, "PASS detail"))

    result = run_probe(
        "claude", "session-1", "/tmp/project", "model-a",
        ports=_ports(_Adapter(verifier), tmp_path),
    )

    assert result == (True, "PASS detail")
    assert verifier.calls == [("session-1", "/tmp/project", "model-a")]


def test_run_probe_defaults_dir_and_model_to_none(tmp_path):
    verifier = _Verifier()

    run_probe("claude", "session-1", ports=_ports(_Adapter(verifier), tmp_path))

    assert verifier.calls == [("session-1", None, None)]


def test_run_probe_rejects_adapter_without_probe_capability(tmp_path):
    adapter = _Adapter(_Verifier(), supports=False)

    with pytest.raises(AgentCapabilityError):
        run_probe("claude", "session-1", ports=_ports(adapter, tmp_path))


def test_run_probe_rejects_adapter_without_verifier_component(tmp_path):
    adapter = _Adapter(_Verifier(), require_fails=True)

    with pytest.raises(AgentCapabilityError):
        run_probe("claude", "session-1", ports=_ports(adapter, tmp_path))


def test_run_probe_translates_adapter_timeout_by_class_name(tmp_path):
    verifier = _Verifier(error=_NamedProbeTimeout("探针超时 30s"))

    with pytest.raises(ProbeTimeout, match="探针超时 30s"):
        run_probe("claude", "session-1", ports=_ports(_Adapter(verifier), tmp_path))


def test_run_probe_propagates_other_adapter_errors(tmp_path):
    verifier = _Verifier(error=RuntimeError("CLI 崩溃"))

    with pytest.raises(RuntimeError, match="CLI 崩溃") as raised:
        run_probe("claude", "session-1", ports=_ports(_Adapter(verifier), tmp_path))
    assert not isinstance(raised.value, ProbeTimeout)


def test_run_agent_prompt_delegates_to_prompt_verifier(tmp_path):
    report = {"status": "completed", "params": {}, "text": "done"}
    verifier = _Verifier(result=report)

    result = run_agent_prompt(
        "claude",
        "session-1",
        "finish the task",
        "/tmp/project",
        "model-a",
        ports=_ports(_Adapter(verifier), tmp_path),
        timeout=45,
    )

    assert result == report
    assert verifier.calls == [
        (
            "session-1",
            "/tmp/project",
            "finish the task",
            "model-a",
            45,
        ),
    ]


def test_run_agent_prompt_requires_prompt_capability(tmp_path):
    adapter = _Adapter(_Verifier(), supports=False)

    with pytest.raises(AgentCapabilityError):
        run_agent_prompt(
            "claude",
            "session-1",
            "continue",
            ports=_ports(adapter, tmp_path),
        )
