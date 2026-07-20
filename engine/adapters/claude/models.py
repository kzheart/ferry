"""Claude 模型发现。"""

import json
from pathlib import Path

ALIASES = [
    ("default", "默认(账号推荐)"), ("best", "best"),
    ("fable", "fable · Fable 5"), ("opus", "opus"),
    ("sonnet", "sonnet"), ("haiku", "haiku"),
    ("opus[1m]", "opus[1m]"), ("sonnet[1m]", "sonnet[1m]"),
    ("opusplan", "opusplan"),
]


def discover():
    models = [{"id": model, "label": label, "source": "alias"} for model, label in ALIASES]
    default = None
    for path in (Path.home() / ".claude/settings.json", Path.home() / ".claude/settings.local.json"):
        try:
            config = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(config, dict) and config.get("model"):
            default = str(config["model"])
            break
    try:
        cache = json.loads((Path.home() / ".claude.json").read_text()).get("additionalModelOptionsCache") or []
        for item in cache:
            if isinstance(item, dict) and item.get("value"):
                models.append({"id": str(item["value"]),
                    "label": str(item.get("label") or item["value"]), "source": "cache"})
    except (OSError, json.JSONDecodeError, TypeError):
        pass
    return models, "alias", default


def fallback():
    return [{"id": model, "label": label, "source": "fallback"} for model, label in ALIASES]
