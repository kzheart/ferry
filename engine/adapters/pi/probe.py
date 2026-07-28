"""Pi native load probe using RPC metadata commands only."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from ...system import executables, probes
from ..shared.scanner import split_jsonl_lines


def _probe_path(path: str, cwd=None):
    path = str(Path(path).resolve())
    session_dir = str(Path(path).parent)
    with tempfile.TemporaryDirectory() as config_dir:
        command = executables.argv(
            "pi", "--mode", "rpc", "--session", path,
            "--session-dir", session_dir, "--offline", "--no-extensions",
            "--no-skills", "--no-prompt-templates", "--no-themes",
            "--no-context-files", "--no-tools", "--no-approve",
        )
        payload = (
            '{"id":"entries","type":"get_entries"}\n'
            '{"id":"tree","type":"get_tree"}\n'
        )
        env = os.environ.copy()
        env.update({
            "PI_CODING_AGENT_DIR": config_dir,
            "PI_CODING_AGENT_SESSION_DIR": session_dir,
            "PI_OFFLINE": "1", "PI_SKIP_VERSION_CHECK": "1",
            "PI_TELEMETRY": "0",
        })
        try:
            result = subprocess.run(
                command, cwd=cwd, input=payload, capture_output=True, text=True,
                timeout=30, env=env, **executables.RUN_FLAGS,
            )
        except subprocess.TimeoutExpired as error:
            return probes.timeout_report("pi", error)
    responses = []
    for line in split_jsonl_lines(result.stdout or ""):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            value.get("type") == "response"
            and value.get("command") in {"get_entries", "get_tree"}
            and value.get("id") in {"entries", "tree"}
        ):
            responses.append(value)
    commands = {
        item.get("command") for item in responses
        if item.get("success") is not False
    }
    if result.returncode == 0 and commands == {"get_entries", "get_tree"}:
        return probes.report("passed", stdout=result.stdout, stderr=result.stderr)
    return probes.report(
        "failed", "probe.process_failed",
        {"tool": "pi", "exit_code": result.returncode},
        stdout=result.stdout, stderr=result.stderr,
    )


class PiVerifier:
    def probe(self, session_id, cwd, model=None):
        from .adapter import resolve

        path = resolve(session_id)
        with tempfile.TemporaryDirectory() as directory:
            shadow = Path(directory) / path.name
            shutil.copy(path, shadow)
            report = _probe_path(str(shadow), cwd)
            report["isolation"] = {
                "kind": "shadow_session", "id": str(shadow), "cleaned": True,
            }
            return report

    def probe_edited(self, editor, doc, result, model=None):
        path = Path(result["saved_as"])
        with tempfile.TemporaryDirectory() as directory:
            shadow = Path(directory) / path.name
            shutil.copy(path, shadow)
            report = _probe_path(str(shadow), doc.data[0].get("cwd"))
            report["isolation"] = {
                "kind": "shadow_session", "id": str(shadow), "cleaned": True,
            }
            return report
