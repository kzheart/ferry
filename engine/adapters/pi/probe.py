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


def _assistant_text(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return None
    return "".join(
        item["text"]
        for item in content
        if isinstance(item, dict)
        and item.get("type") == "text"
        and isinstance(item.get("text"), str)
    )


def _parse_prompt_output(raw):
    events = []
    try:
        for line in split_jsonl_lines(raw):
            if line.strip():
                event = json.loads(line)
                if not isinstance(event, dict):
                    return None
                events.append(event)
    except json.JSONDecodeError:
        return None
    agent_end = next(
        (event for event in reversed(events) if event.get("type") == "agent_end"),
        None,
    )
    if not isinstance(agent_end, dict):
        return None
    messages = agent_end.get("messages")
    if not isinstance(messages, list):
        return None
    assistant = next(
        (
            message
            for message in reversed(messages)
            if isinstance(message, dict) and message.get("role") == "assistant"
        ),
        None,
    )
    if assistant is None:
        return None
    text = _assistant_text(assistant)
    return (assistant, text) if text is not None else None


def _run_prompt_path(path, cwd, prompt, model=None, timeout=360):
    session_path = str(Path(path).resolve())
    command = executables.argv(
        "pi",
        "--mode",
        "json",
        "--session",
        session_path,
        "--approve",
    )
    if model:
        command += ["--model", model]
    result = probes.run_agent_command(
        command,
        cwd=cwd,
        input_text=prompt,
        timeout=timeout,
    )
    params = {"tool": "pi", "exit_code": result.returncode}
    if result.timed_out:
        status, code, text = "failed", "agent_prompt.timeout", ""
    elif result.returncode != 0:
        status, code, text = "failed", "agent_prompt.process_failed", ""
    else:
        parsed = _parse_prompt_output(result.stdout or "")
        if parsed is None:
            status, code, text = "failed", "agent_prompt.invalid_output", ""
        else:
            assistant, text = parsed
            fields = {
                "stopReason": "stop_reason",
                "provider": "provider",
                "model": "model",
                "errorMessage": "error_message",
            }
            for source, target in fields.items():
                if assistant.get(source) is not None:
                    params[target] = assistant[source]
            if assistant.get("stopReason") in {"error", "aborted"}:
                status, code, text = (
                    "failed",
                    "agent_prompt.process_failed",
                    "",
                )
            else:
                status, code = "completed", None
    report = probes.report(
        status,
        code,
        params,
        stdout=result.stdout,
        stderr=result.stderr,
    )
    report["text"], report["text_truncated"] = probes.normalize_agent_text(text)
    return report


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

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        from .adapter import resolve

        return _run_prompt_path(
            resolve(session_id),
            cwd,
            prompt,
            model,
            timeout,
        )

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
