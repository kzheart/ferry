import io
import json
import threading
import time

import pytest

from engine.server.cli import serve
from engine.server.rpc import PROTOCOL


def test_parallel_read_requests_can_finish_out_of_input_order():
    active = 0
    peak = 0
    lock = threading.Lock()

    def handler(request: str) -> dict:
        nonlocal active, peak
        value = json.loads(request)
        with lock:
            active += 1
            peak = max(peak, active)
        try:
            time.sleep(0.04 if value["id"] == "slow" else 0.01)
            return {
                "protocol": PROTOCOL,
                "id": value["id"],
                "ok": True,
                "result": None,
            }
        finally:
            with lock:
                active -= 1

    output = io.StringIO()
    serve(
        io.StringIO(
            f'{{"protocol":"{PROTOCOL}","id":"slow","method":"health","params":{{}}}}\n'
            f'{{"protocol":"{PROTOCOL}","id":"fast","method":"version","params":{{}}}}\n'
        ),
        output,
        handler,
    )

    responses = [json.loads(line) for line in output.getvalue().splitlines()]
    assert peak == 2
    assert [response["id"] for response in responses] == ["fast", "slow"]


def test_non_parallel_request_stays_on_ordered_lane():
    active = 0
    peak = 0
    lock = threading.Lock()

    def handler(request: str) -> dict:
        nonlocal active, peak
        value = json.loads(request)
        with lock:
            active += 1
            peak = max(peak, active)
        try:
            time.sleep(0.01)
            return {
                "protocol": PROTOCOL,
                "id": value["id"],
                "ok": True,
                "result": None,
            }
        finally:
            with lock:
                active -= 1

    output = io.StringIO()
    serve(
        io.StringIO(
            f'{{"protocol":"{PROTOCOL}","id":"first","method":"scan","params":{{}}}}\n'
            f'{{"protocol":"{PROTOCOL}","id":"second","method":"scan","params":{{}}}}\n'
        ),
        output,
        handler,
    )

    assert peak == 1
    assert [json.loads(line)["id"] for line in output.getvalue().splitlines()] == [
        "first",
        "second",
    ]


def test_completed_request_failure_is_not_lost_when_reclaimed():
    def handler(_request: str) -> dict:
        raise RuntimeError("worker failed")

    with pytest.raises(RuntimeError, match="worker failed"):
        serve(
            io.StringIO(
                f'{{"protocol":"{PROTOCOL}","id":"failure",'
                '"method":"health","params":{}}\n'
            ),
            io.StringIO(),
            handler,
        )
