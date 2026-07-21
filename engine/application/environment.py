"""Installed tool and golden fixture inspection."""

import re
import subprocess

from .ports import current
from ..infrastructure import executables


def inspect() -> dict:
    ports = current()
    out = {}
    for tool in ports.adapters():
        info = {"installed": False, "version": None, "path": None,
                "broken": False, "golden": None, "verified": False}
        executable = ports.adapter(tool).manifest.executables[0]
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
        golden = ports.resource_path("golden", tool)
        if golden.exists():
            versions = sorted(path.name for path in golden.iterdir() if path.is_dir())
            info["golden"] = versions[-1] if versions else None
        info["verified"] = bool(info["installed"] and info["golden"]
                                and info["version"] == info["golden"])
        out[tool] = info
    return out
