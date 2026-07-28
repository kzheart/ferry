"""Runtime probe success is an exact reply contract, not just process success."""
import json
import signal
import subprocess

import pytest

from engine.adapters.claude import probe as claude_probe
from engine.adapters.codex import probe as codex_probe
from engine.adapters.grok import probe as grok_probe
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


def test_claude_agent_prompt_command(monkeypatch):
    results = iter([
        probes.AgentProcessResult(
            0,
            json.dumps({
                "result": "finished",
                "stop_reason": "end_turn",
                "session_id": "sid",
            }),
            "",
            False,
        ),
        probes.AgentProcessResult(7, "process failed", "failed", False),
        probes.AgentProcessResult(0, "not-json", "", False),
    ])
    calls = []
    monkeypatch.setattr(
        claude_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        claude_probe.probes,
        "run_agent_command",
        lambda command, **kwargs: (
            calls.append((command, kwargs)),
            next(results),
        )[1],
    )

    verifier = claude_probe.ClaudeVerifier()
    report = verifier.prompt_session(
        "sid", "/work", "do it", model="claude-test", timeout=21,
    )
    failed = verifier.prompt_session("sid", "/work", "again")
    invalid = verifier.prompt_session("sid", "/work", "invalid")

    assert calls[0] == ([
        "claude",
        "-p",
        "do it",
        "--resume",
        "sid",
        "--output-format",
        "json",
        "--dangerously-skip-permissions",
        "--model",
        "claude-test",
    ], {"cwd": "/work", "timeout": 21})
    assert report["status"] == "completed"
    assert report["text"] == "finished"
    assert report["params"]["stop_reason"] == "end_turn"
    assert failed["status"] == "failed"
    assert failed["code"] == "agent_prompt.process_failed"
    assert failed["params"]["exit_code"] == 7
    assert invalid["code"] == "agent_prompt.invalid_output"


def test_codex_agent_prompt_command(monkeypatch):
    results = iter([
        probes.AgentProcessResult(0, "finished\n", "warning", False),
        probes.AgentProcessResult(9, "partial", "failed", False),
    ])
    calls = []
    monkeypatch.setattr(
        codex_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        codex_probe.probes,
        "run_agent_command",
        lambda command, **kwargs: (
            calls.append((command, kwargs)),
            next(results),
        )[1],
    )

    verifier = codex_probe.CodexVerifier()
    report = verifier.prompt_session(
        "sid", "/work", "do it", model="gpt-test", timeout=22,
    )
    failed = verifier.prompt_session("sid", "/work", "again")

    assert calls[0] == ([
        "codex",
        "exec",
        "resume",
        "--skip-git-repo-check",
        "--dangerously-bypass-approvals-and-sandbox",
        "-m",
        "gpt-test",
        "sid",
        "do it",
    ], {"timeout": 22})
    assert report["status"] == "completed"
    assert report["text"] == "finished\n"
    assert report["diagnostic"]["stderr"] == "warning"
    assert failed["status"] == "failed"
    assert failed["code"] == "agent_prompt.process_failed"
    assert failed["params"]["exit_code"] == 9


def test_opencode_agent_prompt_command(monkeypatch):
    results = iter([
        probes.AgentProcessResult(
            0,
            "\n".join([
                json.dumps({"type": "text", "part": {"text": "first"}}),
                json.dumps({"type": "text", "part": {"text": "finished"}}),
            ]),
            "",
            False,
        ),
        probes.AgentProcessResult(4, "", "failed", False),
        probes.AgentProcessResult(0, "not-json", "", False),
    ])
    calls = []
    monkeypatch.setattr(
        opencode_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        opencode_probe.probes,
        "run_agent_command",
        lambda command, **kwargs: (
            calls.append((command, kwargs)),
            next(results),
        )[1],
    )

    verifier = opencode_probe.OpenCodeVerifier()
    report = verifier.prompt_session(
        "sid", "/work", "do it", model="provider/model", timeout=23,
    )
    failed = verifier.prompt_session("sid", "/work", "again")
    invalid = verifier.prompt_session("sid", "/work", "invalid")

    assert calls[0] == ([
        "opencode",
        "run",
        "-s",
        "sid",
        "--dir",
        "/work",
        "--format",
        "json",
        "--auto",
        "-m",
        "provider/model",
        "do it",
    ], {"cwd": "/work", "timeout": 23})
    assert report["status"] == "completed"
    assert report["text"] == "finished"
    assert failed["status"] == "failed"
    assert failed["code"] == "agent_prompt.process_failed"
    assert failed["params"]["exit_code"] == 4
    assert invalid["code"] == "agent_prompt.invalid_output"


def test_pi_agent_prompt_uses_json_session_and_stdin(monkeypatch, tmp_path):
    path = tmp_path / "session.jsonl"
    calls = []
    output = json.dumps({
        "type": "agent_end",
        "messages": [
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "finished"}],
                "stopReason": "stop",
                "provider": "test-provider",
                "model": "test-model",
            },
        ],
    })
    monkeypatch.setattr(
        pi_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        pi_probe.probes,
        "run_agent_command",
        lambda command, **kwargs: (
            calls.append((command, kwargs)),
            probes.AgentProcessResult(0, output, "", False),
        )[1],
    )
    monkeypatch.setattr(
        "engine.adapters.pi.adapter.resolve",
        lambda _session_id: path,
    )

    report = pi_probe.PiVerifier().prompt_session(
        "sid",
        "/work",
        "do it",
        model="provider/model",
        timeout=24,
    )

    assert calls == [([
        "pi",
        "--mode",
        "json",
        "--session",
        str(path.resolve()),
        "--approve",
        "--model",
        "provider/model",
    ], {"cwd": "/work", "input_text": "do it", "timeout": 24})]
    assert report["status"] == "completed"
    assert report["text"] == "finished"
    assert report["params"]["stop_reason"] == "stop"
    assert report["params"]["provider"] == "test-provider"
    assert report["params"]["model"] == "test-model"


