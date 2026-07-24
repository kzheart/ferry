"""Cross-platform locations for external session stores."""
from __future__ import annotations

import os
from collections.abc import Mapping
from pathlib import Path


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
