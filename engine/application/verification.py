"""目标 CLI 运行时验收。"""

import json
import subprocess

PROBE_PROMPT = "Reply with exactly: PROBE_OK"


class ProbeTimeout(RuntimeError):
    pass


def _run(cmd, cwd=None, timeout=180, env=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                              timeout=timeout, env=env)
    except subprocess.TimeoutExpired as error:
        raise ProbeTimeout(f"探针超时: {' '.join(cmd)}") from error


def _clip(text, limit=8000):
    text = text or ""
    return text if len(text) <= limit else text[:limit] + f"\n…(截断,共 {len(text)} 字符)"


def _format_claude_error(out):
    parts = []
    if out.get("result"):
        parts.append(str(out["result"]).strip())
    reason = out.get("terminal_reason") or out.get("stop_reason")
    if reason:
        parts.append(f"terminal_reason={reason}")
    if out.get("api_error_status") is not None:
        parts.append(f"api_error_status={out['api_error_status']}")
    if out.get("session_id"):
        parts.append(f"session_id={out['session_id']}")
    if out.get("modelUsage"):
        parts.append(f"modelUsage={json.dumps(out['modelUsage'], ensure_ascii=False)}")
    return "\n".join(parts) or _clip(json.dumps(out, ensure_ascii=False, indent=2))


def probe_claude(sid, dirpath, model=None):
    cmd = ["claude", "-p", PROBE_PROMPT, "--resume", sid, "--output-format", "json"]
    if model:
        cmd += ["--model", model]
    result = _run(cmd, cwd=dirpath)
    raw, error = (result.stdout or "").strip(), (result.stderr or "").strip()
    if result.returncode != 0 and not raw:
        return False, _clip(error or f"claude 退出码 {result.returncode}")
    try:
        out = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return False, f"非 JSON 输出 (exit={result.returncode}):\n{_clip(raw or error)}"
    if out.get("is_error") or result.returncode != 0:
        return False, _format_claude_error(out)
    return True, _clip(str(out.get("result", "")), 500)


def probe_codex_in_env(sid, model=None, env=None):
    cmd = ["codex", "exec", "resume", sid, "--skip-git-repo-check"]
    if model:
        cmd += ["-m", model]
    result = _run(cmd + [PROBE_PROMPT], env=env)
    ok = result.returncode == 0
    detail = (result.stdout if ok else (result.stderr or result.stdout)) or ""
    return ok, _clip(detail or f"codex 退出码 {result.returncode}")


def probe_codex(sid, _dirpath, model=None):
    return probe_codex_in_env(sid, model=model)


def probe_opencode(sid, dirpath, model=None):
    cmd = ["opencode", "run", "-s", sid]
    if model:
        cmd += ["-m", model]
    if dirpath:
        cmd[2:2] = ["--dir", dirpath]
    result = _run(cmd + [PROBE_PROMPT], cwd=dirpath, timeout=360)
    ok = result.returncode == 0 and bool((result.stdout or "").strip())
    detail = (result.stdout if ok else (result.stderr or result.stdout)) or ""
    return ok, _clip(detail or f"opencode 退出码 {result.returncode}")


PROBES = {"claude": probe_claude, "codex": probe_codex, "opencode": probe_opencode}


def run_probe(tool, session_id, dirpath=None, model=None):
    try:
        verifier = PROBES[tool]
    except KeyError as error:
        raise ValueError(f"未知工具: {tool}") from error
    if tool == "claude" and not dirpath:
        raise ValueError("claude 探针必须提供 --dir(项目目录)")
    return verifier(session_id, dirpath, model)
