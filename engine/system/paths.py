"""Cross-platform locations for external session stores."""
from __future__ import annotations

import os
import json
from collections.abc import Mapping
from pathlib import Path


def is_within(path: str, root: str) -> bool:
    """等价于 Path(path).is_relative_to(root)，按已规范化的路径串比较。"""
    if path == root:
        return True
    prefix = root if root.endswith(os.sep) else root + os.sep
    return path.startswith(prefix)


def opencode_database_path(
    *,
    platform: str | None = None,
    environ: Mapping[str, str] | None = None,
    home: Path | None = None,
) -> Path:
    env = os.environ if environ is None else environ
    override = env.get("FERRY_OPENCODE_DB")
    if override:
        return Path(override).expanduser()

    user_home = Path.home() if home is None else home
    current_platform = os.name if platform is None else platform
    if current_platform == "nt":
        data_home = Path(
            env.get("LOCALAPPDATA", user_home / "AppData" / "Local")
        )
    else:
        data_home = Path(
            env.get("XDG_DATA_HOME", user_home / ".local" / "share")
        )
    return data_home / "opencode" / "opencode.db"


def pi_session_roots(
    *,
    environ: Mapping[str, str] | None = None,
    home: Path | None = None,
) -> tuple[Path, ...]:
    """Return Pi session roots in runtime lookup order.

    Pi's explicit session directory wins.  Otherwise a configured sessionDir
    is followed by the default project-bucket root so scanning can discover
    every cwd without trying to reverse Pi's encoded directory names.
    """
    env = os.environ if environ is None else environ
    explicit = env.get("PI_CODING_AGENT_SESSION_DIR")
    if explicit:
        return (Path(explicit).expanduser(),)

    user_home = Path.home() if home is None else home
    agent_dir = Path(
        env.get("PI_CODING_AGENT_DIR", user_home / ".pi" / "agent")
    ).expanduser()
    settings_path = agent_dir / "settings.json"
    try:
        settings = json.loads(settings_path.read_text())
    except (OSError, ValueError, TypeError):
        settings = {}
    configured = settings.get("sessionDir") if isinstance(settings, dict) else None
    if isinstance(configured, str) and configured.strip():
        return (Path(configured).expanduser(),)
    return (agent_dir / "sessions",)


def grok_home(
    *,
    environ: Mapping[str, str] | None = None,
    home: Path | None = None,
) -> Path:
    env = os.environ if environ is None else environ
    user_home = Path.home() if home is None else home
    return Path(env.get("GROK_HOME", user_home / ".grok")).expanduser()
