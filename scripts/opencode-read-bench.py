#!/usr/bin/env python3
"""Benchmark OpenCode scoped search and session_read against an isolated SQLite store."""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
NATIVE_ENGINE = ROOT / "crates/ferry-engine/target/release/ferry-engine"
CROSS_ENGINE = ROOT / "crates/ferry-engine/target/aarch64-apple-darwin/release/ferry-engine"
DEFAULT_ENGINE = NATIVE_ENGINE if NATIVE_ENGINE.is_file() else CROSS_ENGINE
PROTOCOL = "ferry-ipc/1"
SESSION_COLUMNS = [
    "id", "slug", "project_id", "directory", "path", "title", "version",
    "summary_additions", "summary_deletions", "summary_files", "cost",
    "tokens_input", "tokens_output", "tokens_reasoning", "tokens_cache_read",
    "tokens_cache_write", "time_created", "time_updated", "parent_id", "agent",
    "model", "permission", "share_url", "revert", "time_archived", "time_compacting",
]


def synthesize(database: Path, sessions: int, messages: int) -> str:
    target = "opencode-large-root"
    connection = sqlite3.connect(database)
    columns = ", ".join(f'"{name}"' for name in SESSION_COLUMNS)
    connection.executescript(
        f"CREATE TABLE session ({columns});"
        "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, time_created INTEGER);"
        "CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, data TEXT, time_created INTEGER);"
    )
    placeholders = ",".join("?" for _ in SESSION_COLUMNS)
    rows = []
    for index in range(sessions):
        identifier = target if index == 0 else f"opencode-meta-{index:05d}"
        title = "237 Mars affinity benchmark" if index == 0 else f"metadata session {index}"
        values = {"id": identifier, "directory": "/work/opencode-bench", "title": title,
                  "version": "1.18.3", "time_created": index + 1,
                  "time_updated": index + 1}
        rows.append(tuple(values.get(name) for name in SESSION_COLUMNS))
    connection.executemany(
        f"INSERT INTO session ({columns}) VALUES ({placeholders})",
        rows,
    )
    message_rows = []
    part_rows = []
    parent = None
    for index in range(messages):
        message_id = f"large-message-{index:05d}"
        role = "user" if index % 2 == 0 else "assistant"
        message = {"id": message_id, "sessionID": target, "role": role}
        if parent:
            message["parentID"] = parent
        text = f"OpenCode benchmark message {index} marker_{index % 17}"
        part = {"id": f"large-part-{index:05d}", "messageID": message_id,
                "sessionID": target, "type": "text", "text": text}
        message_rows.append((message_id, target, json.dumps(message), index))
        part_rows.append((part["id"], message_id, target, json.dumps(part), index))
        parent = message_id
    connection.executemany("INSERT INTO message VALUES (?,?,?,?)", message_rows)
    connection.executemany("INSERT INTO part VALUES (?,?,?,?,?)", part_rows)
    connection.commit()
    connection.close()
    return target


class Serve:
    def __init__(self, engine: Path, home: Path, database: Path):
        data_dir = home / ".ferry-opencode-bench"
        env = {
            **os.environ,
            "HOME": str(home),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "FERRY_DATA_DIR": str(data_dir),
            "FERRY_BACKUP_DIR": str(data_dir / "backups"),
            "FERRY_OPENCODE_DB": str(database),
            "GROK_HOME": str(home / ".grok"),
            "PI_CODING_AGENT_SESSION_DIR": str(home / ".pi-sessions"),
        }
        self.process = subprocess.Popen(
            [str(engine), "serve"], cwd=ROOT, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
        )
        self.counter = 0

    def request(self, method: str, params: dict, timeout: float = 120) -> tuple[dict, float]:
        self.counter += 1
        identifier = f"opencode-{self.counter}"
        frame = json.dumps({"protocol": PROTOCOL, "id": identifier,
                            "method": method, "params": params})
        assert self.process.stdin and self.process.stdout
        started = time.monotonic()
        self.process.stdin.write(frame + "\n")
        self.process.stdin.flush()
        deadline = started + timeout
        while time.monotonic() < deadline:
            line = self.process.stdout.readline()
            if not line:
                break
            response = json.loads(line)
            if response.get("id") != identifier:
                continue
            if not response.get("ok"):
                raise RuntimeError(f"{method} failed: {response}")
            return response["result"], time.monotonic() - started
        raise TimeoutError(f"{method} timed out")

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=10)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(len(ordered) * fraction + 0.999999) - 1))
    return ordered[index]


def measure(serve: Serve, method: str, params: dict, warmups: int, rounds: int) -> dict:
    for _ in range(warmups):
        serve.request(method, params)
    values = [serve.request(method, params)[1] * 1000 for _ in range(rounds)]
    return {
        "rounds": rounds,
        "median_ms": statistics.median(values),
        "p95_ms": percentile(values, 0.95),
        "min_ms": min(values),
        "max_ms": max(values),
    }


def markdown(result: dict) -> str:
    read = result["session_read_terms"]
    search = result["scoped_metadata_search"]
    return "\n".join([
        "# OpenCode Read Benchmark", "",
        f"Corpus: {result['corpus']['sessions']} sessions, "
        f"{result['corpus']['messages']} messages in the target root.", "",
        "| Metric | Median | p95 |", "| --- | ---: | ---: |",
        f"| session_read --terms | {read['median_ms']:.1f} ms | {read['p95_ms']:.1f} ms |",
        f"| --agent opencode metadata search | {search['median_ms']:.1f} ms | {search['p95_ms']:.1f} ms |",
        "",
    ])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", type=Path, default=DEFAULT_ENGINE)
    parser.add_argument("--sessions", type=int, default=2000)
    parser.add_argument("--messages", type=int, default=4000)
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not args.engine.is_file():
        raise SystemExit(f"engine binary not found: {args.engine}")

    with tempfile.TemporaryDirectory(prefix="ferry-opencode-bench-") as temporary:
        home = Path(temporary)
        database = home / "opencode.db"
        target = synthesize(database, args.sessions, args.messages)
        serve = Serve(args.engine, home, database)
        try:
            serve.request("health", {})
            scan, _ = serve.request("scan", {})
            record = next(row for row in scan["sessions"]
                          if row["tool"] == "opencode" and row["id"] == target)
            read = measure(
                serve, "session_read",
                {"tool": "opencode", "ref": record["ref"], "terms": ["marker_7"],
                 "limit": 20},
                2, args.rounds,
            )
            search = measure(
                serve, "content_search",
                {"query": "affinity benchmark", "agents": ["opencode"],
                 "scope": "metadata", "limit": 20},
                2, args.rounds,
            )
            result = {
                "engine": str(args.engine),
                "corpus": {"sessions": args.sessions, "messages": args.messages},
                "session_read_terms": read,
                "scoped_metadata_search": search,
            }
            print(json.dumps(result, indent=2))
            print(markdown(result))
            if args.report:
                args.report.write_text(markdown(result), encoding="utf-8")
        finally:
            serve.close()


if __name__ == "__main__":
    main()
