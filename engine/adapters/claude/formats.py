"""Claude Code native JSONL format profiles."""
from __future__ import annotations

from ..base.formats import FormatProfile, FormatRegistry, VersionRange


def extract_templates(records: list[dict]) -> dict:
    templates = {}
    for record in records:
        if (
            record.get("type") == "user"
            and "user" not in templates
            and isinstance(record.get("message", {}).get("content"), str)
        ):
            templates["user"] = record
        if record.get("type") == "assistant" and "assistant" not in templates:
            templates["assistant"] = record
    if set(templates) != {"user", "assistant"}:
        raise ValueError("Claude fixture must contain user and assistant records")
    return templates


def _v1_templates() -> dict:
    return {
        "user": {
            "parentUuid": None,
            "isSidechain": False,
            "promptId": "fixture-prompt-tools",
            "type": "user",
            "message": {
                "role": "user",
                "content": "Run the fixture shell, write, and read operations.",
            },
            "uuid": "fixture-message-user-tools",
            "cwd": "/fixture/claude/tools",
            "sessionId": "fixture-claude-tools",
            "version": "2.1.204",
        },
        "assistant": {
            "parentUuid": "fixture-message-user-tools",
            "isSidechain": False,
            "type": "assistant",
            "message": {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "fixture-tool-shell",
                        "name": "Bash",
                        "input": {"command": "echo format-fixture-shell-test"},
                    }
                ],
                "stop_reason": "tool_use",
            },
            "uuid": "fixture-message-assistant-shell",
            "cwd": "/fixture/claude/tools",
            "sessionId": "fixture-claude-tools",
            "version": "2.1.204",
        },
    }


FORMATS = FormatRegistry(
    agent="claude",
    profiles=(
        FormatProfile(
            id="claude-jsonl-v1",
            output_version="2.1.204",
            compatible=VersionRange("2.1.204", "2.2.0"),
            tested_versions=("2.1.204",),
            template_factory=_v1_templates,
        ),
    ),
)
