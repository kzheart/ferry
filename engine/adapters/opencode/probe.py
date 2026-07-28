"""OpenCode 会话验收探针：编辑后克隆影子副本探测并清理。"""
from __future__ import annotations

import json

from ...system import executables, probes
from . import payload as payload_builder
from . import reader as opencode_reader
from . import writer as opencode_writer
from . import store as opencode_store


def _content_text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(
            item.get("text", "")
            for item in value
            if isinstance(item, dict)
            and item.get("type") == "text"
            and isinstance(item.get("text"), str)
        )
    return None


def _assistant_text(event):
    event_type = event.get("type")
    if event_type == "text":
        part = event.get("part")
        if isinstance(part, dict):
            return _content_text(part.get("text"))
        return _content_text(event.get("text"))
    if event_type == "assistant" or event.get("role") == "assistant":
        message = event.get("message")
        if isinstance(message, dict):
            return _content_text(message.get("content"))
        return _content_text(event.get("content"))
    if event_type in {"message", "message.updated"}:
        message = event.get("message")
        if not isinstance(message, dict):
            properties = event.get("properties")
            message = properties.get("info") if isinstance(properties, dict) else None
        if isinstance(message, dict) and message.get("role") == "assistant":
            return _content_text(message.get("content"))
    return None


def _parse_prompt_output(raw):
    events = []
    try:
        for line in raw.splitlines():
            if line.strip():
                event = json.loads(line)
                if not isinstance(event, dict):
                    return None, False
                events.append(event)
    except json.JSONDecodeError:
        return None, False
    if not events:
        return None, False
    if any(event.get("type") == "error" for event in events):
        return None, True
    texts = [text for event in events if (text := _assistant_text(event)) is not None]
    return (texts[-1] if texts else None), False


def _prompt_session(session_id, cwd, prompt, model=None, timeout=360):
    working_dir = cwd or "."
    command = executables.argv(
        "opencode",
        "run",
        "-s",
        session_id,
        "--dir",
        working_dir,
        "--format",
        "json",
        "--auto",
    )
    if model:
        command += ["-m", model]
    command.append(prompt)
    result = probes.run_agent_command(command, cwd=working_dir, timeout=timeout)
    params = {
        "tool": "opencode",
        "exit_code": result.returncode,
    }
    if result.timed_out:
        status, code, text = "failed", "agent_prompt.timeout", ""
    elif result.returncode != 0:
        status, code, text = "failed", "agent_prompt.process_failed", ""
    else:
        text, process_failed = _parse_prompt_output(result.stdout or "")
        if process_failed:
            status, code, text = "failed", "agent_prompt.process_failed", ""
        elif text is None:
            status, code, text = "failed", "agent_prompt.invalid_output", ""
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


def _probe(session_id, cwd, model=None):
    command = executables.argv("opencode", "run", "-s", session_id)
    if model:
        command += ["-m", model]
    if cwd:
        command[2:2] = ["--dir", cwd]
    result = probes.run(command + [probes.PROBE_PROMPT], cwd=cwd, timeout=360)
    if result.returncode != 0:
        return probes.report("failed", "probe.process_failed",
                             {"tool": "opencode", "exit_code": result.returncode},
                             stdout=result.stdout, stderr=result.stderr)
    if not probes.response_matches(result.stdout):
        return probes.report("failed", "probe.unexpected_response",
                             {"tool": "opencode"}, stdout=result.stdout,
                             stderr=result.stderr)
    return probes.report("passed", stdout=result.stdout, stderr=result.stderr)


class OpenCodeVerifier:
    def probe(self, session_id, cwd, model=None):
        return _probe(session_id, cwd, model)

    def prompt_session(
        self, session_id, cwd, prompt, model=None, timeout=360,
    ):
        return _prompt_session(session_id, cwd, prompt, model, timeout)

    def probe_edited(self, editor, doc, result, model=None):
        authored = editor.load(result["session_id"])
        tree = authored.tree
        cwd = doc.data.get("info", {}).get("directory") or "."
        shadow_id, _ = opencode_writer.write(
            tree,
            cwd=cwd,
            native_payloads={
                tree.source_id: payload_builder.clone(authored.data),
            },
        )
        try:
            rep = _probe(shadow_id, cwd, model)
            rep["isolation"] = {"kind": "shadow_session",
                                "id": shadow_id, "cleaned": True}
            return rep
        finally:
            try:
                shadow = opencode_reader.read(shadow_id)
                ids = [
                    node.source_id
                    for node in reversed(list(shadow.walk()))
                ]
            except Exception:
                ids = [shadow_id]
            for session_id in ids:
                opencode_store.delete_session(session_id)
