"""引擎命令行入口。"""

from concurrent.futures import ThreadPoolExecutor
import json
import sys
import threading

from ..bootstrap import build_engine
from ..contracts.engine_methods import PARALLEL_READ_METHOD_NAMES
from .rpc import RpcDispatcher

MAX_PARALLEL_READS = 4


def _request_method(request: str) -> str | None:
    try:
        value = json.loads(request)
    except json.JSONDecodeError:
        return None
    return value.get("method") if isinstance(value, dict) else None


def serve(input_stream=None, output_stream=None, handler=None) -> None:
    """处理 JSONL RPC：安全的纯读方法有限并发，其余请求严格串行。"""
    input_stream = sys.stdin if input_stream is None else input_stream
    output_stream = sys.stdout if output_stream is None else output_stream
    if handler is None:
        raise ValueError("serve 必须使用进程范围的 RPC dispatcher")
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
    # 常驻引擎可能运行数天；已完成任务不能无限保留在列表中。
    # executor.shutdown(wait=True) 仍会在 EOF 时等待全部在途请求结束。
    futures = set()
    failures = []
    future_lock = threading.Lock()

    def release(future) -> None:
        try:
            future.result()
        except BaseException as error:
            with future_lock:
                failures.append(error)
        finally:
            with future_lock:
                futures.discard(future)

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
            future = executor.submit(complete, request)
            with future_lock:
                futures.add(future)
            future.add_done_callback(release)
    finally:
        reads.shutdown(wait=True)
        serial.shutdown(wait=True)
    if failures:
        raise failures[0]


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if not args:
        sys.exit("缺少命令")
    cmd, rest = args[0], args[1:]
    application = build_engine()
    try:
        dispatcher = RpcDispatcher(application)
        if cmd == "rpc":
            print(json.dumps(dispatcher.handle(rest[0] if rest else sys.stdin.read()), ensure_ascii=False))
            return
        if cmd == "serve":
            # 常驻模式:stdin 每行一个 JSON 请求,stdout 每行一个 JSON 响应
            serve(handler=dispatcher.handle)
            return
        if cmd == "health":
            result = application.health()
        elif cmd in ("version", "--version"):
            result = application.version()
        elif cmd == "scan":
            result = application.scan()
        elif cmd == "show":
            result = application.show_session(rest[0], rest[1])
        elif cmd == "history":
            result = application.migration_history()
        elif cmd == "env":
            result = application.environment()
        else:
            sys.exit(f"未知命令: {cmd}")
        print(json.dumps(result, ensure_ascii=False, indent=2))
    finally:
        application.close()


if __name__ == "__main__":
    main()
