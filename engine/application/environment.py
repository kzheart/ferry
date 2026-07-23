"""Installed tool and native-format profile inspection."""

import re
import subprocess

from .ports import current
from ..infrastructure import executables


def inspect() -> dict:
    ports = current()
    out = {}
    for tool in ports.adapters():
        info = {"installed": False, "version": None, "path": None,
                "broken": False, "format": None, "verified": False}
        plugin = ports.adapter(tool)
        executable = plugin.manifest.executables[0]
        resolved = executables.resolve(executable)
        if resolved:
            info["path"] = resolved
            try:
                process = subprocess.run([resolved, "--version"],
                                         capture_output=True, text=True,
                                         timeout=20, **executables.RUN_FLAGS)
                match = re.search(r"\d+\.\d+\.\d+",
                                  process.stdout + process.stderr)
                info["installed"] = process.returncode == 0
                # 定位到了却跑不起来（如 Node 版本不达标），与"未安装"区分
                info["broken"] = process.returncode != 0
                info["version"] = match.group(0) if match else None
            except (FileNotFoundError, subprocess.TimeoutExpired):
                pass
        if plugin.formats is not None:
            info["format"] = plugin.formats.inspect(info["version"])
            info["verified"] = info["format"]["status"] == "verified"
        out[tool] = info
    return out
