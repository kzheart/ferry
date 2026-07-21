"""跨平台 CLI 定位：PATH（shutil.which）优先，常见安装目录兜底。

macOS 上 GUI 启动的进程只继承 launchd 最小 PATH，Tauri 层已用 fix-path-env
恢复登录 shell PATH；此处兜底覆盖 shell 配置异常与非标准安装位置。
Windows 上 npm 装的 CLI 是 .cmd 垫片，CreateProcess 对裸命令名不查 PATHEXT，
必须先解析出完整路径再执行（shutil.which 会按 PATHEXT 命中 .cmd）。
"""

import os
import shutil
import subprocess
import sys
from functools import lru_cache
from pathlib import Path

_WINDOWS = sys.platform == "win32"

# Windows 下抑制子进程闪现控制台窗口；直接展开进 subprocess.run(**RUN_FLAGS)
RUN_FLAGS = {"creationflags": subprocess.CREATE_NO_WINDOW} if _WINDOWS else {}


def _fallback_dirs() -> list[Path]:
    home = Path.home()
    dirs = [home / ".local" / "bin", home / ".npm-global" / "bin",
            home / ".bun" / "bin", home / ".volta" / "bin",
            home / ".opencode" / "bin"]   # opencode 官方 install 脚本默认目录
    if _WINDOWS:
        appdata = os.environ.get("APPDATA")
        if appdata:
            dirs.append(Path(appdata) / "npm")
    else:
        dirs += [Path("/opt/homebrew/bin"), Path("/usr/local/bin")]
        nvm = home / ".nvm" / "versions" / "node"
        if nvm.is_dir():
            dirs += sorted((v / "bin" for v in nvm.iterdir()), reverse=True)
    return dirs


@lru_cache(maxsize=None)
def resolve(tool: str) -> str | None:
    """解析 CLI 绝对路径；PATH 未命中时扫描常见安装目录。找不到返回 None。"""
    found = shutil.which(tool)
    if found:
        return found
    for directory in _fallback_dirs():
        found = shutil.which(tool, path=str(directory))
        if found:
            # CLI 可能是 node 等运行时的垫片(#!/usr/bin/env node),同目录通常就有
            # 该运行时;把兜底目录挂进本进程 PATH,让它与后续子进程都能找到。
            os.environ["PATH"] = os.pathsep.join(
                [str(directory), os.environ.get("PATH", "")])
            return found
    return None


def argv(tool: str, *args: str) -> list[str]:
    """构造 subprocess 命令行；未解析到时保留裸名，报错语义与原先一致。"""
    return [resolve(tool) or tool, *args]
