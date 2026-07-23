"""格式无关的迁移能力基类与会话树装配。"""
from __future__ import annotations

import json
from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any

from ...domain.events import event
from ...domain.model import tool_result_text
from ...domain.tool_ops import has_valid_tool_input
from .narration import narrate

_DEGRADED_LOSS_CODES = {
    "migration.apply_patch_unparsed",
    "migration.fork_parent_fallback",
    "migration.reasoning_metadata_dropped",
    "session.unpaired_tool_use",
}
_DROPPED_LOSS_CODES = {
    "migration.children_not_migrated",
    "migration.reasoning_dropped",
    "migration.truncated",
    "migration.unknown_block_dropped",
    "session.child_foreign_ignored",
    "session.child_parent_conflict",
}


class Fidelity:
    EXACT = "exact"
    TRANSFORMED = "transformed"
    LOSSY = "lossy"
    NARRATED = "narrated"
    DROPPED = "dropped"

    VALUES = frozenset({EXACT, TRANSFORMED, LOSSY, NARRATED, DROPPED})


@dataclass(frozen=True)
class RenderDecision:
    """一次具体工具调用在目标端的唯一迁移判定。"""

    fidelity: str
    rendered: dict | None = None
    reason_codes: tuple[str, ...] = ()
    consumed_fields: frozenset[str] = field(default_factory=frozenset)
    ignored_fields: frozenset[str] = field(default_factory=frozenset)
    target_records: Any = None

    def __post_init__(self):
        if self.fidelity not in Fidelity.VALUES:
            raise ValueError(f"未知工具保真度: {self.fidelity}")
        if self.ignored_fields and not self.reason_codes:
            raise ValueError("忽略工具字段时必须给出 reason code")

    @property
    def outcome(self) -> str:
        if self.fidelity == Fidelity.EXACT:
            return "native"
        if self.fidelity == Fidelity.DROPPED:
            return "dropped"
        return "degraded"

    @property
    def reason_code(self) -> str | None:
        return self.reason_codes[0] if self.reason_codes else None

    def to_dict(self) -> dict:
        return {
            "fidelity": self.fidelity,
            "outcome": self.outcome,
            "rendered": self.rendered,
            "reason_codes": list(self.reason_codes),
            "reason_code": self.reason_code,
            "consumed_fields": sorted(self.consumed_fields),
            "ignored_fields": sorted(self.ignored_fields),
        }


def _loss_outcome(loss):
    code = loss.get("code") if isinstance(loss, dict) else None
    if code in _DEGRADED_LOSS_CODES:
        return "degraded"
    if code in _DROPPED_LOSS_CODES:
        return "dropped"
    return None


def _walk_meta(nodes):
    for node in nodes:
        yield node
        yield from _walk_meta(node.get("children", []))


def assemble_tree(browser, ref: str, cache):
    """读取会话并按 scanner 元数据装配整棵父子树。"""
    path = browser.resolve_ref(ref)
    session = browser.read(path)
    if session.children:
        return session
    roots = browser.scan(cache)
    target = next((node for node in _walk_meta(roots)
        if node["id"] == session.source_id or
        (node.get("path") and node["path"] == str(path))), None)
    if target is None:
        return session

    def attach(current, meta, root_id):
        current.source_id = meta["id"]
        current.root_id = root_id
        current.parent_id = meta.get("parent_id")
        current.title = current.title or meta.get("title", "")
        current.cwd = current.cwd or meta.get("dir", "")
        existing = {child.source_id: child for child in current.children}
        children = []
        for child_meta in meta.get("children", []):
            child = existing.get(child_meta["id"])
            if child is None:
                child = browser.read(child_meta.get("path") or child_meta["id"])
            attach(child, child_meta, root_id)
            children.append(child)
        current.children = children

    attach(session, target, target.get("root_id") or target["id"])
    return session


