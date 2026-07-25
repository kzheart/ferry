"""Current Grok summary/update/chat v1 structural templates."""
import copy


def extract_templates(capture):
    summary = capture["summary"]
    templates = {"summary": summary}
    for envelope in capture.get("updates", []):
        meta = (envelope.get("params") or {}).get("_meta") or {}
        kind = meta.get("updateType") or (
            (envelope.get("params") or {}).get("update") or {}
        ).get("kind")
        templates.setdefault(f"update.{kind}", envelope)
    for row in capture.get("chat", []):
        templates.setdefault(f"chat.{row.get('type')}", row)
    required = {"summary", "update.UserMessage", "update.AgentMessageChunk",
                "update.ToolCall", "update.ToolCallUpdate",
                "chat.user", "chat.assistant"}
    if not required.issubset(templates):
        raise ValueError("Grok fixture is missing current template records: " +
                         ", ".join(sorted(required - set(templates))))
    return templates


_TEMPLATES = extract_templates({
    "summary": {
        "info": {"id": "fixture-grok-tools", "cwd": "/fixture/grok/tools"},
        "session_summary": "Tools fixture", "generated_title": "Tools fixture",
        "created_at": "2026-07-25T13:00:00Z",
        "updated_at": "2026-07-25T13:00:04Z", "num_messages": 4,
        "num_chat_messages": 4, "current_model_id": "grok-code-fast-1",
        "chat_format_version": 1,
    },
    "updates": [
        {"method": "session/update", "params": {
            "sessionId": "fixture-grok-tools",
            "update": {"kind": "user_message", "content": {
                "type": "text", "text": "Read /fixture/grok/tools/input.txt.",
            }},
            "_meta": {"promptId": "p1", "promptIndex": 0,
                      "updateType": "UserMessage"},
        }},
        {"method": "session/update", "params": {
            "sessionId": "fixture-grok-tools",
            "update": {"content": {"type": "text", "text": "Inspecting "}},
            "_meta": {"promptId": "p1", "promptIndex": 0,
                      "updateType": "AgentMessageChunk", "chunkId": "c1"},
        }},
        {"method": "session/update", "params": {
            "sessionId": "fixture-grok-tools",
            "update": {"kind": "read", "rawInput": {
                "path": "/fixture/grok/tools/input.txt",
            }},
            "_meta": {"promptId": "p1", "promptIndex": 0,
                      "updateType": "ToolCall", "updateParams": {
                          "kind": "read", "status": "Pending",
                          "toolCallId": "tool-1",
                      }},
        }},
        {"method": "session/update", "params": {
            "sessionId": "fixture-grok-tools",
            "update": {"kind": "read", "rawOutput": {
                "FileContent": {
                    "absolute_path": "/fixture/grok/tools/input.txt",
                    "content": "sk-test-fixture", "total_lines": 1,
                },
            }},
            "_meta": {"promptId": "p1", "promptIndex": 0,
                      "updateType": "ToolCallUpdate", "updateParams": {
                          "kind": "read", "status": "Completed",
                          "toolCallId": "tool-1",
                      }},
        }},
    ],
    "chat": [
        {"type": "user", "id": "u1",
         "content": [{"type": "text",
                      "text": "Read /fixture/grok/tools/input.txt."}]},
        {"type": "assistant", "id": "a1", "content": "Inspecting now.",
         "tool_calls": [{"id": "tool-1", "name": "read",
                         "arguments": {
                             "path": "/fixture/grok/tools/input.txt",
                         }}]},
        {"type": "tool_result", "id": "r1", "tool_call_id": "tool-1",
         "content": "sk-test-fixture"},
    ],
})


def templates():
    return copy.deepcopy(_TEMPLATES)
