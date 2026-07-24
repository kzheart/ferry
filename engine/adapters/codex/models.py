"""Codex CLI 模型发现。"""

import json
import subprocess
from pathlib import Path

from ...system import executables


def discover():
    result = subprocess.run(executables.argv("codex", "debug", "models"),
                            capture_output=True, text=True, timeout=60,
                            **executables.RUN_FLAGS)
    if result.returncode != 0:
        raise RuntimeError((result.stderr or result.stdout or "codex debug models 失败")[:300])
    data = json.loads(result.stdout)
    rows = data.get("models") if isinstance(data, dict) else data
    if not isinstance(rows, list):
        raise RuntimeError("codex debug models 输出格式异常")
    models = []
    for row in rows:
        slug = row.get("slug") or row.get("id") if isinstance(row, dict) else None
        if not slug:
            continue
        visibility = row.get("visibility") or "list"
        label = row.get("display_name") or slug
        if visibility != "list":
            label = f"{label} ({visibility})"
        models.append({"id": str(slug), "label": str(label), "source": "cli"})
    default = None
    try:
        for line in (Path.home() / ".codex/config.toml").read_text().splitlines():
            text = line.strip()
            if text.startswith("model") and "=" in text and not text.startswith("model_"):
                default = text.split("=", 1)[1].strip().strip('"').strip("'") or None
                break
    except OSError:
        pass
    return models, "cli", default


def fallback():
    return [{"id": model, "label": model, "source": "fallback"}
            for model in ("gpt-5.4", "gpt-5.5", "o3")]
