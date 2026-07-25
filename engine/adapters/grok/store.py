"""Safe loading and fingerprinting for Grok session bundles."""
from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

from ...errors import AgentFormatChangedError, SessionNotFoundError


@dataclass
class GrokBundle:
    path: Path
    summary: dict
    updates: list[dict]
    chat: list[dict]
    diagnostics: list[dict]

    @property
    def authoritative_members(self):
        members = [self.path / "summary.json"]
        updates = self.path / "updates.jsonl"
        members.append(updates if updates.is_file()
                       else self.path / "chat_history.jsonl")
        return tuple(members)


def _jsonl(path: Path) -> tuple[list[dict], list[dict]]:
    if not path.is_file():
        return [], []
    lines = path.read_text().splitlines()
    nonempty = [i for i, line in enumerate(lines) if line.strip()]
    final = nonempty[-1] if nonempty else -1
    records, diagnostics = [], []
    for index, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            if index != final:
                diagnostics.append({"line": index + 1, "reason": "invalid_json"})
            continue
        if isinstance(value, dict):
            records.append(value)
        else:
            diagnostics.append({"line": index + 1, "reason": "non_object"})
    return records, diagnostics


def load_grok_bundle(path: Path) -> GrokBundle:
    try:
        root = path.resolve(strict=True)
    except OSError as error:
        raise SessionNotFoundError("grok", str(path)) from error
    summary_path = root / "summary.json"
    try:
        summary = json.loads(summary_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise AgentFormatChangedError(
            "grok", "summary.json", "current summary object", None,
        ) from error
    info = summary.get("info") if isinstance(summary, dict) else None
    if (
        not isinstance(info, dict)
        or not isinstance(info.get("id"), str)
        or not isinstance(info.get("cwd"), str)
        or summary.get("chat_format_version") != 1
    ):
        raise AgentFormatChangedError(
            "grok", "summary.json",
            {"chat_format_version": 1, "info": {"id": "str", "cwd": "str"}},
            summary,
        )
    updates, update_diag = _jsonl(root / "updates.jsonl")
    chat, chat_diag = _jsonl(root / "chat_history.jsonl")
    if not updates and not chat:
        raise AgentFormatChangedError(
            "grok", "history", "updates.jsonl or chat_history.jsonl", None,
        )
    return GrokBundle(root, summary, updates, chat, update_diag + chat_diag)


def fingerprint(path: Path) -> str:
    bundle = load_grok_bundle(path)
    digest = hashlib.sha256()
    for member in bundle.authoritative_members:
        digest.update(member.name.encode())
        digest.update(b"\0")
        digest.update(member.read_bytes())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()
