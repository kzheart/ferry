"""Operation 输入的严格形状校验与规范化。"""
from __future__ import annotations

import json

from ..errors import AgentRequestError
from ..sessions import catalog as agent_tools
from .plan_store import canonical_json
from .types import AssistantReply


def validate_edit_input(value: dict) -> dict:
    allowed = {"kind", "tool", "ref", "ops", "probe"}
    if set(value) - allowed:
        raise AgentRequestError(
            "edit operation 包含未知字段",
            {"fields": sorted(set(value) - allowed)},
        )
    tool, ref, ops = value.get("tool"), value.get("ref"), value.get("ops")
    probe = value.get("probe", False)
    if not isinstance(tool, str) or not tool:
        raise AgentRequestError("operation tool 非法")
    if not isinstance(ref, str) or not ref:
        raise AgentRequestError("operation ref 非法")
    if not isinstance(probe, bool):
        raise AgentRequestError("operation probe 必须是布尔值")
    ops = validate_ops(ops)
    if len(canonical_json(ops).encode()) > 64 * 1024:
        raise AgentRequestError("ops 超过 64 KiB")
    return json.loads(canonical_json({
        "kind": "edit",
        "tool": tool,
        "ref": ref,
        "ops": ops,
        "probe": probe,
    }))


def validate_migration_input(value: dict, adapters: tuple[str, ...]) -> dict:
    allowed = {
        "kind", "source_tool", "ref", "target_tool",
        "max_turn", "probe", "probe_model",
    }
    unknown = set(value) - allowed
    if unknown:
        raise AgentRequestError(
            "migration operation 包含未知字段",
            {"fields": sorted(unknown)},
        )
    source_tool = value.get("source_tool")
    target_tool = value.get("target_tool")
    ref = value.get("ref")
    if not isinstance(source_tool, str) or not 1 <= len(source_tool) <= 64:
        raise AgentRequestError("migration source_tool 非法")
    if not isinstance(target_tool, str) or not 1 <= len(target_tool) <= 64:
        raise AgentRequestError("migration target_tool 非法")
    if source_tool not in adapters or target_tool not in adapters:
        raise AgentRequestError("migration Agent 非法")
    if source_tool == target_tool:
        raise AgentRequestError("migration 源和目标不能相同")
    if (
        not isinstance(ref, str)
        or not 1 <= len(ref) <= 512
        or any(ord(character) < 33 for character in ref)
    ):
        raise AgentRequestError("migration ref 非法")
    probe = value.get("probe", False)
    if not isinstance(probe, bool):
        raise AgentRequestError("migration probe 必须是布尔值")
    max_turn = value.get("max_turn")
    if max_turn is not None and (
        isinstance(max_turn, bool)
        or not isinstance(max_turn, int)
        or not 1 <= max_turn <= 1_000_000
    ):
        raise AgentRequestError("migration max_turn 非法")
    probe_model = value.get("probe_model")
    if probe_model is not None and (
        not isinstance(probe_model, str)
        or not 1 <= len(probe_model) <= 512
        or any(ord(character) < 32 for character in probe_model)
    ):
        raise AgentRequestError("migration probe_model 非法")
    result = {
        "kind": "migration",
        "source_tool": source_tool,
        "ref": ref,
        "target_tool": target_tool,
        "probe": probe,
    }
    if max_turn is not None:
        result["max_turn"] = max_turn
    if probe_model is not None:
        result["probe_model"] = probe_model
    return json.loads(canonical_json(result))


