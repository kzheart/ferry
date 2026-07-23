"""引擎命令行接口。"""

from concurrent.futures import ThreadPoolExecutor
import json
import sys
import threading

from ..application import services
from ..contracts.engine_methods import PARALLEL_READ_METHOD_NAMES
from .rpc import rpc

MAX_PARALLEL_READS = 4


def _request_method(request: str) -> str | None:
    try:
        value = json.loads(request)
    except json.JSONDecodeError:
        return None
    return value.get("method") if isinstance(value, dict) else None


def serve(input_stream=None, output_stream=None, handler=rpc) -> None:
    """处理 JSONL RPC：安全的纯读方法有限并发，其余请求严格串行。"""
    input_stream = sys.stdin if input_stream is None else input_stream
    output_stream = sys.stdout if output_stream is None else output_stream
    output_lock = threading.Lock()

    def complete(request: str) -> None:
        response = json.dumps(handler(request), ensure_ascii=False)
        with output_lock:
            output_stream.write(response + "\n")
            output_stream.flush()

    serial = ThreadPoolExecutor(max_workers=1, thread_name_prefix="engine-serial")
    reads = ThreadPoolExecutor(
        max_workers=MAX_PARALLEL_READS, thread_name_prefix="engine-read"
    )
    futures = []
    try:
        for line in input_stream:
            request = line.strip()
            if not request:
                continue
            executor = (
                reads
                if _request_method(request) in PARALLEL_READ_METHOD_NAMES
                else serial
            )
            futures.append(executor.submit(complete, request))
        for future in futures:
            future.result()
    finally:
        reads.shutdown(wait=True)
        serial.shutdown(wait=True)


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
        serve()
        return
    if cmd == "health":
        result = services.health()
    elif cmd in ("version", "--version"):
        result = services.version()
    elif cmd == "scan":
        result = services.scan()
    elif cmd == "show":
        result = services.show(rest[0], rest[1])
    elif cmd == "history":
        result = services.history()
    elif cmd == "env":
        result = services.env()
    else:
        sys.exit(f"未知命令: {cmd}")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
