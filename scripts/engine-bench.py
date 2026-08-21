#!/usr/bin/env python3
"""Session Engine 大语料性能基准。

在隔离沙箱 HOME 里合成大规模 Claude 会话库（默认 2000 个会话，每个 40 条记录，
另加 5 个 400 条记录的大会话），然后以 serve 模式驱动引擎，对同一批 RPC 请求
计时（多轮取中位数）：

  - startup      进程启动 + health 握手
  - scan_cold    首次全量扫描（空扫描缓存）
  - scan_warm    重复扫描（缓存命中，5 轮中位数）
  - search_meta  session_search 元数据搜索（5 轮）
  - search_content content_search 关键词搜索（5 轮）
  - search_regex content_search 正则搜索（3 轮）
  - show_large   show 读取 400 条记录的大会话（5 轮）
  - usage        usage_stats 全量聚合（3 轮）

说明：内容 FTS 索引（content-index.sqlite3）的构建不在计时面内——后台同步的
完成时机不可确定；search 请求命中的是元数据与正则路径。
用法：python3 scripts/engine-bench.py [--sessions N]
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = ROOT / "crates/ferry-engine/target/aarch64-apple-darwin/release/ferry-engine"
PROTOCOL = "ferry-ipc/1"


# ---------------------------------------------------------------------------
# 语料合成
# ---------------------------------------------------------------------------

def _record(kind: str, uuid: str, parent: str | None, session_id: str, cwd: str,
            ts: str, message: dict) -> str:
    return json.dumps({
        "parentUuid": parent, "isSidechain": False, "type": kind,
        "message": message, "uuid": uuid, "timestamp": ts,
        "cwd": cwd, "sessionId": session_id, "version": "2.1.204",
    }, ensure_ascii=False)


def synthesize(home: Path, sessions: int, big_sessions: int = 5) -> None:
    base_ts = 1755400000  # 秒
    for index in range(sessions + big_sessions):
        big = index >= sessions
        rounds = 200 if big else 20
        project = f"proj-{index % 40}"
        session_id = f"bench-{'big-' if big else ''}{index:05d}"
        cwd = f"/work/{project}"
        directory = home / ".claude/projects" / project
        directory.mkdir(parents=True, exist_ok=True)
        lines: list[str] = []
        parent: str | None = None
        needle = " needle_alpha marker" if index % 100 == 7 else ""
        for turn in range(rounds):
            ts = time.strftime(
                "%Y-%m-%dT%H:%M:%SZ",
                time.gmtime(base_ts + index * 60 + turn * 2),
            )
            user_id = f"{session_id}-u{turn}"
            lines.append(_record(
                "user", user_id, parent, session_id, cwd, ts,
                {"role": "user",
                 "content": f"请分析模块 module_{turn} 的性能瓶颈{needle}并给出结论。"},
            ))
            asst_id = f"{session_id}-a{turn}"
            lines.append(_record(
                "assistant", asst_id, user_id, session_id, cwd, ts,
                {"type": "message", "role": "assistant",
                 "model": "claude-opus-5",
                 "usage": {"input_tokens": 1200 + turn, "output_tokens": 300 + turn,
                           "cache_read_input_tokens": 800,
                           "cache_creation_input_tokens": 90},
                 "content": [{"type": "text",
                              "text": f"module_{turn} 的瓶颈在逐行 JSON 解析，建议引入增量缓存。"}],
                 "stop_reason": "end_turn"}))
            parent = asst_id
        (directory / f"{session_id}.jsonl").write_text(
            "\n".join(lines) + "\n", encoding="utf-8")


# ---------------------------------------------------------------------------
# serve 驱动
# ---------------------------------------------------------------------------

class Serve:
    def __init__(self, argv: list[str], env: dict, cwd: Path):
        self.proc = subprocess.Popen(
            argv, cwd=cwd, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.counter = 0

    def request(self, method: str, params: dict) -> tuple[dict, float]:
        self.counter += 1
        rid = f"b{self.counter}"
        frame = json.dumps({"protocol": PROTOCOL, "id": rid,
                            "method": method, "params": params},
                           ensure_ascii=False)
        started = time.monotonic()
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(frame + "\n")
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"引擎在 {method} 期间退出")
            data = json.loads(line)
            if data.get("id") == rid:
                elapsed = time.monotonic() - started
                if not data.get("ok"):
                    raise RuntimeError(f"{method} 失败: {data}")
                return data["result"], elapsed
            # 事件帧或其他响应：跳过

    def close(self) -> None:
        try:
            if self.proc.stdin:
                self.proc.stdin.close()
            self.proc.wait(timeout=30)
        except Exception:
            self.proc.kill()


def median_of(serve: Serve, method: str, params: dict, rounds: int) -> float:
    times = [serve.request(method, params)[1] for _ in range(rounds)]
    return statistics.median(times)


def bench_engine(name: str, argv: list[str], home: Path, cwd: Path) -> dict:
    data_dir = home / f".ferry-{name}"
    if data_dir.exists():
        shutil.rmtree(data_dir)
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(home),
        "XDG_DATA_HOME": str(home / ".local/share"),
        "GROK_HOME": str(home / ".grok"),
        "PI_CODING_AGENT_SESSION_DIR": str(home / ".pi-sessions"),
        "FERRY_OPENCODE_DB": str(home / "absent/opencode.db"),
        "FERRY_DATA_DIR": str(data_dir),
        "FERRY_BACKUP_DIR": str(data_dir / "backups"),
        "LANG": os.environ.get("LANG", "en_US.UTF-8"),
    }
    results: dict[str, float] = {}
    started = time.monotonic()
    serve = Serve(argv, env, cwd)
    _, handshake = serve.request("health", {})
    results["startup"] = time.monotonic() - started
    _, results["scan_cold"] = serve.request("scan", {})
    results["scan_warm"] = median_of(serve, "scan", {}, 5)
    results["search_meta"] = median_of(
        serve, "session_search", {"query": "module_7", "scope": "any"}, 5)
    results["search_content"] = median_of(
        serve, "content_search",
        {"query": "needle_alpha", "limit": 20, "scope": "any"}, 5)
    results["search_regex"] = median_of(
        serve, "content_search",
        {"query": "", "regex": "needle_[a-z]+", "limit": 20, "scope": "any"}, 3)
    scan_result, _ = serve.request("scan", {})
    big = next(row for row in scan_result["sessions"]
               if row["id"].startswith("bench-big-"))
    results["show_large"] = median_of(
        serve, "show", {"tool": "claude", "ref": big["ref"], "limit": 200}, 5)
    results["usage"] = median_of(serve, "usage_stats", {}, 3)
    serve.close()
    return results


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sessions", type=int, default=2000)
    parser.add_argument("--report", type=Path, default=None)
    args = parser.parse_args()

    if not RUST_BIN.exists():
        sys.exit(f"缺少 release 二进制: {RUST_BIN}")

    sandbox = Path(tempfile.mkdtemp(prefix="ferry-bench-"))
    try:
        print(f"合成 {args.sessions} + 5 个会话到 {sandbox} ...", flush=True)
        synthesize(sandbox, args.sessions)

        results = bench_engine("engine", [str(RUST_BIN), "serve"], sandbox, ROOT)
        for key, value in results.items():
            print(f"  {key:14s} {value * 1000:10.1f} ms", flush=True)

        rows = ["| 指标 | 耗时 (ms) |", "| --- | ---: |"]
        rows.extend(f"| {key} | {value * 1000:.1f} |" for key, value in results.items())
        corpus = (f"语料：{args.sessions} 个常规会话（40 条记录）+ 5 个大会话"
                  f"（400 条记录），Claude JSONL 格式，单机 macOS。")
        report = "\n".join([
            "# Ferry Session Engine 性能基准", "",
            corpus, "",
            *rows, "",
            "- 多轮取中位数；scan_cold 为空缓存首扫。",
            "- 内容 FTS 索引构建不在计时面内。",
        ]) + "\n"
        target = args.report or (sandbox / "bench-report.md")
        target.write_text(report, encoding="utf-8")
        print(f"\n报告已写入 {target}")
        print(report)
    finally:
        if args.report is not None:
            shutil.rmtree(sandbox, ignore_errors=True)


if __name__ == "__main__":
    main()
