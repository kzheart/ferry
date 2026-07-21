"""CLI probing primitives shared by adapter-owned verifiers.

返回结构化报告：status/code/params 承载业务判定；
stdout/stderr 是 opaque diagnostic，不翻译、不参与判定。
"""

import subprocess

from . import executables

PROBE_PROMPT = "Reply with exactly: PROBE_OK"
_DIAG_LIMIT = 8000


class ProbeTimeout(RuntimeError):
    pass


def run(cmd, cwd=None, timeout=180, env=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                              timeout=timeout, env=env,
                              **executables.RUN_FLAGS)
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
