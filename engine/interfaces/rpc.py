"""版本 1 RPC 契约与调度。"""

import contextlib
import json
import sys

from ..application import services

PROTOCOL = 1
RPC_METHODS = {
    "health": lambda p: services.health(),
    "version": lambda p: services.version(),
    "scan": lambda p: services.scan(),
    "env": lambda p: services.env(),
    "models": lambda p: services.list_models(p["tool"]),
    "history": lambda p: services.history(),
    "snapshots": lambda p: services.snapshots(),
    "show": lambda p: services.show(p["tool"], p["ref"]),
    "migrate": lambda p: services.migrate(p["src"], p["dst"], p["ref"],
        cwd=p.get("cwd"), dry_run=p.get("dry_run", False),
        probe=p.get("probe", False), max_turn=p.get("max_turn"),
        probe_model=p.get("probe_model") or None),
    "handoff": lambda p: services.handoff(p["src"], p["ref"], p["dst"], cwd=p.get("cwd")),
    "edit_capabilities": lambda p: services.edit_capabilities(p["tool"]),
    "edit_preview": lambda p: services.edit_preview(p["ref"], p["ops"], tool=p.get("tool", "claude")),
    "edit_apply": lambda p: services.edit_apply(p["ref"], p["ops"], probe=p.get("probe", False),
        save_as=p.get("save_as", False), tool=p.get("tool", "claude")),
    "snapshot_restore": lambda p: services.snapshot_restore(p["session"],
        run_probe_after=p.get("probe", False), tool=p.get("tool", "claude")),
    "snapshot_delete": lambda p: services.snapshot_delete(p["path"]),
    "session_delete": lambda p: services.session_delete(p["tool"], p["ref"]),
    "session_undelete": lambda p: services.session_undelete(p["snapshot"]),
    "session_snapshot": lambda p: services.session_snapshot(p["tool"], p["ref"]),
    "session_meta_list": lambda p: services.session_meta_list(),
    "session_meta_set": lambda p: services.session_meta_set(p["id"], p.get("patch") or {}),
}


def rpc(request: str) -> dict:
    req = json.loads(request)
    fn = RPC_METHODS.get(req.get("method"))
    if fn is None:
        return {"error": f"未知 method: {req.get('method')}"}
    try:
        with contextlib.redirect_stdout(sys.stderr):
            result = fn(req.get("params") or {})
        return {"ok": True, "result": result}
    except (SystemExit, Exception) as error:
        return {"ok": False, "error": str(error)[:500]}
