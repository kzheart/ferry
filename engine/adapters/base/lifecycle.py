"""格式无关的生命周期基类：文件型会话的删除/快照/恢复策略。"""
from __future__ import annotations

from pathlib import Path

from ...infrastructure.snapshots import snapshot_file


class BaseLifecycle:
    """通用生命周期默认值；各 Agent 子类覆盖差异点。"""

    tool: str
    executable: str = ""        # 装配时由 plugin 从 manifest executables 注入

    def resume_args(self, session_id: str) -> list[str]:
        raise NotImplementedError

    def handoff_args(self) -> list[str]:
        return []

    def resume_descriptor(self, session_id: str, cwd: str) -> dict:
        """终端启动描述符：executable 必须命中 manifest 白名单。"""
        args = self.resume_args(session_id)
        return {"tool": self.tool, "session_id": session_id, "cwd": cwd,
                "executable": self.executable, "args": args,
                "display_command": f"cd {cwd} && " +
                                   " ".join([self.executable, *args])}

    def handoff_descriptor(self, cwd: str, doc: str) -> dict:
        args = self.handoff_args()
        head = " ".join([self.executable, *args])
        return {"tool": self.tool, "cwd": cwd, "handoff_doc": doc,
                "executable": self.executable, "args": args,
                "display_command": f'cd {cwd} && {head} "$(cat {doc})"'}

    def cleanup(self, session_id: str, dest) -> None:
        raise NotImplementedError

    def validation_ref(self, _session_id: str, dest) -> str:
        return str(dest)

    def probe_cwd(self, cwd):
        """探针是否需要工作目录；默认需要。"""
        return cwd

    def delete(self, plugin, ref: str) -> dict:
        raise NotImplementedError


class FileSessionLifecycle(BaseLifecycle):
    """文件型会话：删除前落快照（回收站语义），可通过 undelete 撤销。"""

    def delete(self, plugin, ref: str) -> dict:
        doc = plugin.require("editor").load(ref)
        path = doc.handle if isinstance(doc.handle, Path) else \
            Path(plugin.browser.resolve_ref(ref))
        children = self._delete_children(doc, path)
        snap = snapshot_file(path, "snapshot.before_delete", self.tool,
                             {"children": children} if children else None)
        self._archive_sidecar(path, snap)
        path.unlink()
        return {"ok": True, "snapshot": str(snap), "undoable": True,
                "children": len(children)}

    def _delete_children(self, doc, path: Path) -> list[dict]:
        return []

    def _archive_sidecar(self, path: Path, snap: Path) -> None:
        pass
