"""Pi model catalog without reading credentials."""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
import shutil
from pathlib import Path

from ...system import executables


def _agent_dir():
    return Path(os.environ.get(
        "PI_CODING_AGENT_DIR", Path.home() / ".pi" / "agent",
    )).expanduser()


def discover():
    rows, default = [], None
    try:
        settings = json.loads((_agent_dir() / "settings.json").read_text())
    except (OSError, ValueError, TypeError):
        settings = {}
    if isinstance(settings, dict):
        provider = settings.get("defaultProvider")
        model = settings.get("defaultModel")
        if model:
            default = f"{provider}/{model}" if provider else str(model)
    try:
        custom = json.loads((_agent_dir() / "models.json").read_text())
    except (OSError, ValueError, TypeError):
        custom = {}
    providers = custom.get("providers") if isinstance(custom, dict) else {}
    if isinstance(providers, dict):
        for provider_id, provider in providers.items():
            models = provider.get("models") if isinstance(provider, dict) else None
            if not isinstance(models, list):
                continue
            for model in models:
                if isinstance(model, dict) and model.get("id"):
                    model_id = f"{provider_id}/{model['id']}"
                    rows.append({
                        "id": model_id,
                        "label": str(model.get("name") or model_id),
                        "source": "models.json",
                    })
    try:
        with tempfile.TemporaryDirectory() as config_dir:
            source_settings = _agent_dir() / "settings.json"
            if source_settings.is_file():
                shutil.copy(source_settings, Path(config_dir) / "settings.json")
            env = os.environ.copy()
            env.update({
                "PI_CODING_AGENT_DIR": config_dir, "PI_OFFLINE": "1",
                "PI_SKIP_VERSION_CHECK": "1", "PI_TELEMETRY": "0",
            })
            result = subprocess.run(
                executables.argv(
                    "pi", "--list-models", "--offline", "--no-extensions",
                    "--no-skills", "--no-prompt-templates", "--no-themes",
                    "--no-context-files", "--no-approve",
                ),
                capture_output=True, text=True, timeout=10, env=env,
                **executables.RUN_FLAGS,
            )
        for line in result.stdout.splitlines():
            columns = line.split()
            if len(columns) >= 2 and columns[0] not in {"provider", "Provider"}:
                model_id = f"{columns[0]}/{columns[1]}"
                rows.append({
                    "id": model_id, "label": line.strip(), "source": "cli",
                })
    except (OSError, subprocess.TimeoutExpired):
        pass
    if default and all(row["id"] != default for row in rows):
        rows.append({"id": default, "label": default, "source": "settings"})
    return rows, "cli" if rows else "settings", default


def fallback():
    return []
