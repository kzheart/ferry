"""Opt-in real-engine check. Never points an agent at the user's native stores.

FERRY_TEST_NATIVE_FORKS=1 pytest tests/test_native_fork_integration.py
"""

import os
import json
import pathlib
import subprocess
import time
import uuid
import shutil
import queue
import threading
import pytest

ROOT = pathlib.Path(__file__).resolve().parents[1]


@pytest.mark.skipif(
    os.environ.get("FERRY_TEST_NATIVE_FORKS") != "1",
    reason="requires local OpenCode and a built engine",
)
def test_opencode_native_fork_and_bounded_resume(tmp_path):
    root = tmp_path.resolve()
    cwd = root / "work"
    cwd.mkdir()
    env = {
        **os.environ,
        "HOME": str(root),
        "XDG_DATA_HOME": str(root / "data"),
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_STATE_HOME": str(root / "state"),
        "XDG_CACHE_HOME": str(root / "cache"),
        "FERRY_DATA_DIR": str(root / "ferry"),
        "FERRY_RUNTIME_DATA_DIR": str(root / "ferry"),
        "CLAUDE_CONFIG_DIR": str(root / "claude"),
        "CODEX_HOME": str(root / "codex"),
        "PI_CODING_AGENT_DIR": str(root / "pi"),
        "GROK_HOME": str(root / "grok"),
        "FERRY_CURSOR_DB": str(root / "none"),
    }
    exe = shutil.which("opencode")
    assert exe, "Install opencode for the opt-in native test"
    sid = "ses_fixturefork"
    now = int(time.time() * 1000)
    info = {
        "id": sid,
        "slug": "fixture",
        "version": "1.18.32",
        "projectID": "global",
        "directory": str(cwd),
        "title": "Fork fixture",
        "time": {"created": now, "updated": now},
    }
    messages = []
    for i, role in enumerate(["user", "assistant", "user", "assistant"]):
        mid = f"msg_fixture{i}"
        m = {
            "id": mid,
            "sessionID": sid,
            "role": role,
            "time": {"created": now + i, "completed": now + i},
        }
        if role == "user":
            m.update(agent="build", model={"providerID": "openai", "modelID": "gpt-5"})
        else:
            m.update(
                parentID=f"msg_fixture{i-1}",
                modelID="gpt-5",
                providerID="openai",
                mode="build",
                agent="build",
                path={"cwd": str(cwd), "root": str(cwd)},
                cost=0,
                tokens={
                    "input": 0,
                    "output": 0,
                    "reasoning": 0,
                    "cache": {"read": 0, "write": 0},
                },
                finish="stop",
            )
        messages.append(
            {
                "info": m,
                "parts": [
                    {
                        "id": f"prt_fixture{i}",
                        "messageID": mid,
                        "sessionID": sid,
                        "type": "text",
                        "text": "FUTURE_SENTINEL" if i >= 2 else "before",
                    }
                ],
            }
        )
    fixture = root / "import.json"
    fixture.write_text(json.dumps({"info": info, "messages": messages}))
    r = subprocess.run(
        [exe, "import", str(fixture)],
        env=env,
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert r.returncode == 0, r.stderr
    engine = str(ROOT / "crates/ferry-engine/target/debug/ferry-engine")
    assert pathlib.Path(engine).is_file(), "Build ferry-engine first"
    proc = subprocess.Popen(
        [engine, "serve"],
        env=env,
        cwd=cwd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=open(root / "engine.log", "w"),
        text=True,
    )
    responses = queue.Queue()
    threading.Thread(
        target=lambda: [responses.put(line) for line in proc.stdout], daemon=True
    ).start()

    def rpc(method, params={}):
        reqid = str(uuid.uuid4())
        proc.stdin.write(
            json.dumps(
                {
                    "protocol": "ferry-ipc/1",
                    "id": reqid,
                    "method": method,
                    "params": params,
                }
            )
            + "\n"
        )
        proc.stdin.flush()
        while True:
            line = responses.get(timeout=30)
            if not line:
                raise Exception(
                    "engine exited " + (root / "engine.log").read_text()[-1500:]
                )
            r = json.loads(line)
            if r.get("id") == reqid:
                if not r["ok"]:
                    raise Exception(r)
                return r["result"]

    try:
        scan = rpc("scan")
        rows = scan if isinstance(scan, list) else scan.get("sessions", [])
        row = next(r for r in rows if r.get("id") == sid)
        ref = row["ref"]
        data = rpc("show", {"tool": "opencode", "ref": ref})
        point = rpc(
            "branch_point",
            {
                "tool": "opencode",
                "ref": ref,
                "turn_locator": data["completed_turns"][0],
            },
        )
        bounded = rpc(
            "session_read",
            {
                "tool": "opencode",
                "ref": ref,
                "through": point["through"],
                "include_tool_outputs": True,
            },
        )
        assert "FUTURE_SENTINEL" not in json.dumps(bounded)
        result = rpc(
            "session_fork",
            {
                "tool": "opencode",
                "ref": ref,
                "through": point["through"],
                "request_id": "native-smoke",
            },
        )
        assert (
            rpc(
                "session_fork",
                {
                    "tool": "opencode",
                    "ref": ref,
                    "through": point["through"],
                    "request_id": "native-smoke",
                },
            )["id"]
            == result["id"]
        )
        child = rpc("show", {"tool": "opencode", "ref": result["ref"]})
        assert "FUTURE_SENTINEL" not in json.dumps(child)
        source = rpc("show", {"tool": "opencode", "ref": ref})
        assert "FUTURE_SENTINEL" in json.dumps(source)
        assert child["fork_origin"]["session_id"] == sid
        found = rpc(
            "session_read",
            {
                "tool": "opencode",
                "ref": ref,
                "through": point["through"],
                "terms": ["FUTURE_SENTINEL"],
                "include_tool_outputs": True,
            },
        )
        assert found["matches"] == []
    finally:
        proc.terminate()
        proc.wait(timeout=5)
