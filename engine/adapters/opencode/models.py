"""OpenCode CLI 模型发现。"""

import subprocess

from ...system import executables


def discover():
    result = subprocess.run(executables.argv("opencode", "models"),
                            capture_output=True, text=True, timeout=90,
                            **executables.RUN_FLAGS)
    if result.returncode != 0:
        raise RuntimeError((result.stderr or result.stdout or "opencode models 失败")[:300])
    models = []
    for line in (result.stdout or "").splitlines():
        model = line.strip()
        if not model or model.startswith(("┌", "│", "└")):
            continue
        if " " in model and "/" not in model:
            continue
        models.append({"id": model, "label": model, "source": "cli"})
    if not models:
        raise RuntimeError("opencode models 未返回任何模型")
    return models, "cli", None


def fallback():
    return []