def validate_metadata_input(value: dict) -> dict:
    allowed = {"kind", "tool", "ref", "patch"}
    unknown = set(value) - allowed
    if unknown:
        raise AgentRequestError(
            "metadata operation 包含未知字段",
            {"fields": sorted(unknown)},
        )
    tool = value.get("tool")
    ref = value.get("ref")
    patch = value.get("patch")
    if not isinstance(tool, str) or not 1 <= len(tool) <= 64:
        raise AgentRequestError("metadata tool 非法")
    if (
        not isinstance(ref, str)
        or not 1 <= len(ref) <= 512
        or any(ord(character) < 33 for character in ref)
    ):
        raise AgentRequestError("metadata ref 非法")
    allowed_fields = {"name", "pinned", "archived", "tags"}
    if not isinstance(patch, dict) or not patch or not set(patch) <= allowed_fields:
        raise AgentRequestError("metadata patch 字段非法")
    agent_tools._validate_json_shape(patch, max_depth=3, max_nodes=50)
    if (
        "name" in patch
        and (not isinstance(patch["name"], str) or len(patch["name"]) > 200)
    ):
        raise AgentRequestError("metadata name 非法")
    for field in ("pinned", "archived"):
        if field in patch and not isinstance(patch[field], bool):
            raise AgentRequestError(f"metadata {field} 必须是 boolean")
    if "tags" in patch:
        tags = patch["tags"]
        if (
            not isinstance(tags, list)
            or len(tags) > 20
            or not all(
                isinstance(tag, str) and 1 <= len(tag) <= 64
                for tag in tags
            )
        ):
            raise AgentRequestError("metadata tags 非法")
    if len(canonical_json(patch).encode()) > 4096:
        raise AgentRequestError("metadata patch 超过 4 KiB")
    return json.loads(canonical_json({
        "kind": "metadata",
        "tool": tool,
        "ref": ref,
        "patch": patch,
    }))


def validate_delete_input(value: dict, adapters: tuple[str, ...]) -> dict:
    allowed = {"kind", "tool", "ref"}
    unknown = set(value) - allowed
    if unknown:
        raise AgentRequestError(
            "delete operation 包含未知字段",
            {"fields": sorted(unknown)},
        )
    tool = value.get("tool")
    ref = value.get("ref")
    if tool not in adapters:
        raise AgentRequestError("delete tool 非法")
    if (
        not isinstance(ref, str)
        or not 1 <= len(ref) <= 512
        or any(ord(character) < 33 for character in ref)
    ):
        raise AgentRequestError("delete ref 非法")
    return {"kind": "delete", "tool": tool, "ref": ref}


def validate_restore_delete_input(value: dict) -> dict:
    if set(value) != {"kind", "recovery_id"}:
        raise AgentRequestError("restore-delete operation 参数非法")
    recovery_id = value.get("recovery_id")
    if (
        not isinstance(recovery_id, str)
        or not recovery_id.startswith("recovery_")
        or not 16 <= len(recovery_id) <= 128
        or not all(
            character.isalnum() or character in "_-"
            for character in recovery_id
        )
    ):
        raise AgentRequestError("recovery_id 非法")
    return {
        "kind": "restore-delete",
        "recovery_id": recovery_id,
    }


def validate_ops(ops) -> list[dict]:
    if not isinstance(ops, list) or not ops or len(ops) > 50:
        raise AgentRequestError("ops 必须是 1 到 50 项的数组")
    agent_tools._validate_json_shape(ops)
    ordinary = []
    normalized = []
    replaced_turns = []
    for operation in ops:
        if not isinstance(operation, dict):
            raise AgentRequestError("每个 edit op 必须是 object")
        if operation.get("op") != "replace-assistant-reply":
            ordinary.append(operation)
            normalized.append(operation)
            continue
        if set(operation) != {"op", "turn", "reply"}:
            raise AgentRequestError("replace-assistant-reply 参数非法")
        turn = operation["turn"]
        if (
            isinstance(turn, bool)
            or not isinstance(turn, (int, str))
            or (isinstance(turn, int) and turn < 1)
            or (isinstance(turn, str) and not 1 <= len(turn) <= 512)
        ):
            raise AgentRequestError("replace-assistant-reply turn 参数非法")
        reply = AssistantReply.from_dict(operation["reply"])
        turn_key = (type(turn).__name__, turn)
        if turn_key in replaced_turns:
            raise AgentRequestError(
                "同一轮次不能在一次编辑中重复替换",
                {"field": "ops.turn"},
            )
        replaced_turns.append(turn_key)
        normalized.append({
            "op": "replace-assistant-reply",
            "turn": turn,
            "reply": reply.to_dict(),
        })
    if ordinary:
        agent_tools._validate_ops(ordinary)
    return normalized
