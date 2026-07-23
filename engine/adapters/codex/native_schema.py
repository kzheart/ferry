"""The single Codex rollout structure supported by Ferry."""
from __future__ import annotations

import copy


def extract_templates(records: list[dict]) -> dict:
    templates = {}
    for record in records:
        record_type = record["type"]
        payload_type = (record.get("payload") or {}).get("type")
        key = f"{record_type}.{payload_type}" if payload_type else record_type
        templates.setdefault(key, record)
        if key == "response_item.message":
            templates.setdefault(f"message.{record['payload']['role']}", record)
    required = {
        "session_meta",
        "response_item.custom_tool_call",
        "response_item.custom_tool_call_output",
        "response_item.function_call",
        "response_item.function_call_output",
        "message.user",
        "message.assistant",
    }
    if not required.issubset(templates):
        missing = ", ".join(sorted(required - set(templates)))
        raise ValueError(f"Codex fixture is missing template records: {missing}")
    return templates


def _current_templates() -> dict:
    return {
        "session_meta": {
            "type": "session_meta",
            "payload": {
                "id": "fixture-codex-tools",
                "session_id": "fixture-codex-tools",
                "cwd": "/fixture/codex/tools",
                "cli_version": "0.144.0",
            },
        },
        "turn_context": {
            "type": "turn_context",
            "payload": {
                "turn_id": "fixture-turn-tools",
                "cwd": "/fixture/codex/tools",
            },
        },
        "response_item.message": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Run the fixture shell, write, and read operations.",
                    }
                ],
            },
        },
        "response_item.custom_tool_call": {
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "id": "fixture-custom-call-shell",
                "status": "completed",
                "call_id": "fixture-call-shell",
                "name": "exec",
                "input": (
                    'const r = await tools.exec_command({"cmd":"echo '
                    'format-fixture-shell-test","workdir":"/fixture/codex/tools"});\n'
                    "text(JSON.stringify(r));\n"
                ),
            },
        },
        "response_item.custom_tool_call_output": {
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call_output",
                "call_id": "fixture-call-shell",
                "output": [
                    {
                        "type": "input_text",
                        "text": '{"exit_code":0,"output":"format-fixture-shell-test\\n"}',
                    }
                ],
            },
        },
        "response_item.function_call": {
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "id": "fixture-function-call-shell",
                "status": "completed",
                "call_id": "fixture-function-shell",
                "name": "exec_command",
                "arguments": (
                    '{"cmd":"pwd","workdir":"/fixture/codex/tools"}'
                ),
            },
        },
        "response_item.function_call_output": {
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "fixture-function-shell",
                "output": "/fixture/codex/tools",
            },
        },
        "message.user": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Run the fixture shell, write, and read operations.",
                    }
                ],
            },
        },
        "message.assistant": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "fixture-message-assistant-tools",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Fixture operations completed.",
                    }
                ],
                "phase": "final_answer",
            },
        },
    }

_TEMPLATES = _current_templates()


def templates() -> dict:
    """Return an independent copy of the current native record templates."""
    return copy.deepcopy(_TEMPLATES)
