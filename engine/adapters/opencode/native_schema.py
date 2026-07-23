"""The single OpenCode export/import structure supported by Ferry."""
from __future__ import annotations

import copy
import json


def export_from_capture(capture: dict) -> dict:
    info = (
        json.loads(capture["session"]["data"])
        if "data" in capture["session"]
        else dict(capture["session"])
    )
    parts = {}
    for row in capture.get("parts", []):
        part = json.loads(row["data"])
        parts.setdefault(row["message_id"], []).append(part)
    messages = [
        {
            "info": json.loads(row["data"]),
            "parts": parts.get(row["id"], []),
        }
        for row in capture.get("messages", [])
    ]
    return {"info": info, "messages": messages}


def extract_templates(capture: dict) -> dict:
    data = export_from_capture(capture)
    templates = {"info": data["info"]}
    for message in data["messages"]:
        role = message["info"].get("role")
        templates.setdefault(f"msg.{role}", message["info"])
        for part in message["parts"]:
            templates.setdefault(f"part.{part.get('type')}", part)
    required = {"info", "msg.user", "msg.assistant", "part.text", "part.tool"}
    if not required.issubset(templates):
        missing = ", ".join(sorted(required - set(templates)))
        raise ValueError(f"OpenCode fixture is missing template records: {missing}")
    return templates


def _current_templates() -> dict:
    return {
        "info": {
            "id": "fixture-opencode-tools",
            "directory": "/fixture/opencode/tools",
            "title": "Tools fixture",
            "version": "1.18.3",
        },
        "msg.user": {
            "id": "fixture-message-user-tools",
            "sessionID": "fixture-opencode-tools",
            "role": "user",
        },
        "part.text": {
            "id": "fixture-part-user-tools",
            "messageID": "fixture-message-user-tools",
            "sessionID": "fixture-opencode-tools",
            "type": "text",
            "text": "Run the fixture shell, write, and read operations.",
        },
        "msg.assistant": {
            "id": "fixture-message-assistant-tools",
            "sessionID": "fixture-opencode-tools",
            "parentID": "fixture-message-user-tools",
            "role": "assistant",
            "finish": "tool-calls",
        },
        "part.tool": {
            "id": "fixture-part-shell",
            "messageID": "fixture-message-assistant-tools",
            "sessionID": "fixture-opencode-tools",
            "type": "tool",
            "tool": "bash",
            "callID": "fixture-call-shell",
            "state": {
                "status": "completed",
                "input": {"command": "echo format-fixture-shell-test"},
                "output": "format-fixture-shell-test\n",
                "metadata": {"exit": 0},
            },
        },
    }

_TEMPLATES = _current_templates()


def templates() -> dict:
    """Return an independent copy of the current native record templates."""
    return copy.deepcopy(_TEMPLATES)
