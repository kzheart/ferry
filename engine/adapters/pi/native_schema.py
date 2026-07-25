"""The single current Pi session structure supported by Ferry."""
from __future__ import annotations

import copy


def extract_templates(records: list[dict]) -> dict:
    templates: dict[str, dict] = {}
    for record in records:
        kind = record.get("type")
        key = kind
        if kind == "message":
            message = record.get("message") or {}
            key = f"message.{message.get('role')}"
            for part in message.get("content", []) if isinstance(message.get("content"), list) else []:
                templates.setdefault(f"content.{part.get('type')}", part)
        if isinstance(key, str):
            templates.setdefault(key, record)
    required = {
        "session", "message.user", "message.assistant",
        "message.toolResult", "content.text", "content.toolCall",
        "message.bashExecution", "content.thinking", "content.image",
        "compaction",
    }
    if not required.issubset(templates):
        missing = ", ".join(sorted(required - set(templates)))
        raise ValueError(f"Pi fixture is missing template records: {missing}")
    return templates


_TEMPLATES = extract_templates([
    {"type": "session", "version": 3, "id": "fixture-pi-tools",
     "timestamp": "2026-07-25T10:00:00.000Z", "cwd": "/fixture/pi/tools"},
    {"type": "message", "id": "u1", "parentId": None,
     "timestamp": "2026-07-25T10:00:01.000Z",
     "message": {"role": "user", "content": [
         {"type": "text", "text": "Inspect /fixture/pi/tools and token sk-test-fixture."},
         {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
     ], "timestamp": 1784973601000}},
    {"type": "message", "id": "a1", "parentId": "u1",
     "timestamp": "2026-07-25T10:00:02.000Z",
     "message": {"role": "assistant", "content": [
         {"type": "thinking", "thinking": "Use two tools."},
         {"type": "text", "text": "I will inspect it."},
         {"type": "toolCall", "id": "call-1", "name": "read",
          "arguments": {"path": "/fixture/pi/tools/input.txt"}},
     ], "api": "anthropic-messages", "provider": "fixture",
         "model": "fixture-model", "usage": {"input": 10, "output": 5,
         "cacheRead": 2, "cacheWrite": 1, "totalTokens": 18,
         "cost": {"input": 0, "output": 0, "cacheRead": 0,
                  "cacheWrite": 0, "total": 0}},
         "stopReason": "toolUse", "timestamp": 1784973602000}},
    {"type": "message", "id": "r1", "parentId": "a1",
     "timestamp": "2026-07-25T10:00:03.000Z",
     "message": {"role": "toolResult", "toolCallId": "call-1",
         "toolName": "read", "content": [{"type": "text", "text": "fixture output"}],
         "isError": False, "timestamp": 1784973603000}},
    {"type": "message", "id": "b1", "parentId": "r1",
     "timestamp": "2026-07-25T10:00:03.500Z",
     "message": {"role": "bashExecution", "command": "pwd",
         "output": "/fixture/pi/tools\n", "exitCode": 0,
         "cancelled": False, "truncated": False,
         "timestamp": 1784973603500}},
    {"type": "compaction", "id": "c1", "parentId": "b1",
     "timestamp": "2026-07-25T10:00:04.000Z",
     "summary": "Fixture summary", "firstKeptEntryId": "u1",
     "tokensBefore": 123},
])


def templates() -> dict:
    return copy.deepcopy(_TEMPLATES)
