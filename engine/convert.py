#!/usr/bin/env python3
"""转换 CLI(MVP:claude → codex)。

用法:
    python3 -m engine.convert --from claude --to codex <会话JSONL路径或sessionId> [--cwd 目标目录]

输出新 session id 与 rollout 路径;随后用 harness/probe.py 验收。
"""
import argparse
import glob
import os
import sys

from . import reader_claude, writer_codex

READERS = {"claude": reader_claude.read}
WRITERS = {"codex": writer_codex.write}


def resolve_claude_path(ref: str) -> str:
    if os.path.exists(ref):
        return ref
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{ref}.jsonl"))
    if not hits:
        sys.exit(f"找不到 Claude 会话: {ref}")
    return hits[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", choices=READERS, required=True)
    ap.add_argument("--to", dest="dst", choices=WRITERS, required=True)
    ap.add_argument("ref", help="会话文件路径或 session id")
    ap.add_argument("--cwd", default=None, help="目标会话的工作目录(默认沿用源)")
    args = ap.parse_args()

    path = resolve_claude_path(args.ref)
    sess = READERS[args.src](path)
    print(f"读取 {args.src}: {sess.source_id}  消息数={len(sess.messages)}")
    sid, dest = WRITERS[args.dst](sess, cwd=args.cwd)
    print(f"写出 {args.dst}: {sid}\n  {dest}")
    if sess.loss:
        print("损耗报告:")
        for l in sess.loss:
            print(f"  - {l}")
    print(f"\n验收: python3 harness/probe.py {args.dst} {sid}")


if __name__ == "__main__":
    main()
