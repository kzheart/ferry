"""引擎命令行接口。"""

import json
import sys

from ..application import services
from .rpc import rpc


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if not args:
        sys.exit("缺少命令")
    cmd, rest = args[0], args[1:]
    if cmd == "rpc":
        print(json.dumps(rpc(rest[0] if rest else sys.stdin.read()), ensure_ascii=False))
        return
    if cmd == "serve":
        # 常驻模式:stdin 每行一个 JSON 请求,stdout 每行一个 JSON 响应
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            print(json.dumps(rpc(line), ensure_ascii=False), flush=True)
        return
    if cmd == "health":
        result = services.health()
    elif cmd in ("version", "--version"):
        result = services.version()
    elif cmd == "scan":
        result = services.scan()
    elif cmd == "show":
        result = services.show(rest[0], rest[1])
    elif cmd == "migrate":
        result = services.migrate(rest[0], rest[1], rest[2],
            cwd=rest[rest.index("--cwd") + 1] if "--cwd" in rest else None,
            dry_run="--dry-run" in rest, probe="--probe" in rest)
    elif cmd == "history":
        result = services.history()
    elif cmd == "snapshots":
        result = services.snapshots()
    elif cmd == "env":
        result = services.env()
    else:
        sys.exit(f"未知命令: {cmd}")
    print(json.dumps(result, ensure_ascii=False, indent=2))
