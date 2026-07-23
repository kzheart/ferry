import io
import json
import threading
import time

from engine.interfaces.cli import serve


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
            time.sleep(0.04 if value["request_id"] == "slow" else 0.01)
            return {"ok": True, "request_id": value["request_id"]}
        finally:
            with lock:
                active -= 1

    output = io.StringIO()
    serve(
        io.StringIO(
            '{"method":"health","request_id":"slow"}\n'
            '{"method":"version","request_id":"fast"}\n'
        ),
        output,
        handler,
    )

    responses = [json.loads(line) for line in output.getvalue().splitlines()]
    assert peak == 2
    assert [response["request_id"] for response in responses] == ["fast", "slow"]


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
            return {"ok": True, "request_id": value["request_id"]}
        finally:
            with lock:
                active -= 1

    output = io.StringIO()
    serve(
        io.StringIO(
            '{"method":"scan","request_id":"first"}\n'
            '{"method":"scan","request_id":"second"}\n'
        ),
        output,
        handler,
    )

    assert peak == 1
    assert [json.loads(line)["request_id"] for line in output.getvalue().splitlines()] == [
        "first",
        "second",
    ]
