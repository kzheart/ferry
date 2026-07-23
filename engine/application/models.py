"""模型发现与用户扩展合并用例。"""

import json
from pathlib import Path

from .ports import ApplicationPorts

MODELS_CONFIG = Path.home() / ".resume-harness/models.json"


def _user_model_ids(tool):
    try:
        data = json.loads(MODELS_CONFIG.read_text())
    except (OSError, json.JSONDecodeError):
        return []
    raw = data.get(tool) if isinstance(data, dict) else None
    if not isinstance(raw, list):
        return []
    return [item.strip() if isinstance(item, str) else str(item["id"]).strip()
            for item in raw if (isinstance(item, str) and item.strip())
            or (isinstance(item, dict) and item.get("id"))]


def list_models(tool_name: str, ports: ApplicationPorts):
    catalog = ports.adapter(tool_name).models
    error = default = None
    try:
        rows, source, default = catalog.discover()
    except Exception as exc:
        error, source, rows = str(exc)[:400], "fallback", catalog.fallback()
    rows += [{"id": model, "label": model, "source": "user"} for model in _user_model_ids(tool_name)]
    seen, models = set(), []
    for row in rows:
        if row.get("id") and row["id"] not in seen:
            seen.add(row["id"])
            models.append(row)
    return {"tool": tool_name, "default": default, "models": models,
        "source": source, "error": error, "allow_custom": True,
        "config_path": str(MODELS_CONFIG)}
