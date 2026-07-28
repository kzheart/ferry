"""Claude 会话验收探针：真实探测与编辑后的影子副本探测。"""
from __future__ import annotations

import json
import shutil
import uuid
from pathlib import Path

from ...system import executables, probes
from . import editing as claude_edit


def _prompt_report(result, *, status, code=None, params=None, text=""):
    report = probes.report(
        status,
        code,
        params,
        stdout=result.stdout,
        stderr=result.stderr,
    )
    report["text"], report["text_truncated"] = probes.normalize_agent_text(text)
    return report


def _prompt_session(session_id, cwd, prompt, model=None, timeout=360):
    if not cwd:
        raise ValueError("claude prompt 必须提供项目目录")
    command = executables.argv(
        "claude",
        "-p",
        prompt,
        "--resume",
        session_id,
        "--output-format",
        "json",
        "--dangerously-skip-permissions",
    )
    if model:
        command += ["--model", model]
    result = probes.run_agent_command(command, cwd=cwd, timeout=timeout)
    params = {
        "tool": "claude",
        "exit_code": result.returncode,
    }
    if result.timed_out:
        return _prompt_report(
            result,
            status="failed",
            code="agent_prompt.timeout",
            params=params,
        )
    raw = (result.stdout or "").strip()
    try:
        output = json.loads(raw) if raw else None
    except json.JSONDecodeError:
        output = None
    if not isinstance(output, dict):
        code = (
            "agent_prompt.process_failed"
            if result.returncode != 0
            else "agent_prompt.invalid_output"
        )
        return _prompt_report(
            result,
            status="failed",
            code=code,
            params=params,
        )
    for key in ("terminal_reason", "stop_reason", "session_id"):
        if output.get(key) is not None:
            params[key] = output[key]
    if output.get("is_error") or result.returncode != 0:
        return _prompt_report(
            result,
            status="failed",
            code="agent_prompt.process_failed",
            params=params,
        )
    if not isinstance(output.get("result"), str):
        return _prompt_report(
            result,
            status="failed",
            code="agent_prompt.invalid_output",
            params=params,
        )
    return _prompt_report(
        result,
        status="completed",
        params=params,
        text=output["result"],
    )


def _probe(session_id, cwd, model=None):
    if not cwd:
        raise ValueError("claude 探针必须提供 --dir(项目目录)")
    command = executables.argv(
        "claude", "-p", probes.PROBE_PROMPT, "--resume", session_id,
        "--output-format", "json")
    if model:
        command += ["--model", model]
    result = probes.run(command, cwd=cwd)
    raw, error = (result.stdout or "").strip(), (result.stderr or "").strip()
    if result.returncode != 0 and not raw:
        return probes.report("failed", "probe.process_failed",
                             {"tool": "claude", "exit_code": result.returncode},
                             stderr=error)
    try:
        output = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return probes.report("failed", "probe.non_json_output",
                             {"tool": "claude", "exit_code": result.returncode},
                             stdout=raw, stderr=error)
    if output.get("is_error") or result.returncode != 0:
        params = {"tool": "claude", "exit_code": result.returncode}
        for key in ("terminal_reason", "stop_reason", "api_error_status", "session_id"):
            if output.get(key) is not None:
                params[key] = output[key]
        return probes.report("failed", "probe.process_failed", params,
                             stdout=raw, stderr=error)
    reply = str(output.get("result", ""))
    if not probes.response_matches(reply):
        return probes.report("failed", "probe.unexpected_response",
                             {"tool": "claude"}, stdout=reply, stderr=error)
    return probes.report("passed", stdout=reply)


class ClaudeVerifier:
    def probe(self, session_id, cwd, model=None):
        return _probe(session_id, cwd, model)

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        return _prompt_session(session_id, cwd, prompt, model, timeout)

    def probe_edited(self, editor, _doc, result, model=None):
        path = Path(result["saved_as"])
        records = claude_edit.load(path)
        cwd = next((row.get("cwd") for row in records if row.get("cwd")), ".")
        shadow_id = str(uuid.uuid4())
        for row in records:
            if "sessionId" in row:
                row["sessionId"] = shadow_id
        shadow = path.with_name(f"{shadow_id}.jsonl")
        claude_edit.save(shadow, records)
        sidecar = path.with_suffix("")
        shadow_sidecar = shadow.with_suffix("")
        if sidecar.is_dir():
            shutil.copytree(sidecar, shadow_sidecar, dirs_exist_ok=True)
        try:
            rep = _probe(shadow_id, cwd, model)
            rep["isolation"] = {"kind": "shadow_session", "id": shadow_id,
                                "cleaned": True}
            return rep
        finally:
            shadow.unlink(missing_ok=True)
            shutil.rmtree(shadow_sidecar, ignore_errors=True)
