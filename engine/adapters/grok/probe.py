"""Validate Grok bundles with official export/list/search commands."""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import quote

from ...system import executables, probes


def probe_bundle(bundle: Path):
    from .writer import index_bundle

    summary = json.loads((bundle / "summary.json").read_text())
    sid, cwd = summary["info"]["id"], summary["info"]["cwd"]
    title = str(
        summary.get("generated_title")
        or summary.get("session_summary")
        or sid
    )
    sentinel = next(
        (token for token in re.findall(r"[\w-]+", title)
         if len(token) >= 3),
        "Migrated",
    )
    with tempfile.TemporaryDirectory() as directory:
        home = Path(directory)
        sessions = home / "sessions"
        command_cwd = Path(cwd)
        if not command_cwd.is_dir():
            command_cwd = home / "probe-cwd"
            command_cwd.mkdir()
        target = sessions / quote(
            str(command_cwd.resolve()), safe="",
        ) / sid
        target.parent.mkdir(parents=True)
        shutil.copytree(bundle, target)
        if str(command_cwd) != cwd:
            shadow_summary = json.loads(
                (target / "summary.json").read_text()
            )
            shadow_summary["info"]["cwd"] = str(command_cwd)
            (target / "summary.json").write_text(json.dumps(
                shadow_summary, ensure_ascii=False,
            ))
        index_bundle(target, sessions)
        env = os.environ.copy()
        env["GROK_HOME"] = str(home)
        commands = (
            executables.argv("grok", "export", sid, str(home / "export.md")),
            executables.argv("grok", "sessions", "list"),
            executables.argv("grok", "sessions", "search", sentinel),
        )
        outputs = []
        for command in commands:
            try:
                result = subprocess.run(
                    command, capture_output=True, text=True, timeout=30,
                    env=env, cwd=str(command_cwd), **executables.RUN_FLAGS,
                )
            except subprocess.TimeoutExpired as error:
                return probes.timeout_report("grok", error)
            outputs.append((result.stdout or "") + (result.stderr or ""))
            if result.returncode:
                return probes.report(
                    "failed", "probe.process_failed",
                    {"tool": "grok", "exit_code": result.returncode},
                    stdout=result.stdout, stderr=result.stderr,
                )
        exported = home / "export.md"
        if (
            not exported.is_file()
            or sid not in outputs[1]
            or sid not in outputs[2]
        ):
            return probes.report(
                "failed", "probe.unexpected_response", {"tool": "grok"},
                stdout="\n".join(outputs),
            )
        return probes.report("passed", stdout="\n".join(outputs))


class GrokVerifier:
    def probe(self, session_id, cwd, model=None):
        from .adapter import resolve

        return probe_bundle(resolve(session_id))

    def probe_edited(self, editor, doc, result, model=None):
        return probe_bundle(Path(result["saved_as"]))
