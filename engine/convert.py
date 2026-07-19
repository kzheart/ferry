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

from . import reader_claude, reader_codex, rw_opencode, writer_claude, \
    writer_codex

READERS = {"claude": reader_claude.read, "codex": reader_codex.read,
           "opencode": rw_opencode.read}
WRITERS = {"codex": writer_codex.write, "claude": writer_claude.write,
           "opencode": rw_opencode.write}


def resolve_ref(src: str, ref: str) -> str:
    """会话 id → 可供 reader 使用的引用(文件路径;opencode 直接用 id)。"""
    if src == "opencode":
        return ref
    if os.path.exists(ref):
        return ref
    pattern = {"claude": f"~/.claude/projects/*/{ref}.jsonl",
               "codex": f"~/.codex/sessions/*/*/*/rollout-*-{ref}.jsonl"}[src]
    hits = glob.glob(os.path.expanduser(pattern))
    if not hits:
        sys.exit(f"找不到 {src} 会话: {ref}")
    return hits[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", choices=READERS, required=True)
    ap.add_argument("--to", dest="dst", choices=WRITERS, required=True)
    ap.add_argument("ref", help="会话文件路径或 session id")
    ap.add_argument("--cwd", default=None, help="目标会话的工作目录(默认沿用源)")
    args = ap.parse_args()
    if args.src == args.dst:
        sys.exit("源与目标相同")

    sess = READERS[args.src](resolve_ref(args.src, args.ref))
    print(f"读取 {args.src}: {sess.source_id}  消息数={len(sess.messages)}")
    sid, dest = WRITERS[args.dst](sess, cwd=args.cwd)
    print(f"写出 {args.dst}: {sid}\n  {dest}")
    if sess.loss:
        print("损耗报告:")
        for l in sess.loss:
            print(f"  - {l}")
    dir_arg = f" --dir {args.cwd or sess.cwd}" if args.dst != "codex" else ""
    print(f"\n验收: python3 harness/probe.py {args.dst} {sid}{dir_arg}")


if __name__ == "__main__":
    main()
