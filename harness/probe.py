#!/usr/bin/env python3
"""resume 探针:用目标 CLI 无头加载指定会话,判定"这个会话能否被原生 resume"。

这是写回/编辑操作的唯一验收标准(目标工具当裁判)。

用法:
    python3 harness/probe.py claude   <session_id> --dir <项目目录> [--model M]
    python3 harness/probe.py codex    <session_id> [--model M]
    python3 harness/probe.py opencode <session_id> --dir <会话目录> [--model M]

退出码 0 = 可 resume;非 0 = 不可(stderr 带原因)。
注意:探针会对传入的 session 真实 resume 并发送一条极小提示词(消耗一次
模型调用),会向该会话追加消息/产生派生会话。因此调用方必须传入可丢弃的
临时影子副本(见 engine/api.py 的隔离探针),不要直接传正式会话 id。
"""
import argparse
import json
import subprocess
import sys

PROBE_PROMPT = "Reply with exactly: PROBE_OK"


class ProbeTimeout(RuntimeError):
    pass


def _run(cmd, cwd=None, timeout=180, env=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True,
                              text=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired:
        raise ProbeTimeout(f"探针超时: {' '.join(cmd)}")


def _clip(text: str, limit: int = 8000) -> str:
    text = text or ""
    if len(text) <= limit:
        return text
    return text[:limit] + f"\n…(截断,共 {len(text)} 字符)"


def _format_claude_error(out: dict) -> str:
    """把 Claude JSON 结果整理成可读错误,保留关键字段全文。"""
    parts = []
    result = out.get("result")
    if result:
        parts.append(str(result).strip())
    reason = out.get("terminal_reason") or out.get("stop_reason")
    if reason:
        parts.append(f"terminal_reason={reason}")
    if out.get("api_error_status") is not None:
        parts.append(f"api_error_status={out.get('api_error_status')}")
    if out.get("session_id"):
        parts.append(f"session_id={out.get('session_id')}")
    usage = out.get("modelUsage") or {}
    if usage:
        parts.append(f"modelUsage={json.dumps(usage, ensure_ascii=False)}")
    # 若关键字段都空,退回完整 JSON
    if not parts:
        return _clip(json.dumps(out, ensure_ascii=False, indent=2))
    return "\n".join(parts)


def probe_claude(sid, dirpath, model=None):
    cmd = ["claude", "-p", PROBE_PROMPT, "--resume", sid,
           "--output-format", "json"]
    if model:
        cmd += ["--model", model]
    r = _run(cmd, cwd=dirpath)
    raw = (r.stdout or "").strip()
    err = (r.stderr or "").strip()
    if r.returncode != 0 and not raw:
        return False, _clip(err or f"claude 退出码 {r.returncode}")
    try:
        out = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        detail = raw or err
        return False, f"非 JSON 输出 (exit={r.returncode}):\n{_clip(detail)}"
    if out.get("is_error") or r.returncode != 0:
        return False, _format_claude_error(out)
    return True, _clip(str(out.get("result", "")), 500)


def probe_codex(sid, _dirpath, model=None):
    cmd = ["codex", "exec", "resume", sid, "--skip-git-repo-check"]
    if model:
        cmd += ["-m", model]
    cmd.append(PROBE_PROMPT)
    r = _run(cmd)
    ok = r.returncode == 0
    detail = (r.stdout if ok else (r.stderr or r.stdout)) or ""
    if not ok and r.returncode:
        detail = detail or f"codex 退出码 {r.returncode}"
    return ok, _clip(detail)


def probe_codex_in_env(sid, model=None, env=None):
    """在独立 CODEX_HOME 中运行 Codex 探针。"""
    cmd = ["codex", "exec", "resume", sid, "--skip-git-repo-check"]
    if model:
        cmd += ["-m", model]
    cmd.append(PROBE_PROMPT)
    r = _run(cmd, env=env)
    ok = r.returncode == 0
    detail = (r.stdout if ok else (r.stderr or r.stdout)) or ""
    return ok, _clip(detail or f"codex 退出码 {r.returncode}")


def probe_opencode(sid, dirpath, model=None):
    cmd = ["opencode", "run", "-s", sid]
    if model:
        cmd += ["-m", model]
    cmd.append(PROBE_PROMPT)
    if dirpath:
        cmd[2:2] = ["--dir", dirpath]
    r = _run(cmd, cwd=dirpath, timeout=360)
    ok = r.returncode == 0 and bool((r.stdout or "").strip())
    detail = (r.stdout if ok else (r.stderr or r.stdout)) or ""
    if not ok and not detail:
        detail = f"opencode 退出码 {r.returncode}"
    return ok, _clip(detail)


PROBES = {"claude": probe_claude, "codex": probe_codex,
          "opencode": probe_opencode}


def run_probe(tool: str, session_id: str, dirpath: str | None = None,
              model: str | None = None) -> tuple[bool, str]:
    """运行指定工具探针，供引擎进程内调用。"""
    if tool not in PROBES:
        raise ValueError(f"未知工具: {tool}")
    if tool == "claude" and not dirpath:
        raise ValueError("claude 探针必须提供 --dir(项目目录)")
    return PROBES[tool](session_id, dirpath, model)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("tool", choices=PROBES)
    ap.add_argument("session_id")
    ap.add_argument("--dir", default=None,
                    help="claude: 项目目录(必填); opencode: 会话目录")
    ap.add_argument("--model", default=None,
                    help="探针使用的模型(各 CLI 原生 --model/-m)")
    args = ap.parse_args(argv)
    if args.tool == "claude" and not args.dir:
        ap.error("claude 探针必须提供 --dir(项目目录)")
    try:
        ok, detail = run_probe(args.tool, args.session_id, args.dir, args.model)
    except ProbeTimeout as error:
        print(str(error), file=sys.stderr)
        return 3
    tag = "PASS" if ok else "FAIL"
    print(f"[{tag}] {args.tool} {args.session_id}\n{detail}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
