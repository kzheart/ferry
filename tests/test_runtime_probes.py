"""Runtime probe success is an exact reply contract, not just process success."""
import json
import signal
import subprocess

import pytest

from engine.adapters.claude import probe as claude_probe
from engine.adapters.codex import probe as codex_probe
from engine.adapters.opencode import probe as opencode_probe
from engine.adapters.pi import probe as pi_probe
from engine.system import probes


class _Result:
    def __init__(self, stdout, returncode=0, stderr=""):
        self.stdout = stdout
        self.returncode = returncode
        self.stderr = stderr


@pytest.mark.parametrize("stdout,expected", [
    ("PROBE_OK", True),
    (" PROBE_OK\n", True),
    ("PROBE_OK\nextra", False),
    ("", False),
])
def test_probe_response_must_match_exact_token(stdout, expected):
    assert probes.response_matches(stdout) is expected


def test_claude_probe_rejects_extra_response_text(monkeypatch):
    monkeypatch.setattr(claude_probe.probes, "run", lambda *_args, **_kwargs:
                        _Result(json.dumps({"result": "PROBE_OK\nextra"})))

    report = claude_probe._probe("sid", "/work")

    assert report["status"] == "failed"
    assert report["code"] == "probe.unexpected_response"


@pytest.mark.parametrize("module,call", [
    (codex_probe, lambda module: module._probe_in_env("sid")),
    (opencode_probe, lambda module: module._probe("sid", "/work")),
])
def test_cli_probe_rejects_success_exit_with_wrong_response(monkeypatch, module, call):
    monkeypatch.setattr(module.probes, "run", lambda *_args, **_kwargs:
                        _Result("not the expected reply"))

    report = call(module)

    assert report["status"] == "failed"
    assert report["code"] == "probe.unexpected_response"


def test_pi_rpc_probe_requires_both_load_responses(monkeypatch, tmp_path):
    monkeypatch.setattr(
        pi_probe.subprocess, "run",
        lambda *_args, **_kwargs: _Result(
            '{"type":"response","command":"get_entries","success":true}\n'
        ),
    )
    report = pi_probe._probe_path(str(tmp_path / "session.jsonl"), str(tmp_path))
    assert report["status"] == "failed"


def test_agent_command_timeout_terminates_process_group(monkeypatch):
    class _TimedOutProcess:
        pid = 4321
        returncode = -signal.SIGKILL

        def __init__(self):
            self.communicate_calls = 0

        def communicate(self, input=None, timeout=None):
            self.communicate_calls += 1
            if self.communicate_calls <= 2:
                raise subprocess.TimeoutExpired(["agent"], timeout)
            return "partial stdout", "partial stderr"

        def poll(self):
            return None

    process = _TimedOutProcess()
    popen_kwargs = {}
    signals = []

    def fake_popen(_cmd, **kwargs):
        popen_kwargs.update(kwargs)
        return process

    monkeypatch.setattr(probes.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(probes.os, "killpg", lambda pid, sig: signals.append((pid, sig)))
    monkeypatch.setattr(probes.sys, "platform", "darwin")

    result = probes.run_agent_command(["agent"], input_text="hello", timeout=1)

    assert popen_kwargs["start_new_session"] is True
    assert popen_kwargs["stdin"] is subprocess.PIPE
    assert signals == [(4321, signal.SIGTERM), (4321, signal.SIGKILL)]
    assert result == probes.AgentProcessResult(
        -signal.SIGKILL, "partial stdout", "partial stderr", True,
    )


def test_agent_text_is_bounded():
    text, truncated = probes.normalize_agent_text("x" * 65537)

    assert text == "x" * 65536
    assert truncated is True
    assert probes.normalize_agent_text("ok") == ("ok", False)
