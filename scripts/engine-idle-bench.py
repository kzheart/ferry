#!/usr/bin/env python3
"""Measure Ferry engine idle resources and post-read recovery in an isolated corpus."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import platform
import statistics
import subprocess
import tempfile
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
NATIVE_ENGINE = ROOT / "crates/ferry-engine/target/release/ferry-engine"
CROSS_ENGINE = ROOT / "crates/ferry-engine/target/aarch64-apple-darwin/release/ferry-engine"
DEFAULT_ENGINE = NATIVE_ENGINE if NATIVE_ENGINE.is_file() else CROSS_ENGINE
PROTOCOL = "ferry-ipc/1"


def record(kind: str, session_id: str, ordinal: int, parent: str | None) -> str:
    identifier = f"{session_id}-{kind}-{ordinal}"
    role = "user" if kind == "user" else "assistant"
    content: str | list[dict[str, str]]
    if role == "user":
        content = f"idle benchmark turn {ordinal} marker_{ordinal % 17}"
    else:
        content = [{"type": "text", "text": f"benchmark response {ordinal}"}]
    return json.dumps(
        {
            "parentUuid": parent,
            "isSidechain": False,
            "type": kind,
            "message": {
                "role": role,
                "model": "claude-opus-5" if role == "assistant" else None,
                "content": content,
            },
            "uuid": identifier,
            "timestamp": "2026-08-21T00:00:00Z",
            "cwd": "/work/idle-bench",
            "sessionId": session_id,
            "version": "2.1.204",
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )


def synthesize(home: Path, sessions: int, records: int, large_records: int) -> None:
    directory = home / ".claude/projects/idle-bench"
    directory.mkdir(parents=True)
    for session_index in range(sessions + 1):
        count = large_records if session_index == sessions else records
        session_id = f"idle-{'large' if session_index == sessions else session_index:0>5}"
        lines: list[str] = []
        parent = None
        for ordinal in range(count // 2):
            user = f"{session_id}-user-{ordinal}"
            lines.append(record("user", session_id, ordinal, parent))
            lines.append(record("assistant", session_id, ordinal, user))
            parent = f"{session_id}-assistant-{ordinal}"
        (directory / f"{session_id}.jsonl").write_text("\n".join(lines) + "\n")


class RpcServe:
    def __init__(self, engine: Path, home: Path):
        data_dir = home / ".ferry-idle-bench"
        env = {
            **os.environ,
            "HOME": str(home),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "FERRY_DATA_DIR": str(data_dir),
            "FERRY_BACKUP_DIR": str(data_dir / "backups"),
            "FERRY_OPENCODE_DB": str(home / "absent/opencode.db"),
            "GROK_HOME": str(home / ".grok"),
            "PI_CODING_AGENT_SESSION_DIR": str(home / ".pi-sessions"),
        }
        self.process = subprocess.Popen(
            [str(engine), "serve"],
            cwd=ROOT,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self._condition = threading.Condition()
        self._write_lock = threading.Lock()
        self._responses: dict[str, dict] = {}
        self._counter = 0
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def _read(self) -> None:
        assert self.process.stdout
        for line in self.process.stdout:
            message = json.loads(line)
            identifier = message.get("id")
            if identifier is None:
                continue
            with self._condition:
                self._responses[str(identifier)] = message
                self._condition.notify_all()

    def request(self, method: str, params: dict, timeout: float = 120) -> tuple[dict, float]:
        with self._write_lock:
            self._counter += 1
            identifier = f"idle-{self._counter}"
            frame = json.dumps(
                {"protocol": PROTOCOL, "id": identifier, "method": method, "params": params}
            )
            assert self.process.stdin
            started = time.monotonic()
            self.process.stdin.write(frame + "\n")
            self.process.stdin.flush()
        deadline = started + timeout
        with self._condition:
            while identifier not in self._responses:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"{method} timed out after {timeout}s")
                self._condition.wait(remaining)
            response = self._responses.pop(identifier)
        if not response.get("ok"):
            raise RuntimeError(f"{method} failed: {response}")
        return response["result"], time.monotonic() - started

    def close(self) -> str:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=10)
        assert self.process.stderr
        return self.process.stderr.read()


@dataclass
class ProcessSample:
    at: float
    cpu_seconds: float
    rss_mib: float
    threads: int | None


def parse_cpu_time(value: str) -> float:
    days = 0
    if "-" in value:
        day, value = value.split("-", 1)
        days = int(day)
    parts = [float(part) for part in value.split(":")]
    seconds = 0.0
    for part in parts:
        seconds = seconds * 60 + part
    return days * 86400 + seconds


def process_sample(pid: int) -> ProcessSample:
    system = platform.system()
    output = subprocess.check_output(
        ["ps", "-o", "time=", "-o", "rss=", "-p", str(pid)], text=True
    ).strip()
    cpu_time, rss_kib = output.split()
    if system == "Darwin":
        thread_lines = subprocess.check_output(
            ["ps", "-M", "-p", str(pid), "-o", "pid="], text=True
        )
        threads = len(thread_lines.splitlines())
    else:
        status = Path(f"/proc/{pid}/status").read_text()
        threads = next(
            int(line.split(":", 1)[1])
            for line in status.splitlines()
            if line.startswith("Threads:")
        )
    return ProcessSample(
        at=time.monotonic(),
        cpu_seconds=parse_cpu_time(cpu_time),
        rss_mib=int(rss_kib) / 1024,
        threads=int(threads),
    )


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(len(ordered) * fraction + 0.999999) - 1))
    return ordered[index]


def measure_process(pid: int, seconds: int, interval: float, include_samples: bool) -> dict:
    samples = [process_sample(pid)]
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        time.sleep(min(interval, max(0.0, deadline - time.monotonic())))
        samples.append(process_sample(pid))
    cpu_percent = []
    for before, after in zip(samples, samples[1:]):
        wall = after.at - before.at
        cpu_percent.append((after.cpu_seconds - before.cpu_seconds) / wall * 100)
    result = {
        "duration_seconds": samples[-1].at - samples[0].at,
        "cpu_average_percent": (
            (samples[-1].cpu_seconds - samples[0].cpu_seconds)
            / (samples[-1].at - samples[0].at)
            * 100
        ),
        "cpu_p95_percent": percentile(cpu_percent, 0.95),
        "rss_start_mib": samples[0].rss_mib,
        "rss_end_mib": samples[-1].rss_mib,
        "rss_peak_mib": max(sample.rss_mib for sample in samples),
        "rss_growth_mib": samples[-1].rss_mib - samples[0].rss_mib,
        "threads_peak": max(sample.threads or 0 for sample in samples),
        "sample_count": len(samples),
    }
    if include_samples:
        result["samples"] = [asdict(sample) for sample in samples]
    return result


def wait_for_index(serve: RpcServe, timeout: int = 180) -> dict:
    deadline = time.monotonic() + timeout
    last = {}
    while time.monotonic() < deadline:
        result, _ = serve.request(
            "content_search",
            {"query": "marker_7", "scope": "content", "limit": 1},
        )
        last = result.get("content_index", {})
        if last.get("ready"):
            return last
        time.sleep(0.5)
    raise TimeoutError(f"content index did not become ready: {last}")


def stress(serve: RpcServe, ref: str, concurrency: int) -> dict:
    params = {"tool": "claude", "ref": ref, "from_message": 1, "limit": 200}
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency + 1) as executor:
        reads = [executor.submit(serve.request, "show", params) for _ in range(concurrency)]
        time.sleep(0.01)
        _, health_seconds = serve.request("health", {}, timeout=30)
        read_seconds = [future.result()[1] for future in reads]
    return {
        "concurrency": concurrency,
        "health_ms": health_seconds * 1000,
        "read_median_ms": statistics.median(read_seconds) * 1000,
        "read_p95_ms": percentile(read_seconds, 0.95) * 1000,
    }


def markdown(result: dict) -> str:
    idle = result["idle"]
    recovery = result["recovery"]
    stress_result = result["stress"]
    rows = [
        ("idle CPU average", f"{idle['cpu_average_percent']:.3f}%"),
        ("idle CPU p95", f"{idle['cpu_p95_percent']:.3f}%"),
        ("idle RSS end", f"{idle['rss_end_mib']:.1f} MiB"),
        ("idle RSS growth", f"{idle['rss_growth_mib']:.1f} MiB"),
        ("stress health", f"{stress_result['health_ms']:.1f} ms"),
        ("stress read p95", f"{stress_result['read_p95_ms']:.1f} ms"),
        ("recovery CPU average", f"{recovery['cpu_average_percent']:.3f}%"),
        ("recovery RSS end", f"{recovery['rss_end_mib']:.1f} MiB"),
    ]
    body = ["# Ferry Engine Idle Benchmark", "", f"Platform: `{result['platform']}`", ""]
    body.extend(["| Metric | Value |", "| --- | ---: |"])
    body.extend(f"| {name} | {value} |" for name, value in rows)
    return "\n".join(body) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", type=Path, default=DEFAULT_ENGINE)
    parser.add_argument("--sessions", type=int, default=2000)
    parser.add_argument("--records", type=int, default=40)
    parser.add_argument("--large-records", type=int, default=4000)
    parser.add_argument("--idle-seconds", type=int, default=60)
    parser.add_argument("--recovery-seconds", type=int, default=60)
    parser.add_argument("--sample-interval", type=float, default=1.0)
    parser.add_argument("--stress-concurrency", type=int, default=4)
    parser.add_argument("--include-samples", action="store_true")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not args.engine.is_file():
        raise SystemExit(f"engine binary not found: {args.engine}")

    with tempfile.TemporaryDirectory(prefix="ferry-idle-bench-") as temporary:
        home = Path(temporary)
        synthesize(home, args.sessions, args.records, args.large_records)
        serve = RpcServe(args.engine, home)
        try:
            _, startup_seconds = serve.request("health", {})
            scan, scan_seconds = serve.request("scan", {})
            content_index = wait_for_index(serve)
            large = next(
                session for session in scan["sessions"] if session["id"] == "idle-large"
            )
            idle = measure_process(
                serve.process.pid,
                args.idle_seconds,
                args.sample_interval,
                args.include_samples,
            )
            stress_result = stress(serve, large["ref"], args.stress_concurrency)
            recovery = measure_process(
                serve.process.pid,
                args.recovery_seconds,
                args.sample_interval,
                args.include_samples,
            )
            result = {
                "platform": platform.platform(),
                "engine": str(args.engine),
                "corpus": {
                    "sessions": args.sessions,
                    "records": args.records,
                    "large_records": args.large_records,
                },
                "startup_ms": startup_seconds * 1000,
                "scan_ms": scan_seconds * 1000,
                "content_index": content_index,
                "idle": idle,
                "stress": stress_result,
                "recovery": recovery,
            }
            print(json.dumps(result, ensure_ascii=False, indent=2))
            print(markdown(result))
            if args.report:
                args.report.write_text(markdown(result))
        finally:
            serve.close()


if __name__ == "__main__":
    main()
