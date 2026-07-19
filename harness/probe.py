#!/usr/bin/env python3
"""resume 探针:用目标 CLI 无头加载指定会话,判定"这个会话能否被原生 resume"。

这是写回/编辑操作的唯一验收标准(目标工具当裁判)。

用法:
    python3 harness/probe.py claude   <session_id> --dir <项目目录>
    python3 harness/probe.py codex    <session_id>
    python3 harness/probe.py opencode <session_id> --dir <会话目录>

退出码 0 = 可 resume;非 0 = 不可(stderr 带原因)。
注意:探针会真实发送一条极小提示词(消耗一次模型调用),并可能在目标工具中
产生一个派生会话;它不修改被探测的原会话内容本身。
"""
import argparse
import json
import subprocess
import sys

PROBE_PROMPT = "Reply with exactly: PROBE_OK"


def _run(cmd, cwd=None, timeout=180):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True,
                              text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        print(f"探针超时: {' '.join(cmd)}", file=sys.stderr)
        sys.exit(3)


def probe_claude(sid, dirpath):
    r = _run(["claude", "-p", PROBE_PROMPT, "--resume", sid,
              "--output-format", "json"], cwd=dirpath)
    if r.returncode != 0:
        return False, r.stderr[-500:] or r.stdout[-500:]
    try:
        out = json.loads(r.stdout)
    except json.JSONDecodeError:
        return False, f"非 JSON 输出: {r.stdout[-300:]}"
    if out.get("is_error"):
        return False, str(out)[:500]
    return True, out.get("result", "")[:200]


def probe_codex(sid, _dirpath):
    r = _run(["codex", "exec", "resume", sid, "--skip-git-repo-check",
              PROBE_PROMPT])
    ok = r.returncode == 0
    return ok, (r.stdout if ok else r.stderr)[-500:]


def probe_opencode(sid, dirpath):
    cmd = ["opencode", "run", "-s", sid, PROBE_PROMPT]
    if dirpath:
        cmd[2:2] = ["--dir", dirpath]
    # server 冷启动可能很慢,超时放宽
    r = _run(cmd, cwd=dirpath, timeout=360)
    ok = r.returncode == 0 and bool(r.stdout.strip())
    return ok, (r.stdout if ok else r.stderr)[-500:]


PROBES = {"claude": probe_claude, "codex": probe_codex,
          "opencode": probe_opencode}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("tool", choices=PROBES)
    ap.add_argument("session_id")
    ap.add_argument("--dir", default=None,
                    help="claude: 项目目录(必填); opencode: 会话目录")
    args = ap.parse_args()
    if args.tool == "claude" and not args.dir:
        ap.error("claude 探针必须提供 --dir(项目目录)")
    ok, detail = PROBES[args.tool](args.session_id, args.dir)
    tag = "PASS" if ok else "FAIL"
    print(f"[{tag}] {args.tool} {args.session_id}\n{detail}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
