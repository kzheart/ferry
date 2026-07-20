#!/usr/bin/env python3
"""外部验证器 CLI；生产实现由 engine.application.verification 所有。"""

import argparse
import sys

from engine.application.verification import (
    PROBES, ProbeTimeout, _clip, _format_claude_error, _run, probe_claude,
    probe_codex, probe_codex_in_env, probe_opencode, run_probe,
)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("tool", choices=PROBES)
    parser.add_argument("session_id")
    parser.add_argument("--dir", default=None)
    parser.add_argument("--model", default=None)
    args = parser.parse_args(argv)
    if args.tool == "claude" and not args.dir:
        parser.error("claude 探针必须提供 --dir(项目目录)")
    try:
        ok, detail = run_probe(args.tool, args.session_id, args.dir, args.model)
    except ProbeTimeout as error:
        print(str(error), file=sys.stderr)
        return 3
    print(f"[{'PASS' if ok else 'FAIL'}] {args.tool} {args.session_id}\n{detail}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
