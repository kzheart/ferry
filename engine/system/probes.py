"""CLI probing primitives shared by adapter-owned verifiers.

返回结构化报告：status/code/params 承载业务判定；
stdout/stderr 是 opaque diagnostic，不翻译、不参与判定。
"""

import os
import signal
import subprocess
import sys
from dataclasses import dataclass

from . import executables

PROBE_TOKEN = "PROBE_OK"
PROBE_PROMPT = (
    "Runtime validation only. Do not explain, use tools, or add formatting. "
    f"Your entire response must be exactly this single token: {PROBE_TOKEN}"
)
_DIAG_LIMIT = 8000
_AGENT_TEXT_LIMIT = 65536


class ProbeTimeout(RuntimeError):
    pass


@dataclass(frozen=True)
class AgentProcessResult:
    returncode: int | None
    stdout: str
    stderr: str
    timed_out: bool


def run(cmd, cwd=None, timeout=180, env=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                              timeout=timeout, env=env,
                              **executables.RUN_FLAGS)
    except subprocess.TimeoutExpired as error:
        raise ProbeTimeout(f"探针超时: {' '.join(cmd)}") from error


def _signal_process_group(process: subprocess.Popen, *, force: bool) -> None:
    if sys.platform == "win32":
        if process.poll() is not None:
            return
        if force:
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                capture_output=True,
                check=False,
                **executables.RUN_FLAGS,
            )
        else:
            process.send_signal(signal.CTRL_BREAK_EVENT)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
        except ProcessLookupError:
            pass


def run_agent_command(
    cmd: list[str],
    *,
    cwd: str | None = None,
    input_text: str | None = None,
    timeout: int = 360,
    env: dict[str, str] | None = None,
) -> AgentProcessResult:
    if (
        not isinstance(cmd, list)
        or not cmd
        or any(not isinstance(part, str) or not part for part in cmd)
    ):
        raise ValueError("Agent 命令必须是非空 argv")
    if isinstance(timeout, bool) or not isinstance(timeout, int) or not 1 <= timeout <= 360:
        raise ValueError("Agent timeout 必须在 1..360 秒")
    flags = dict(executables.RUN_FLAGS)
    if sys.platform == "win32":
        flags["creationflags"] = (
            flags.get("creationflags", 0) | subprocess.CREATE_NEW_PROCESS_GROUP
        )
    else:
        flags["start_new_session"] = True
    process = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdin=subprocess.PIPE if input_text is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
        **flags,
    )
    try:
        stdout, stderr = process.communicate(input=input_text, timeout=timeout)
        return AgentProcessResult(
            process.returncode, stdout or "", stderr or "", False,
        )
    except subprocess.TimeoutExpired:
        _signal_process_group(process, force=False)
        try:
            stdout, stderr = process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            _signal_process_group(process, force=True)
            stdout, stderr = process.communicate()
        return AgentProcessResult(
            process.returncode, stdout or "", stderr or "", True,
        )


def normalize_agent_text(value: str | None) -> tuple[str, bool]:
    text = value or ""
    return text[:_AGENT_TEXT_LIMIT], len(text) > _AGENT_TEXT_LIMIT


def report(status, code=None, params=None, stdout="", stderr=""):
    stdout, stderr = stdout or "", stderr or ""
    truncated = len(stdout) > _DIAG_LIMIT or len(stderr) > _DIAG_LIMIT
    return {"status": status, "code": code, "params": params or {},
            "diagnostic": {"stdout": stdout[:_DIAG_LIMIT],
                           "stderr": stderr[:_DIAG_LIMIT],
                           "truncated": truncated}}


def timeout_report(tool, error):
    return report("failed", "probe.timeout", {"tool": tool}, stderr=str(error))


def response_matches(stdout: str | None) -> bool:
    """A resumed agent passes only when it returns the probe token exactly."""
    return (stdout or "").strip() == PROBE_TOKEN
