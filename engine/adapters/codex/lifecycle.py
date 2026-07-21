"""Codex 会话生命周期：清理、删除（含原生子会话树）策略。"""
from __future__ import annotations

import json
from pathlib import Path

from ...infrastructure.snapshots import snapshot_file
from ..base.lifecycle import FileSessionLifecycle
from .native import CodexStore
from .registry import unregister_tree


class CodexLifecycle(FileSessionLifecycle):
    tool = "codex"

    def resume_args(self, session_id):
        return ["resume", session_id]

    def cleanup(self, session_id, dest):
        store = CodexStore.for_rollout(Path(dest))
        owned_ids = set()
        owned_paths = []
        for hit in store.sessions_dir.glob("*/*/*/rollout-*.jsonl"):
            try:
                records = (json.loads(line) for line in
                           hit.read_text().splitlines() if line.strip())
                meta = next((row.get("payload", {}) for row in records
                             if row.get("type") == "session_meta"), {})
                if meta.get("id") == session_id or meta.get("session_id") == session_id:
                    if meta.get("id"):
                        owned_ids.add(str(meta["id"]))
                    owned_paths.append(hit)
            except (OSError, json.JSONDecodeError):
                continue
        unregister_tree(store.state_db, owned_ids)
        for path in owned_paths:
            path.unlink(missing_ok=True)

    def probe_cwd(self, cwd):
        # codex resume 从 rollout 记录恢复工作目录，探针不额外传 cwd。
        return None

    def _delete_children(self, doc, path: Path) -> list[dict]:
        children = []
        closure = getattr(doc, "context", None)
        if closure is not None and hasattr(closure, "nodes"):
            for node in closure.nodes.values():
                child = Path(node.path)
                if child != path and child.exists():
                    child_snap = snapshot_file(child, "snapshot.before_delete", self.tool)
                    children.append({"snapshot": str(child_snap),
                                     "source": str(child)})
                    child.unlink()
        return children
