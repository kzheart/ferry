"""Installed session-source executable inspection."""

import subprocess

from .ports import current
from ..infrastructure import executables


def inspect() -> dict:
    ports = current()
    out = {}
    for tool in ports.adapters():
        info = {"installed": False, "path": None, "broken": False}
        plugin = ports.adapter(tool)
        executable = plugin.manifest.executables[0]
        resolved = executables.resolve(executable)
        if resolved:
            info["path"] = resolved
            try:
                process = subprocess.run([resolved, "--version"],
                                         capture_output=True, text=True,
                                         timeout=20, **executables.RUN_FLAGS)
                info["installed"] = process.returncode == 0
                # 定位到了却跑不起来（如 Node 版本不达标），与"未安装"区分
                info["broken"] = process.returncode != 0
            except (FileNotFoundError, subprocess.TimeoutExpired):
                pass
        out[tool] = info
    return out
