"""Runtime probe success is an exact reply contract, not just process success."""
import json

import pytest

from engine.adapters.claude import probe as claude_probe
from engine.adapters.codex import probe as codex_probe
from engine.adapters.opencode import probe as opencode_probe
from engine.infrastructure import probes


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
