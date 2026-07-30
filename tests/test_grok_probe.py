import json
from pathlib import Path

import pytest

from engine.adapters.grok import probe
from engine.system import executables


class _Result:
    def __init__(self, stdout="", returncode=0, stderr=""):
        self.stdout = stdout
        self.returncode = returncode
        self.stderr = stderr


def _bundle(tmp_path, sid="probe-session"):
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "summary.json").write_text(json.dumps({
        "info": {"id": sid, "cwd": str(tmp_path)},
        "session_summary": "sentinel-native-probe",
        "generated_title": "sentinel-native-probe",
        "created_at": "2026-07-25T12:00:00Z",
        "updated_at": "2026-07-25T12:00:01Z",
        "num_messages": 1, "num_chat_messages": 1,
        "current_model_id": "grok-code-fast-1",
        "chat_format_version": 1,
    }))
    (bundle / "updates.jsonl").write_text(
        json.dumps({"method": "session/update", "params": {
            "sessionId": sid,
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "text", "text": "sentinel-native-probe",
                },
                "_meta": {"promptIndex": 0,
                          "modelId": "grok-code-fast-1"},
            },
            "_meta": {"eventId": "event-1"},
        }}) + "\n"
    )
    (bundle / "chat_history.jsonl").write_text(json.dumps({
        "type": "user", "id": "user-1",
        "content": [{"type": "text", "text": "sentinel-native-probe"}],
    }) + "\n")
    return bundle


def test_probe_requires_export_list_and_search_discovery(
        tmp_path, monkeypatch):
    bundle = _bundle(tmp_path)
    calls = []

    def run(command, **kwargs):
        if Path(command[0]).name == "stat":
            return _Result("apfs")
        calls.append((command, kwargs))
        if "export" in command:
            Path(command[-1]).write_text("# exported")
            return _Result("exported")
        return _Result("probe-session")

    monkeypatch.setattr(probe.subprocess, "run", run)

    report = probe.probe_bundle(bundle)

    assert report["status"] == "passed"
    assert len(calls) == 3
    assert all(call[1]["cwd"] == str(tmp_path) for call in calls)
    assert all(
        Path(call[1]["env"]["GROK_HOME"]).parent == Path(call[1]["env"]["TMPDIR"])
        if "TMPDIR" in call[1]["env"] else True
        for call in calls
    )


def test_probe_rejects_search_that_does_not_return_session(
        tmp_path, monkeypatch):
    bundle = _bundle(tmp_path)

    def run(command, **_kwargs):
        if "export" in command:
            Path(command[-1]).write_text("# exported")
            return _Result("exported")
        if "search" in command:
            return _Result("Total: 0")
        return _Result("probe-session")

    monkeypatch.setattr(probe.subprocess, "run", run)

    report = probe.probe_bundle(bundle)

    assert report["status"] == "failed"
    assert report["code"] == "probe.unexpected_response"


def test_probe_reports_nonzero_official_command(tmp_path, monkeypatch):
    bundle = _bundle(tmp_path)
    monkeypatch.setattr(
        probe.subprocess, "run",
        lambda *_args, **_kwargs: _Result(
            returncode=2, stderr="invalid bundle",
        ),
    )

    report = probe.probe_bundle(bundle)

    assert report["status"] == "failed"
    assert report["code"] == "probe.process_failed"


def test_probe_uses_isolated_fallback_when_recorded_cwd_is_missing(
        tmp_path, monkeypatch):
    bundle = _bundle(tmp_path)
    summary = json.loads((bundle / "summary.json").read_text())
    summary["info"]["cwd"] = str(tmp_path / "deleted-project")
    (bundle / "summary.json").write_text(json.dumps(summary))
    command_cwds = []
    cwd_exists = []

    def run(command, **kwargs):
        if Path(command[0]).name == "stat":
            return _Result("apfs")
        command_cwds.append(Path(kwargs["cwd"]))
        cwd_exists.append(Path(kwargs["cwd"]).is_dir())
        if "export" in command:
            Path(command[-1]).write_text("# exported")
            return _Result("exported")
        return _Result("probe-session")

    monkeypatch.setattr(probe.subprocess, "run", run)

    report = probe.probe_bundle(bundle)

    assert report["status"] == "passed"
    assert all(cwd_exists)
    assert all(path != tmp_path / "deleted-project"
               for path in command_cwds)


@pytest.mark.skipif(
    not Path(executables.argv("grok", "--version")[0]).exists(),
    reason="未安装 Grok Build CLI",
)
def test_probe_accepts_current_official_cli(tmp_path):
    report = probe.probe_bundle(_bundle(tmp_path, "probe-native-session"))

    assert report["status"] == "passed"
