"""格式无关的生命周期基类：文件型会话的永久删除策略。"""
from __future__ import annotations

from pathlib import Path


class BaseLifecycle:
    """通用生命周期默认值；各 Agent 子类覆盖差异点。"""

    tool: str
    executable: str = ""        # 装配时由 adapter 从 manifest executables 注入

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

    def delete(self, adapter, ref: str) -> dict:
        raise NotImplementedError


class FileSessionLifecycle(BaseLifecycle):
    """文件型会话：永久删除会话文件及其归属产物，不留快照。"""

    def delete(self, adapter, ref: str) -> dict:
        doc = adapter.editor.load(ref)
        path = doc.handle if isinstance(doc.handle, Path) else \
            Path(adapter.browser.resolve_ref(ref))
        children = self._delete_children(doc, path)
        self._delete_sidecar(path)
        path.unlink()
        return {"ok": True, "children": children}

    def _delete_children(self, doc, path: Path) -> int:
        return 0

    def _delete_sidecar(self, path: Path) -> None:
        pass
