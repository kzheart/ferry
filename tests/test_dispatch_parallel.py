"""RPC 分流:纯读方法走并行读池,写方法与有副作用的方法仍严格串行。"""

import io
import json
import threading

from engine.contracts.engine_methods import (
    ENGINE_METHOD_POLICIES,
    PARALLEL_READ_METHOD_NAMES,
)
from engine.server.cli import serve
from engine.server.rpc import PROTOCOL


def _pool_of(methods: list[str]) -> dict[str, str]:
    """跑一遍 serve,记录每个方法实际落在哪个线程池。"""
    seen: dict[str, str] = {}

    def handler(request: str) -> dict:
        value = json.loads(request)
        seen[value["method"]] = threading.current_thread().name
        return {"protocol": PROTOCOL, "id": value["id"], "ok": True, "result": None}

    requests = "".join(
        f'{{"protocol":"{PROTOCOL}","id":"{index}",'
        f'"method":"{method}","params":{{}}}}\n'
        for index, method in enumerate(methods)
    )
    serve(io.StringIO(requests), io.StringIO(), handler)
    return {method: name.split("_")[0] for method, name in seen.items()}


def test_session_reads_run_in_the_parallel_pool():
    pools = _pool_of(["show", "session_asset", "session_meta_list"])
    assert pools == {
        "show": "engine-read",
        "session_asset": "engine-read",
        "session_meta_list": "engine-read",
    }


def test_writes_and_side_effecting_reads_stay_serial():
    pools = _pool_of(["scan", "resume", "pricing", "history_delete", "operation.apply"])
    assert pools == {
        "scan": "engine-serial",
        "resume": "engine-serial",
        # pricing 会走网络并非原子地覆写价格缓存文件,并行会互相踩
        "pricing": "engine-serial",
        "history_delete": "engine-serial",
        "operation.apply": "engine-serial",
    }


def test_parallel_dispatch_is_limited_to_pure_reads():
    for name in PARALLEL_READ_METHOD_NAMES:
        assert ENGINE_METHOD_POLICIES[name]["kind"] == "read"
    assert {"show", "session_asset"} <= PARALLEL_READ_METHOD_NAMES
    assert not {"pricing", "resume", "scan"} & PARALLEL_READ_METHOD_NAMES