def test_pi_agent_prompt_checks_stop_reason(monkeypatch, tmp_path):
    results = iter([
        probes.AgentProcessResult(
            0,
            json.dumps({
                "type": "agent_end",
                "messages": [{
                    "role": "assistant",
                    "content": [{"type": "text", "text": "partial"}],
                    "stopReason": "error",
                    "errorMessage": "boom",
                }],
            }),
            "",
            False,
        ),
        probes.AgentProcessResult(
            0,
            json.dumps({
                "type": "agent_end",
                "messages": [{
                    "role": "assistant",
                    "content": [{"type": "text", "text": "partial"}],
                    "stopReason": "aborted",
                }],
            }),
            "",
            False,
        ),
        probes.AgentProcessResult(
            0,
            json.dumps({
                "type": "agent_end",
                "messages": [{
                    "role": "assistant",
                    "content": [{"type": "text", "text": "bounded"}],
                    "stopReason": "length",
                }],
            }),
            "",
            False,
        ),
    ])
    monkeypatch.setattr(
        pi_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        pi_probe.probes,
        "run_agent_command",
        lambda *_args, **_kwargs: next(results),
    )

    failed = pi_probe._run_prompt_path(
        tmp_path / "session.jsonl",
        "/work",
        "first",
    )
    aborted = pi_probe._run_prompt_path(
        tmp_path / "session.jsonl",
        "/work",
        "second",
    )
    limited = pi_probe._run_prompt_path(
        tmp_path / "session.jsonl",
        "/work",
        "third",
    )

    assert failed["status"] == "failed"
    assert failed["code"] == "agent_prompt.process_failed"
    assert failed["params"]["stop_reason"] == "error"
    assert failed["params"]["error_message"] == "boom"
    assert aborted["status"] == "failed"
    assert aborted["params"]["stop_reason"] == "aborted"
    assert limited["status"] == "completed"
    assert limited["params"]["stop_reason"] == "length"
    assert limited["text"] == "bounded"


def test_grok_agent_prompt_uses_headless_resume(monkeypatch, tmp_path):
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "summary.json").write_text(json.dumps({
        "info": {"id": "019f0000-0000-7000-8000-000000000000"},
    }))
    calls = []
    monkeypatch.setattr(
        grok_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        grok_probe.probes,
        "run_agent_command",
        lambda command, **kwargs: (
            calls.append((command, kwargs)),
            probes.AgentProcessResult(
                0,
                json.dumps({
                    "text": "finished",
                    "stopReason": "EndTurn",
                    "sessionId": "019f0000-0000-7000-8000-000000000000",
                    "requestId": "request-id",
                }),
                "",
                False,
            ),
        )[1],
    )
    monkeypatch.setattr(
        "engine.adapters.grok.adapter.resolve",
        lambda _session_id: bundle,
    )

    report = grok_probe.GrokVerifier().prompt_session(
        "sid",
        "/work",
        "do it",
        model="grok-test",
        timeout=25,
    )

    assert calls == [([
        "grok",
        "--no-auto-update",
        "--cwd",
        "/work",
        "--resume",
        "019f0000-0000-7000-8000-000000000000",
        "--single",
        "do it",
        "--verbatim",
        "--output-format",
        "json",
        "--always-approve",
        "--model",
        "grok-test",
    ], {"cwd": "/work", "timeout": 25})]
    assert "--session-id" not in calls[0][0]
    assert report["status"] == "completed"
    assert report["text"] == "finished"
    assert report["params"]["stop_reason"] == "EndTurn"
    assert report["params"]["session_id"] == (
        "019f0000-0000-7000-8000-000000000000"
    )
    assert report["params"]["request_id"] == "request-id"


def test_grok_agent_prompt_error_json(monkeypatch, tmp_path):
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "summary.json").write_text(json.dumps({
        "info": {"id": "019f0000-0000-7000-8000-000000000000"},
    }))
    results = iter([
        probes.AgentProcessResult(
            0,
            json.dumps({"type": "error", "message": "failed"}),
            "",
            False,
        ),
        probes.AgentProcessResult(
            7,
            json.dumps({"text": "partial"}),
            "failed",
            False,
        ),
    ])
    monkeypatch.setattr(
        grok_probe.executables,
        "argv",
        lambda tool, *args: [tool, *args],
    )
    monkeypatch.setattr(
        grok_probe.probes,
        "run_agent_command",
        lambda *_args, **_kwargs: next(results),
    )
    monkeypatch.setattr(
        "engine.adapters.grok.adapter.resolve",
        lambda _session_id: bundle,
    )
    verifier = grok_probe.GrokVerifier()

    error_json = verifier.prompt_session("sid", "/work", "first")
    nonzero = verifier.prompt_session("sid", "/work", "second")

    assert error_json["status"] == "failed"
    assert error_json["code"] == "agent_prompt.process_failed"
    assert nonzero["status"] == "failed"
    assert nonzero["code"] == "agent_prompt.process_failed"
    assert nonzero["params"]["exit_code"] == 7
