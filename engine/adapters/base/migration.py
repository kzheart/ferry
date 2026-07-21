"""格式无关的迁移能力基类与会话树装配。"""
from __future__ import annotations

from collections.abc import Mapping

from ...domain.events import event
from ...domain.tool_ops import has_valid_tool_input


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

    def classify_tool_call(self, tool_call) -> str:
        if not has_valid_tool_input(tool_call.op, tool_call.input):
            return "degrade"
        return self.tool_fidelity.get(tool_call.op, "degrade")

    def plan(self, session) -> dict:
        """预演统计原生映射/降级/丢弃，与 write 的分发逻辑一致。"""
        native = degrade = 0
        details = []
        dropped = []
        for node in session.walk():
            # Tool fidelity is derived from the target writer below. Do not
            # reclassify writer-emitted degradation events as dropped losses.
            dropped.extend(loss for loss in node.loss
                           if loss.get("code") != "migration.tool_degraded")
            for message in node.messages:
                for block in message.blocks:
                    if block.kind == "text":
                        native += 1
                    elif block.kind == "tool":
                        verdict = self.classify_tool_call(block.tool)
                        if verdict == "native":
                            native += 1
                        elif verdict == "degrade":
                            degrade += 1
                            details.append(event("migration.tool_degraded",
                                                 tool_name=block.tool.name))
                        else:
                            dropped.append(event("migration.tool_dropped",
                                                 tool_name=block.tool.name))
        return {"native": native, "degrade": degrade, "drop": len(dropped),
                "degrade_details": details, "drop_details": dropped}

    def write(self, session, cwd: str):
        raise NotImplementedError
