"""Validate Grok bundles with official export/list/search commands."""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import quote

from ...system import executables, probes


def _prompt_session(session_id, cwd, prompt, model=None, timeout=360):
    from .adapter import resolve

    bundle = resolve(session_id)
    summary = json.loads((bundle / "summary.json").read_text())
    native_id = str(summary["info"]["id"])
    command = executables.argv(
        "grok",
        "--no-auto-update",
        "--cwd",
        cwd,
        "--resume",
        native_id,
        "--single",
        prompt,
        "--verbatim",
        "--output-format",
        "json",
        "--always-approve",
    )
    if model:
        command += ["--model", model]
    result = probes.run_agent_command(command, cwd=cwd, timeout=timeout)
    params = {"tool": "grok", "exit_code": result.returncode}
    if result.timed_out:
        status, code, text = "failed", "agent_prompt.timeout", ""
    else:
        raw = (result.stdout or "").strip()
        try:
            output = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            output = None
        if not isinstance(output, dict):
            status = "failed"
            code = (
                "agent_prompt.process_failed"
                if result.returncode != 0
                else "agent_prompt.invalid_output"
            )
            text = ""
        else:
            fields = {
                "stopReason": "stop_reason",
                "sessionId": "session_id",
                "requestId": "request_id",
            }
            for source, target in fields.items():
                if output.get(source) is not None:
                    params[target] = output[source]
            if result.returncode != 0 or output.get("type") == "error":
                status, code, text = (
                    "failed",
                    "agent_prompt.process_failed",
                    "",
                )
            elif not isinstance(output.get("text"), str):
                status, code, text = (
                    "failed",
                    "agent_prompt.invalid_output",
                    "",
                )
            else:
                status, code, text = "completed", None, output["text"]
    report = probes.report(
        status,
        code,
        params,
        stdout=result.stdout,
        stderr=result.stderr,
    )
    report["text"], report["text_truncated"] = probes.normalize_agent_text(text)
    return report


def probe_bundle(bundle: Path):
    from .writer import index_bundle

    summary = json.loads((bundle / "summary.json").read_text())
    sid, cwd = summary["info"]["id"], summary["info"]["cwd"]
    title = str(
        summary.get("generated_title")
        or summary.get("session_summary")
        or sid
    )
    sentinel = next(
        (token for token in re.findall(r"[\w-]+", title)
         if len(token) >= 3),
        "Migrated",
    )
    with tempfile.TemporaryDirectory() as directory:
        home = Path(directory)
        sessions = home / "sessions"
        command_cwd = Path(cwd)
        if not command_cwd.is_dir():
            command_cwd = home / "probe-cwd"
            command_cwd.mkdir()
        target = sessions / quote(
            str(command_cwd.resolve()), safe="",
        ) / sid
        target.parent.mkdir(parents=True)
        shutil.copytree(bundle, target)
        if str(command_cwd) != cwd:
            shadow_summary = json.loads(
                (target / "summary.json").read_text()
            )
            shadow_summary["info"]["cwd"] = str(command_cwd)
            (target / "summary.json").write_text(json.dumps(
                shadow_summary, ensure_ascii=False,
            ))
        index_bundle(target, sessions)
        env = os.environ.copy()
        env["GROK_HOME"] = str(home)
        commands = (
            executables.argv("grok", "export", sid, str(home / "export.md")),
            executables.argv("grok", "sessions", "list"),
            executables.argv("grok", "sessions", "search", sentinel),
        )
        outputs = []
        for command in commands:
            try:
                result = subprocess.run(
                    command, capture_output=True, text=True, timeout=30,
                    env=env, cwd=str(command_cwd), **executables.RUN_FLAGS,
                )
            except subprocess.TimeoutExpired as error:
                return probes.timeout_report("grok", error)
            outputs.append((result.stdout or "") + (result.stderr or ""))
            if result.returncode:
                return probes.report(
                    "failed", "probe.process_failed",
                    {"tool": "grok", "exit_code": result.returncode},
                    stdout=result.stdout, stderr=result.stderr,
                )
        exported = home / "export.md"
        if (
            not exported.is_file()
            or sid not in outputs[1]
            or sid not in outputs[2]
        ):
            return probes.report(
                "failed", "probe.unexpected_response", {"tool": "grok"},
                stdout="\n".join(outputs),
            )
        return probes.report("passed", stdout="\n".join(outputs))


class GrokVerifier:
    def probe(self, session_id, cwd, model=None):
        from .adapter import resolve

        return probe_bundle(resolve(session_id))

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        return _prompt_session(session_id, cwd, prompt, model, timeout)

    def probe_edited(self, editor, doc, result, model=None):
        return probe_bundle(Path(result["saved_as"]))
