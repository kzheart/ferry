"""OpenCode 会话验收探针：编辑后克隆影子副本探测并清理。"""
from __future__ import annotations

from ...system import executables, probes
from . import reader as opencode_reader
from . import writer as opencode_writer
from . import store as opencode_store


def _probe(session_id, cwd, model=None):
    command = executables.argv("opencode", "run", "-s", session_id)
    if model:
        command += ["-m", model]
    if cwd:
        command[2:2] = ["--dir", cwd]
    result = probes.run(command + [probes.PROBE_PROMPT], cwd=cwd, timeout=360)
    if result.returncode != 0:
        return probes.report("failed", "probe.process_failed",
                             {"tool": "opencode", "exit_code": result.returncode},
                             stdout=result.stdout, stderr=result.stderr)
    if not probes.response_matches(result.stdout):
        return probes.report("failed", "probe.unexpected_response",
                             {"tool": "opencode"}, stdout=result.stdout,
                             stderr=result.stderr)
    return probes.report("passed", stdout=result.stdout, stderr=result.stderr)


class OpenCodeVerifier:
    def probe(self, session_id, cwd, model=None):
        return _probe(session_id, cwd, model)

    def probe_edited(self, editor, doc, result, model=None):
        authored = editor.load(result["session_id"])
        tree = authored.tree
        cwd = doc.data.get("info", {}).get("directory") or "."
        shadow_id, _ = opencode_writer.write(
            tree,
            cwd=cwd,
            native_payloads={
                tree.source_id: opencode_writer._clone(authored.data),
            },
        )
        try:
            rep = _probe(shadow_id, cwd, model)
            rep["isolation"] = {"kind": "shadow_session",
                                "id": shadow_id, "cleaned": True}
            return rep
        finally:
            try:
                shadow = opencode_reader.read(shadow_id)
                ids = [
                    node.source_id
                    for node in reversed(list(shadow.walk()))
                ]
            except Exception:
                ids = [shadow_id]
            for session_id in ids:
                opencode_store.delete_session(session_id)