def linked_agent_edge(session, tool, message=None, *, allow_message=False):
    if tool.source_call_id:
        edge = next((edge for edge in session.agent_edges
                     if edge.source_call_id == tool.source_call_id), None)
        if edge:
            return edge
    if tool.agent_id:
        edge = next((edge for edge in session.agent_edges
                     if edge.agent_id == tool.agent_id or
                     edge.child_session_id == tool.agent_id), None)
        if edge:
            return edge
    if allow_message and message and message.source_id:
        matches = [edge for edge in session.agent_edges
                   if edge.spawn_message_id == message.source_id]
        if len(matches) == 1:
            return matches[0]
    return None


class TreeMigrationSource:
    """任何提供 browser 能力的插件都可以此作为迁移来源。"""

    def __init__(self, browser):
        self._browser = browser

    def export_tree(self, ref: str, cache=None):
        return assemble_tree(self._browser, ref, cache)


class MigrationTargetBase:
    """迁移目标基类：write 由子类实现，plan/classify 提供默认策略。"""

    tool: str
    tool_fidelity: Mapping[str, str] = {}
    tool_result_statuses = frozenset({"success", "error"})
    tool_result_native_blocks = frozenset({"text"})
    tool_result_projected_blocks = frozenset({"json"})
    preserves_tool_result_attachments = False
    preserves_tool_result_metadata = False

    def _with_result_fidelity(self, tool, decision: RenderDecision) -> RenderDecision:
        result = tool.result
        if result is None or decision.rendered is None:
            return decision
        if result.status == "unknown":
            return RenderDecision(
                Fidelity.NARRATED,
                reason_codes=("unknown_result_status",),
                ignored_fields=frozenset(tool.input)
                if isinstance(tool.input, dict) else frozenset(),
            )
        if result.status not in self.tool_result_statuses:
            return RenderDecision(
                Fidelity.NARRATED,
                reason_codes=("unsupported_result_status",),
                ignored_fields=frozenset(tool.input)
                if isinstance(tool.input, dict) else frozenset(),
            )
        reasons = list(decision.reason_codes)
        fidelity = decision.fidelity
        block_kinds = {block.kind for block in result.blocks}
        projected = block_kinds & self.tool_result_projected_blocks
        dropped = block_kinds - (
            self.tool_result_native_blocks | self.tool_result_projected_blocks
        )
        if projected:
            reasons.append("tool_result_block_degraded")
            if fidelity == Fidelity.EXACT:
                fidelity = Fidelity.TRANSFORMED
        if dropped:
            reasons.append("tool_result_block_dropped")
            if fidelity in {Fidelity.EXACT, Fidelity.TRANSFORMED}:
                fidelity = Fidelity.LOSSY
        if any(block.metadata for block in result.blocks):
            reasons.append("tool_result_block_metadata_dropped")
            if fidelity in {Fidelity.EXACT, Fidelity.TRANSFORMED}:
                fidelity = Fidelity.LOSSY
        if result.attachments and not self.preserves_tool_result_attachments:
            reasons.append("tool_result_attachments_dropped")
            if fidelity in {Fidelity.EXACT, Fidelity.TRANSFORMED}:
                fidelity = Fidelity.LOSSY
        if result.metadata and not self.preserves_tool_result_metadata:
            reasons.append("tool_result_metadata_dropped")
            if fidelity in {Fidelity.EXACT, Fidelity.TRANSFORMED}:
                fidelity = Fidelity.LOSSY
        if result.truncated is True:
            reasons.append("tool_result_truncated")
            if fidelity in {Fidelity.EXACT, Fidelity.TRANSFORMED}:
                fidelity = Fidelity.LOSSY
        if fidelity == decision.fidelity and not reasons:
            return decision
        return RenderDecision(
            fidelity=fidelity,
            rendered=decision.rendered,
            reason_codes=tuple(dict.fromkeys(reasons)),
            consumed_fields=decision.consumed_fields,
            ignored_fields=decision.ignored_fields,
            target_records=decision.target_records,
        )

    def classify_tool_call(self, tool_call) -> str:
        if not has_valid_tool_input(tool_call.op, tool_call.input):
            return "degrade"
        return self.tool_fidelity.get(tool_call.op, "degrade")

    def evaluate_tool(self, tool, session, message=None) -> RenderDecision:
        """返回 plan、preview 和 writer 共用的调用级判定。"""
        valid = has_valid_tool_input(tool.op, tool.input)
        if not valid:
            return RenderDecision(
                Fidelity.NARRATED,
                reason_codes=("invalid_tool_input",),
                ignored_fields=frozenset(tool.input) if isinstance(tool.input, dict)
                else frozenset(),
            )
        verdict = self.classify_tool_call(tool)
        rendered = self.preview_tool(tool, session, message)
        if rendered is None:
            fidelity = Fidelity.DROPPED if verdict == "drop" else Fidelity.NARRATED
            reason = "tool_unsupported" if fidelity == Fidelity.DROPPED \
                else "tool_to_history"
            return RenderDecision(
                fidelity, reason_codes=(reason,),
                ignored_fields=frozenset(tool.input) if isinstance(tool.input, dict)
                else frozenset(),
            )

        rendered = dict(rendered)
        conversion = rendered.pop("conversion", None)
        explicit_fidelity = rendered.pop("_fidelity", None)
        consumed = frozenset(rendered.pop("_consumed_fields", ()))
        ignored = frozenset(rendered.pop("_ignored_fields", ()))
        reasons = tuple(rendered.pop("_reason_codes", ()))
        if not consumed and isinstance(tool.input, dict):
            consumed = frozenset(tool.input) - ignored
        if explicit_fidelity:
            fidelity = explicit_fidelity
        elif conversion == "transformed" or verdict == "degrade":
            fidelity = Fidelity.TRANSFORMED
        elif ignored:
            fidelity = Fidelity.LOSSY
        else:
            fidelity = Fidelity.EXACT
        if fidelity != Fidelity.EXACT and not reasons:
            reasons = ("tool_transformed" if fidelity == Fidelity.TRANSFORMED
                       else "tool_fields_ignored" if fidelity == Fidelity.LOSSY
                       else "tool_to_history",)
        decision = RenderDecision(
            fidelity, rendered=rendered, reason_codes=reasons,
            consumed_fields=consumed, ignored_fields=ignored,
        )
        return self._with_result_fidelity(tool, decision)

    def _tool_decision(self, tool, session, message=None) -> dict:
        """兼容既有调用方；新代码应直接消费 RenderDecision。"""
        return self.evaluate_tool(tool, session, message).to_dict()

    def plan(self, session) -> dict:
        """预演统计原生映射/降级/丢弃，与 write 的分发逻辑一致。"""
        native = degrade = 0
        fidelity_counts = {value: 0 for value in Fidelity.VALUES}
        details = []
        dropped = []
        for node in session.walk():
            for loss in node.loss:
                outcome = _loss_outcome(loss)
                if outcome == "degraded":
                    degrade += 1
                    details.append(loss)
                elif outcome == "dropped":
                    dropped.append(loss)
            for message in node.messages:
                for block in message.blocks:
                    if block.kind == "text":
                        native += 1
                        fidelity_counts[Fidelity.EXACT] += 1
                    elif block.kind == "tool" and block.tool:
                        decision = self.evaluate_tool(block.tool, node, message)
                        fidelity_counts[decision.fidelity] += 1
                        if decision.outcome == "native":
                            native += 1
                        elif decision.outcome == "degraded":
                            degrade += 1
                            details.append(event("migration.tool_degraded",
                                                 tool_name=block.tool.name,
                                                 fidelity=decision.fidelity,
                                                 reason_codes=list(
                                                     decision.reason_codes),
                                                 ignored_fields=sorted(
                                                     decision.ignored_fields)))
                        else:
                            dropped.append(event("migration.tool_dropped",
                                                 tool_name=block.tool.name))
                    elif block.kind in {"image", "thinking"}:
                        fidelity_counts[Fidelity.DROPPED] += 1
                        dropped.append(event("migration.content_dropped",
                                             kind=block.kind))
        return {"native": native, "degrade": degrade, "drop": len(dropped),
                **fidelity_counts,
                "degrade_details": details, "drop_details": dropped}

    def preview_tool(self, tool, session, message=None):
        """返回目标端可见的工具块；None 表示会降级成历史叙述。"""
        if self.classify_tool_call(tool) != "native":
            return None
        output = tool_result_text(tool.result)
        return {"kind": "tool", "name": tool.name, "input": tool.input,
                "output": output, "conversion": "native"}

    def preview(self, session, cwd: str | None = None) -> dict:
        """构建写入前可展示的目标会话语义，不修改 session 或目标存储。"""
        differences = []

        def snapshot(value, kind: str, label: str) -> dict:
            if isinstance(value, str):
                text = value
            else:
                text = json.dumps(value, ensure_ascii=False, indent=2, default=str)
            compact = " ".join(text.split())
            limit = 2500
            return {"kind": kind, "label": label,
                    "summary": compact[:180] + ("…" if len(compact) > 180 else ""),
                    "detail": text[:limit] + ("\n…" if len(text) > limit else ""),
                    "truncated": len(text) > limit, "char_count": len(text)}

        def tool_source(tool) -> dict:
            payload = {
                "input": tool.input,
                "output": tool_result_text(tool.result),
            }
            return snapshot(payload, "tool", tool.name)

        def add_difference(*, diff_id, kind, reason_code, value, node_key,
                           node_path, message_key=None, message_index=None,
                           block_index=None, round_index=None, role=None,
                           source=None, target=None, anchor_id=None,
                           scope="block", raw_event=None, fidelity=None,
                           reason_codes=None, consumed_fields=None,
                           ignored_fields=None):
            differences.append({
                "id": diff_id, "kind": kind, "fidelity": fidelity,
                "reason_code": reason_code,
                "reason_codes": reason_codes or ([reason_code] if reason_code else []),
                "consumed_fields": consumed_fields or [],
                "ignored_fields": ignored_fields or [],
                "scope": scope, "node_key": node_key, "node_id": value.source_id,
                "node_title": value.title, "node_path": node_path,
                "round_index": round_index, "message_key": message_key,
                "message_index": message_index, "block_index": block_index,
                "role": role, "anchor_id": anchor_id,
                "source": source, "target": target, "event": raw_event,
            })

        def node(value, path="0", depth=0):
            node_key = f"n:{path}"
            messages = []
            node_differences = []
            visible_rounds = set()
            round_index = 0
            for message_index, message in enumerate(value.messages):
                if message.role == "user":
                    round_index += 1
                elif round_index == 0:
                    round_index = 1
                message_key = f"{node_key}/m:{message_index}"
                round_key = f"{node_key}/r:{round_index}"
                blocks = []
                for block_index, block in enumerate(message.blocks):
                    block_key = f"{message_key}/b:{block_index}"
                    if block.kind == "text" and block.text:
                        blocks.append({"key": block_key, "kind": "text", "text": block.text})
                    elif block.kind == "tool" and block.tool:
                        tool_decision = self.evaluate_tool(
                            block.tool, value, message)
                        decision = tool_decision.to_dict()
                        rendered = decision["rendered"]
                        if rendered is not None:
                            rendered = {key: item for key, item in rendered.items()
                                        if key != "conversion"}
                            rendered["key"] = block_key
                            blocks.append(rendered)
                        else:
                            rendered = {"key": block_key, "kind": "text",
                                        "text": narrate(block.tool)}
                            if decision["outcome"] == "degraded":
                                blocks.append(rendered)
                        if decision["outcome"] != "native":
                            target = None if decision["outcome"] == "dropped" else (
                                snapshot(rendered.get("text", {
                                    "name": rendered.get("name"),
                                    "input": rendered.get("input"),
                                    "output": rendered.get("output", ""),
                                }), rendered["kind"],
                                rendered.get("name") or "history"))
                            node_differences.append({
                                "diff_id": f"{block_key}/difference",
                                "kind": decision["outcome"],
                                "fidelity": decision["fidelity"],
                                "reason_code": decision["reason_code"],
                                "reason_codes": decision["reason_codes"],
                                "consumed_fields": decision["consumed_fields"],
                                "ignored_fields": decision["ignored_fields"],
                                "value": value, "node_key": node_key,
                                "node_path": path, "message_key": message_key,
                                "message_index": message_index,
                                "block_index": block_index,
                                "round_index": round_index, "role": message.role,
                                "source": tool_source(block.tool), "target": target,
                                "round_key": round_key,
                            })
                    elif block.kind in {"image", "thinking"}:
                        if block.kind == "thinking":
                            source = snapshot(block.text or "", "thinking", "thinking")
                            reason_code = "unsupported_thinking"
                        else:
                            image = block.image
                            metadata = {"id": image.id if image else "",
                                        "mime_type": image.mime_type if image else "",
                                        "filename": image.filename if image else None}
                            source = snapshot(metadata, "image",
                                              metadata["filename"] or metadata["mime_type"] or "image")
                            reason_code = "unsupported_image"
                        node_differences.append({
                            "diff_id": f"{block_key}/difference", "kind": "dropped",
                            "fidelity": Fidelity.DROPPED,
                            "reason_code": reason_code, "value": value,
                            "node_key": node_key, "node_path": path,
                            "message_key": message_key, "message_index": message_index,
                            "block_index": block_index, "round_index": round_index,
                            "role": message.role, "source": source, "target": None,
                            "round_key": round_key,
                        })
                if blocks:
                    visible_rounds.add(round_key)
                    messages.append({"key": message_key, "round_index": round_index,
                                     "role": message.role if message.role in {"user", "assistant"} else "user",
                                     "created_at": message.created_at, "blocks": blocks})
            for item in node_differences:
                round_key = item.pop("round_key")
                add_difference(**item,
                               anchor_id=round_key if round_key in visible_rounds else None)
            node_loss = [(loss, _loss_outcome(loss)) for loss in value.loss]
            for loss_index, (loss, outcome) in enumerate(node_loss):
                if outcome is None:
                    continue
                add_difference(
                    diff_id=f"{node_key}/loss:{loss_index}", kind=outcome,
                    fidelity=(Fidelity.NARRATED if outcome == "degraded"
                              else Fidelity.DROPPED),
                    reason_code=loss.get("code") or "source_loss", value=value,
                    node_key=node_key, node_path=path, scope="node",
                    source=snapshot(loss.get("params", {}), "event",
                                    loss.get("code") or "source loss"),
                    raw_event=loss)
            return {"id": value.source_id, "title": value.title, "cwd": value.cwd,
                    "key": node_key, "path": path, "agent_path": value.agent_path,
                    "depth": depth, "messages": messages,
                    "children": [node(child, f"{path}.{index}", depth + 1)
                                 for index, child in enumerate(value.children)]}

        root = node(session)
        degraded = sum(item["kind"] == "degraded" for item in differences)
        dropped = sum(item["kind"] == "dropped" for item in differences)
        fidelity_counts = {
            value: sum(item.get("fidelity") == value for item in differences)
            for value in Fidelity.VALUES
        }
        exact = sum(1 for node_value in session.walk()
                    for message in node_value.messages
                    for block in message.blocks if block.kind == "text")
        exact += sum(
            self.evaluate_tool(block.tool, node_value, message).fidelity ==
            Fidelity.EXACT
            for node_value in session.walk()
            for message in node_value.messages
            for block in message.blocks
            if block.kind == "tool" and block.tool
        )
        fidelity_counts[Fidelity.EXACT] = exact
        return {"schema_version": 3, "target_tool": self.tool,
                "root": root, "read_only": True,
                "differences": {"counts": {
                    "total": degraded + dropped,
                    "degraded": degraded, "dropped": dropped,
                    **fidelity_counts,
                }, "items": differences}}

    def write(self, session, cwd: str):
        raise NotImplementedError
