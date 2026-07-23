"""格式无关的生命周期基类：文件型会话的删除/快照/恢复策略。"""
from __future__ import annotations

import shutil
from pathlib import Path

from ...domain.errors import OperationUnsupportedError, SnapshotInvalidSourceError
from ...infrastructure.snapshots import snapshot_file


class BaseLifecycle:
    """通用生命周期默认值；各 Agent 子类覆盖差异点。"""

    tool: str
    executable: str = ""        # 装配时由 plugin 从 manifest executables 注入
    delete_undoable = False

    def resume_args(self, session_id: str) -> list[str]:
        raise NotImplementedError

    def resume_descriptor(self, session_id: str, cwd: str) -> dict:
        """终端启动描述符：executable 必须命中 manifest 白名单。"""
        args = self.resume_args(session_id)
        return {"tool": self.tool, "session_id": session_id, "cwd": cwd,
                "executable": self.executable, "args": args,
                "display_command": f"cd {cwd} && " +
                                   " ".join([self.executable, *args])}

    def cleanup(self, session_id: str, dest) -> None:
        raise NotImplementedError

    def validation_ref(self, _session_id: str, dest) -> str:
        return str(dest)

    def probe_cwd(self, cwd):
        """探针是否需要工作目录；默认需要。"""
        return cwd

    def delete(self, plugin, ref: str) -> dict:
        raise NotImplementedError

    def restore_delete(self, _snapshot, _meta: dict) -> dict:
        raise OperationUnsupportedError(self.tool, "undelete")


class FileSessionLifecycle(BaseLifecycle):
    """文件型会话：删除前落快照（回收站语义），可通过 undelete 撤销。"""

    delete_undoable = True

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

    def restore_delete(self, snapshot, meta: dict) -> dict:
        """Restore a file session and its adapter-owned sidecar/children."""
        source = meta.get("source")
        if not source or not Path(source).is_absolute():
            raise SnapshotInvalidSourceError("该快照没有可恢复的源路径",
                                             {"snapshot": str(snapshot)})
        target = Path(source)
        if target.exists():
            raise SnapshotInvalidSourceError("源会话仍存在,未覆盖",
                                             {"target": str(target)})
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(snapshot, target)

        sidecar = Path(snapshot).with_suffix("")
        if sidecar.is_dir():
            shutil.move(str(sidecar), str(target.with_suffix("")))

        restored = 1
        for child in meta.get("children", []):
            if not isinstance(child, dict):
                continue
            child_snap = Path(child.get("snapshot", ""))
            child_source = Path(child.get("source", ""))
            if child_snap.exists() and child_source.is_absolute() and not child_source.exists():
                child_source.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(child_snap, child_source)
                restored += 1
        return {"ok": True, "restored": restored, "target": str(target)}
