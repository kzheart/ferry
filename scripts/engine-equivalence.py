#!/usr/bin/env python3
"""Python 引擎与 Rust 引擎的只读面等价性对照（docs/rust-engine-refactor-plan.md §WP-E）。

在同一个合成 HOME 沙箱里物化 `tests/fixtures/agent_formats` 的全部 case，把同一
批只读 RPC 请求分别喂给：

    python3 -m engine.server.cli rpc '<json>'
    crates/ferry-engine/target/debug/ferry-engine rpc '<json>'

规范化后逐方法 diff，输出 PASS/FAIL 报告。

用法
----
    python3 scripts/engine-equivalence.py                 # 全量
    python3 scripts/engine-equivalence.py --only scan show
    python3 scripts/engine-equivalence.py --rust-binary path/to/ferry-engine
    python3 scripts/engine-equivalence.py --verbose        # 打印完整 diff

覆盖的方法
----------
    health  version  env  scan  show  session_search
    agent_search_sessions  agent_get_usage

`pricing` 不在其中：它会打 models.dev 的网络，结果与运行时刻和缓存有关，不是
可判定的等价面。写操作面（operation.*）也不在其中，本脚本是**只读**对照。

规范化规则（两侧都做同一套变换后才比较）
--------------------------------------
1. 沙箱绝对路径 → `<home>`；
2. `fsr_`/`fml_` 是每个进程随机签发的句柄（方案 §2.2 第 14 条），按**首次出现
   顺序**替换成 `<ref:N>` / `<locator:N>`。两侧的会话排序规则一致，所以首次出现
   顺序也一致——顺序若真的不同，替换后照样会 diff 出来，不会掩盖问题；
3. 耗时/时刻类字段（`took_ms`、`elapsed_ms`、`now`、`fetched_at`、`indexed_at`）
   置为 `<elapsed>`；
4. 其余一律逐字节比较，包括 `revision`、`generation`、`updated`、`size`。

`show` 需要一个 Engine 签发的 ref，而 ref 只在**签发它的那个进程**里有效，所以
这条走 `serve` 模式：同一个进程里先 `scan` 拿 ref，再 `show`。其余方法都用
一次性 `rpc`。
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
DEFAULT_RUST_BINARY = ROOT / "crates" / "ferry-engine" / "target" / "debug" / "ferry-engine"

# 只有 Rust 引擎实现的 Agent：Python 侧根本不装配它们，两侧 env / scan 出参
# 必然不同，对照前整体剔除。常量与 engine.adapters.registry 同源，避免漂移。
from engine.adapters.registry import RUST_ONLY_AGENTS  # noqa: E402

SANDBOX_MARKER = "<home>"
# 每次调用都会变、且与语义无关的字段。
VOLATILE_KEYS = {"took_ms", "elapsed_ms", "now", "fetched_at", "indexed_at"}

REF_PATTERN = re.compile(r"fsr_[A-Za-z0-9_-]{8,}")
LOCATOR_PATTERN = re.compile(r"fml_[A-Za-z0-9_-]{8,}")


def load_fixture_tools():
    """复用 dump-canonical-fixtures.py 的沙箱与物化逻辑（脚本名带连字符，只能动态载入）。"""
    path = ROOT / "scripts" / "dump-canonical-fixtures.py"
    spec = importlib.util.spec_from_file_location("ferry_golden_dump", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# --------------------------------------------------------------------------
# 沙箱
# --------------------------------------------------------------------------


def sandbox_environ(dump, sandbox) -> dict:
    """`Sandbox.apply_env` 的**非破坏性**版本：返回子进程 env，不动本进程。"""
    home = str(sandbox.home)
    environ = dict(os.environ)
    environ.update(
        {
            "HOME": home,
            "USERPROFILE": home,
            "XDG_DATA_HOME": str(sandbox.home / ".local" / "share"),
            "XDG_CONFIG_HOME": str(sandbox.home / ".config"),
            "FERRY_DATA_DIR": str(sandbox.home / ".ferry"),
            "FERRY_BACKUP_DIR": str(sandbox.home / ".ferry" / "backups"),
            "FERRY_OPENCODE_DB": str(sandbox.opencode_db),
            "GROK_HOME": str(sandbox.home / ".grok"),
            "PI_CODING_AGENT_SESSION_DIR": str(sandbox.home / "pi-sessions"),
            # 两侧都要走干净的默认值，不能读到运行者的真实目录。
            "PYTHONPATH": str(ROOT),
        }
    )
    for key in ("PI_CODING_AGENT_DIR", "CODEX_HOME", "FERRY_DEBUG"):
        environ.pop(key, None)
    return environ


def materialize_all(dump, sandbox) -> dict:
    """把 13 个 case 全部物化进同一个沙箱（不像黄金 dump 那样逐 case reset）。"""
    counts = {}
    sandbox.opencode_db.parent.mkdir(parents=True, exist_ok=True)
    (sandbox.home / ".ferry").mkdir(parents=True, exist_ok=True)
    for agent in dump.AGENTS:
        sandbox.reset(agent)
        prepared = 0
        for case_dir in dump.cases(agent):
            try:
                dump.PREPARE[agent](sandbox, case_dir)
                prepared += 1
            except Exception as error:  # noqa: BLE001 - 逐 case 隔离
                print(f"  ! 物化失败 {agent}/{case_dir.name}: {error}", file=sys.stderr)
        counts[agent] = prepared
    return counts


# --------------------------------------------------------------------------
# 请求
# --------------------------------------------------------------------------


def protocol() -> str:
    return json.loads((ROOT / "contracts" / "ipc.json").read_text())["protocol"]


def envelope(method: str, params: dict, request_id: str) -> str:
    return json.dumps(
        {"protocol": protocol(), "id": request_id, "method": method, "params": params},
        ensure_ascii=False,
    )


# 一次性 rpc 就能判定的方法。
ONE_SHOT = [
    ("health", "health", {}),
    ("version", "version", {}),
    ("env", "env", {}),
    ("scan", "scan", {}),
    ("session_search", "session_search", {"query": "the", "limit": 5}),
    (
        "agent_search_sessions",
        "agent_search_sessions",
        {"query": "the", "limit": 5, "scope": "any"},
    ),
    ("agent_get_usage", "agent_get_usage", {}),
]
# 需要 ref 的方法（同一进程内先 scan 再问）。
SERVE_BACKED = ["show"]
ALL_METHODS = [name for name, _, _ in ONE_SHOT] + SERVE_BACKED


# --------------------------------------------------------------------------
# 两个引擎的调用
# --------------------------------------------------------------------------


class Engine:
    def __init__(self, name: str, argv: list[str], environ: dict):
        self.name = name
        self.argv = argv
        self.environ = environ

    def rpc(self, method: str, params: dict) -> dict:
        request = envelope(method, params, f"eq-{method}")
        process = subprocess.run(
            [*self.argv, "rpc", request],
            env=self.environ,
            capture_output=True,
            text=True,
            timeout=300,
            cwd=ROOT,
        )
        line = next(
            (item for item in process.stdout.splitlines() if item.strip()), ""
        )
        if not line:
            raise RuntimeError(
                f"{self.name} {method} 无输出\nstdout={process.stdout!r}\n"
                f"stderr={process.stderr[-4000:]}"
            )
        return json.loads(line)

    def serve_scan_then(self, follow_up) -> list[dict]:
        """serve 模式：先发 scan，读到应答后用其中的 ref 组装后续请求。

        `follow_up(scan_result) -> list[(method, params)]`。
        """
        process = subprocess.Popen(
            [*self.argv, "serve"],
            env=self.environ,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=ROOT,
        )
        try:
            process.stdin.write(envelope("scan", {}, "eq-serve-scan") + "\n")
            process.stdin.flush()
            scan = self._read_response(process, "eq-serve-scan")
            responses = []
            for index, (method, params) in enumerate(follow_up(scan)):
                request_id = f"eq-serve-{method}-{index}"
                process.stdin.write(envelope(method, params, request_id) + "\n")
                process.stdin.flush()
                responses.append(self._read_response(process, request_id))
            process.stdin.close()
            process.wait(timeout=120)
            return responses
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=30)

    @staticmethod
    def _read_response(process, request_id: str) -> dict:
        """跳过事件帧（无 `id`），等到目标 id 的应答。"""
        while True:
            line = process.stdout.readline()
            if not line:
                raise RuntimeError(f"serve 提前退出，缺 {request_id} 的应答")
            if not line.strip():
                continue
            frame = json.loads(line)
            if frame.get("id") == request_id:
                return frame


# --------------------------------------------------------------------------
# 规范化
# --------------------------------------------------------------------------


class Normalizer:
    """把两侧各自的随机 ref / 沙箱路径 / 耗时字段折成可比较的稳定形状。"""

    def __init__(self, home: str):
        self.home = home
        self.refs: dict[str, str] = {}
        self.locators: dict[str, str] = {}

    def _token(self, table: dict, prefix: str, raw: str) -> str:
        if raw not in table:
            table[raw] = f"<{prefix}:{len(table)}>"
        return table[raw]

    def _text(self, value: str) -> str:
        value = value.replace(self.home, SANDBOX_MARKER)
        value = REF_PATTERN.sub(lambda m: self._token(self.refs, "ref", m.group(0)), value)
        value = LOCATOR_PATTERN.sub(
            lambda m: self._token(self.locators, "locator", m.group(0)), value
        )
        return value

    def apply(self, value):
        if isinstance(value, str):
            return self._text(value)
        if isinstance(value, list):
            return [self.apply(item) for item in value]
        if isinstance(value, dict):
            return {
                self._text(str(key)): (
                    "<elapsed>" if key in VOLATILE_KEYS else self.apply(item)
                )
                for key, item in value.items()
            }
        return value


def render(value) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True)


def diff_lines(left: str, right: str, limit: int) -> list[str]:
    import difflib

    lines = list(
        difflib.unified_diff(
            left.splitlines(), right.splitlines(), "python", "rust", lineterm="", n=1
        )
    )
    return lines if limit <= 0 else lines[:limit]


# --------------------------------------------------------------------------
# 主流程
# --------------------------------------------------------------------------


def first_session_query(scan_response: dict):
    """从 scan 应答里挑一条会话，组装 show 请求。"""
    sessions = (scan_response.get("result") or {}).get("sessions") or []
    if not sessions:
        return []
    # 排序已由引擎保证（updated 降序、稳定），取首条即可两侧对齐。
    head = sessions[0]
    return [("show", {"tool": head["tool"], "ref": head["ref"]})]


def _sort_scan_rows(rows: list) -> None:
    """并列序归一：两侧都按 updated 降序排会话行，但 updated 完全相同时
    Python 保留 glob 的文件系统序、Rust 保留排序后的 glob 序（刻意差异，
    见 docs/rust-engine-refactor-plan.md §5）。fixture 把 mtime 钉成同一值，
    必然踩中并列；真实数据几乎不会。这里给并列行补一个确定性的次级键。"""
    if not isinstance(rows, list):
        return
    for row in rows:
        if isinstance(row, dict):
            _sort_scan_rows(row.get("children"))
    rows.sort(
        key=lambda row: (
            -(row.get("updated") or 0),
            str(row.get("tool", "")),
            str(row.get("id", "")),
        )
    )


def _drop_rust_only(name: str, value) -> None:
    """就地剔除 Rust-only Agent 的出参（env 的键、scan 的工具状态与会话行）。"""
    result = value.get("result") if isinstance(value, dict) else None
    if not isinstance(result, dict):
        return
    if name == "env":
        for agent_id in RUST_ONLY_AGENTS:
            result.pop(agent_id, None)
        return
    if name != "scan":
        return
    tools = result.get("tools")
    if isinstance(tools, dict):
        for agent_id in RUST_ONLY_AGENTS:
            tools.pop(agent_id, None)
    sessions = result.get("sessions")
    if isinstance(sessions, list):
        result["sessions"] = [
            row for row in sessions
            if not (isinstance(row, dict) and row.get("tool") in RUST_ONLY_AGENTS)
        ]


def canonicalize_method(name: str, value):
    _drop_rust_only(name, value)
    if name == "scan" and isinstance(value, dict):
        result = value.get("result")
        if isinstance(result, dict):
            _sort_scan_rows(result.get("sessions"))
    return value


def compare(name: str, python_value, rust_value, home: str, verbose: bool) -> bool:
    python_value = canonicalize_method(name, python_value)
    rust_value = canonicalize_method(name, rust_value)
    left = render(Normalizer(home).apply(python_value))
    right = render(Normalizer(home).apply(rust_value))
    if left == right:
        print(f"PASS  {name}")
        return True
    print(f"FAIL  {name}")
    for line in diff_lines(left, right, 0 if verbose else 40):
        print(f"      {line}")
    if not verbose:
        print("      （只显示前 40 行，完整 diff 用 --verbose）")
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--rust-binary", default=str(DEFAULT_RUST_BINARY),
        help="Rust 引擎二进制路径（默认 crates/ferry-engine/target/debug/ferry-engine）",
    )
    parser.add_argument(
        "--only", nargs="*", metavar="METHOD",
        help=f"只跑指定方法，可选：{' '.join(ALL_METHODS)}",
    )
    parser.add_argument("--verbose", action="store_true", help="打印完整 diff")
    args = parser.parse_args()

    rust_binary = Path(args.rust_binary).resolve()
    if not rust_binary.exists():
        print(
            f"找不到 Rust 引擎二进制：{rust_binary}\n"
            "先跑 `cd crates/ferry-engine && cargo build`。",
            file=sys.stderr,
        )
        return 2

    selected = set(args.only) if args.only else set(ALL_METHODS)
    unknown = selected - set(ALL_METHODS)
    if unknown:
        print(f"未知方法: {sorted(unknown)}", file=sys.stderr)
        return 2

    dump = load_fixture_tools()

    with tempfile.TemporaryDirectory(prefix="ferry-equivalence-") as tmp:
        sandbox = dump.Sandbox(Path(tmp).resolve())
        counts = materialize_all(dump, sandbox)
        total = sum(counts.values())
        print(f"沙箱 HOME: {sandbox.home}")
        print(
            "物化 case: "
            + ", ".join(f"{agent}={count}" for agent, count in counts.items())
            + f"（共 {total}）"
        )
        print(f"已豁免（Rust-only，无 Python 实现）：{sorted(RUST_ONLY_AGENTS)}\n")

        environ = sandbox_environ(dump, sandbox)
        python_engine = Engine(
            "python", [sys.executable, "-m", "engine.server.cli"], environ
        )
        rust_engine = Engine("rust", [str(rust_binary)], environ)

        results: dict[str, bool] = {}

        for name, method, params in ONE_SHOT:
            if name not in selected:
                continue
            try:
                left = python_engine.rpc(method, params)
                right = rust_engine.rpc(method, params)
            except Exception as error:  # noqa: BLE001 - 逐方法隔离
                print(f"FAIL  {name}: {type(error).__name__}: {error}")
                results[name] = False
                continue
            results[name] = compare(
                name, left, right, str(sandbox.home), args.verbose
            )

        if "show" in selected:
            try:
                left = python_engine.serve_scan_then(first_session_query)
                right = rust_engine.serve_scan_then(first_session_query)
            except Exception as error:  # noqa: BLE001
                print(f"FAIL  show: {type(error).__name__}: {error}")
                results["show"] = False
            else:
                if not left or not right:
                    print("FAIL  show: scan 未返回任何会话，无法取 ref")
                    results["show"] = False
                else:
                    results["show"] = compare(
                        "show", left, right, str(sandbox.home), args.verbose
                    )

    passed = sum(1 for ok in results.values() if ok)
    print(f"\n{passed}/{len(results)} 通过")
    for name in ALL_METHODS:
        if name in results and not results[name]:
            print(f"  - FAIL {name}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
