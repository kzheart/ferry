"""Tool-specific lifecycle capabilities used by application orchestration."""

import glob
import json
import os
import shutil
import subprocess
import tempfile
import uuid
from pathlib import Path

from .claude import editing as claude_edit
from .opencode import session as opencode_session
from ..infrastructure import probes


def cleanup_claude(sid, _dest):
    for hit in glob.glob(os.path.expanduser(f"~/.claude/projects/*/{sid}.jsonl")):
        Path(hit).unlink(missing_ok=True)
        shutil.rmtree(Path(hit).with_suffix(""), ignore_errors=True)


def cleanup_codex(sid, _dest):
    for hit in glob.glob(os.path.expanduser("~/.codex/sessions/*/*/*/rollout-*.jsonl")):
        try:
            records = (json.loads(line) for line in Path(hit).read_text().splitlines() if line.strip())
            meta = next((row.get("payload", {}) for row in records
                         if row.get("type") == "session_meta"), {})
            if meta.get("id") == sid or meta.get("session_id") == sid:
                Path(hit).unlink()
        except (OSError, json.JSONDecodeError):
            continue


def cleanup_opencode(sid, _dest):
    try:
        tree = opencode_session.read(sid)
        ids = [node.source_id for node in reversed(list(tree.walk()))]
    except Exception:
        ids = [sid]
    for session_id in ids:
        subprocess.run(["opencode", "session", "delete", session_id],
                       capture_output=True, text=True, timeout=30)


def probe_claude_edit(editor, _doc, result, model=None):
    path = Path(result["saved_as"])
    records = claude_edit.load(path)
    cwd = next((row.get("cwd") for row in records if row.get("cwd")), ".")
    shadow_id = str(uuid.uuid4())
    for row in records:
        if "sessionId" in row:
            row["sessionId"] = shadow_id
    shadow = path.with_name(f"{shadow_id}.jsonl")
    claude_edit.save(shadow, records)
    sidecar = path.with_suffix("")
    shadow_sidecar = shadow.with_suffix("")
    if sidecar.is_dir():
        shutil.copytree(sidecar, shadow_sidecar, dirs_exist_ok=True)
    try:
        ok, detail = probes.run_probe("claude", shadow_id, cwd, model)
        return ok, f"(影子副本 {shadow_id} 已探测并清理)\n{detail}"
    finally:
        shadow.unlink(missing_ok=True)
        shutil.rmtree(shadow_sidecar, ignore_errors=True)


def probe_codex_edit(_editor, _doc, result, model=None):
    with tempfile.TemporaryDirectory(prefix="ferry-codex-probe-") as tmp:
        codex_home = Path(tmp) / ".codex"
        sessions = codex_home / "sessions" / "probe" / "01" / "01"
        sessions.mkdir(parents=True)
        for raw in result.get("published_paths", [result["saved_as"]]):
            shutil.copy(raw, sessions / Path(raw).name)
        for name in ("auth.json", "config.toml"):
            source = Path.home() / ".codex" / name
            if source.exists():
                shutil.copy(source, codex_home / name)
        env = dict(os.environ)
        env["CODEX_HOME"] = str(codex_home)
        ok, detail = probes.probe_codex_in_env(result["session_id"], env=env, model=model)
        return ok, f"(临时 CODEX_HOME 完整树探测 {result['session_id']}，已清理)\n{detail}"


def probe_opencode_edit(editor, doc, _result, model=None):
    shadow = editor.save_copy(editor.load(doc.ref))
    try:
        cwd = doc.data.get("info", {}).get("directory") or "."
        ok, detail = probes.run_probe("opencode", shadow["session_id"], cwd, model)
        return ok, f"(影子副本 {shadow['session_id']} 已探测并清理)\n{detail}"
    finally:
        editor.discard(shadow)
