"""目标 CLI 的子进程探针实现。

返回结构化报告：status/code/params 承载业务判定；
stdout/stderr 是 opaque diagnostic，不翻译、不参与判定。
"""

import json
import subprocess

PROBE_PROMPT = "Reply with exactly: PROBE_OK"
_DIAG_LIMIT = 8000


class ProbeTimeout(RuntimeError):
    pass


def _run(cmd, cwd=None, timeout=180, env=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                              timeout=timeout, env=env)
    except subprocess.TimeoutExpired as error:
        raise ProbeTimeout(f"探针超时: {' '.join(cmd)}") from error


def report(status, code=None, params=None, stdout="", stderr=""):
    stdout, stderr = stdout or "", stderr or ""
    truncated = len(stdout) > _DIAG_LIMIT or len(stderr) > _DIAG_LIMIT
    return {"status": status, "code": code, "params": params or {},
            "diagnostic": {"stdout": stdout[:_DIAG_LIMIT],
                           "stderr": stderr[:_DIAG_LIMIT],
                           "truncated": truncated}}


def timeout_report(tool, error):
    return report("failed", "probe.timeout", {"tool": tool}, stderr=str(error))


def probe_claude(sid, dirpath, model=None):
    if not dirpath:
        raise ValueError("claude 探针必须提供 --dir(项目目录)")
    cmd = ["claude", "-p", PROBE_PROMPT, "--resume", sid, "--output-format", "json"]
    if model:
        cmd += ["--model", model]
    result = _run(cmd, cwd=dirpath)
    raw, error = (result.stdout or "").strip(), (result.stderr or "").strip()
    if result.returncode != 0 and not raw:
        return report("failed", "probe.process_failed",
                      {"tool": "claude", "exit_code": result.returncode},
                      stderr=error)
    try:
        out = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return report("failed", "probe.non_json_output",
                      {"tool": "claude", "exit_code": result.returncode},
                      stdout=raw, stderr=error)
    if out.get("is_error") or result.returncode != 0:
        params = {"tool": "claude", "exit_code": result.returncode}
        for key in ("terminal_reason", "stop_reason", "api_error_status",
                    "session_id"):
            if out.get(key) is not None:
                params[key] = out[key]
        return report("failed", "probe.process_failed", params,
                      stdout=raw, stderr=error)
    return report("passed", stdout=str(out.get("result", "")))


def probe_codex_in_env(sid, model=None, env=None):
    cmd = ["codex", "exec", "resume", sid, "--skip-git-repo-check"]
    if model:
        cmd += ["-m", model]
    result = _run(cmd + [PROBE_PROMPT], env=env)
    if result.returncode != 0:
        return report("failed", "probe.process_failed",
                      {"tool": "codex", "exit_code": result.returncode},
                      stdout=result.stdout, stderr=result.stderr)
    return report("passed", stdout=result.stdout, stderr=result.stderr)


def probe_codex(sid, _dirpath, model=None):
    return probe_codex_in_env(sid, model=model)


def probe_opencode(sid, dirpath, model=None):
    cmd = ["opencode", "run", "-s", sid]
    if model:
        cmd += ["-m", model]
    if dirpath:
        cmd[2:2] = ["--dir", dirpath]
    result = _run(cmd + [PROBE_PROMPT], cwd=dirpath, timeout=360)
    if result.returncode != 0 or not (result.stdout or "").strip():
        return report("failed", "probe.process_failed",
                      {"tool": "opencode", "exit_code": result.returncode},
                      stdout=result.stdout, stderr=result.stderr)
    return report("passed", stdout=result.stdout, stderr=result.stderr)
