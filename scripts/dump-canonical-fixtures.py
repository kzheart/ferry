#!/usr/bin/env python3
"""把 tests/fixtures/agent_formats 的原生 capture 用 Python 引擎 dump 成黄金基线。

这是 Rust 引擎重写（docs/rust-engine-refactor-plan.md §WP-G / §4）的主安全网：
Python 引擎是行为基准（golden oracle），Rust 侧适配器工作包 C1..C5 逐字段比对
本脚本产出的 JSON。

用法
----
    python3 scripts/dump-canonical-fixtures.py            # 写入 tests/golden/
    python3 scripts/dump-canonical-fixtures.py --check    # 只校验、不落盘

产出
----
    tests/golden/canonical/<agent>/<case>.json   canonical Session 全字段快照
    tests/golden/scan/<agent>/<case>.json        scanner 扫描行（含归一化说明）

canonical 文件格式约定
----------------------
* 顶层就是 `engine.sessions.model.Session` 的 dict，**没有**包裹层，Rust 侧可以
  直接反序列化成对应的 struct。
* dataclass → dict 的转换是**通用的**（走 `dataclasses.fields()`），不维护字段
  清单；Python 侧新增字段会自动出现在黄金文件里，从而在 diff 中暴露。
* 字段全量输出：值为 `None` 的字段也显式写成 `null`，不做省略。字段顺序不表达
  语义（`sort_keys=True`，键按字典序）。
* `children` 递归展开为同样形状的 Session 对象数组；`messages[].blocks[].tool`
  / `.image`、`tool.result`、`agent_edges[]`、`context_compactions[]` 同理。
* 未被 dataclass 覆盖的自由字典（`loss[]` 事件、`ToolCall.input`、
  `ToolResultBlock.data`、`ContextCompaction.metrics/source_meta`）原样输出。
* 编码：`ensure_ascii=False`、`indent=2`、末尾一个换行。非 ASCII 字面保留，
  Rust 侧按 UTF-8 读取即可。
* `set`/`tuple` 不出现在 canonical model 中；若将来出现，转换器会把 tuple 当
  list 处理，set 会直接报错（刻意不静默排序，避免掩盖不确定性）。

scan 文件格式约定
-----------------
* 顶层是 `{"_normalized": {...}, "rows": [...]}`。`rows` 是 scanner 返回的行
  （已经过 `sessions.topology.session_roots` 装配，含 `children` 嵌套）。
* `_normalized.environment_dependent_fields` 列出**内容无关、由运行环境决定**
  的字段，Rust 侧对照时应当按各自环境重新计算而不是硬编码期望值。
* 路径类字段（`path`）里的沙箱根被替换成字面量 `<home>`，即
  `<home>/.claude/projects/<case>/<id>.jsonl` 这种形态；这样黄金文件与临时目录
  无关，同时保留了各家 agent 的真实存储布局。
* mtime 类字段（`updated`/`own_updated`）之所以是稳定值，是因为物化 fixture 时
  统一把 mtime 设成 `FIXED_MTIME`（见下），而不是事后抹掉。

幂等性
------
脚本每次运行都在一个全新临时目录里重建各 agent 的原生存储布局（HOME 沙箱），
把 fixture 原样拷进去，再把所有物化文件的 mtime 钉到 `FIXED_MTIME`。因此：

* 扫描行里的 `updated`/`created`/`size` 全部只取决于 fixture 内容；
* 输出中任何残留的沙箱绝对路径都会被替换成 `<home>`；
* reader 侧不产生随机 ID（随机 ID 只在 writer/codec 里，本脚本不走写链路）。

连续运行两次 `git diff` 必须为空；这是 WP-G 的验收条件之一。

沙箱与外部依赖
--------------
各 adapter 的 scanner/reader 依赖真实的 agent 存储位置，脚本在 import engine
**之前**改写环境变量（`HOME` / `GROK_HOME` / `PI_CODING_AGENT_SESSION_DIR` /
`FERRY_OPENCODE_DB` / `FERRY_DATA_DIR` / `FERRY_BACKUP_DIR`），因为若干模块在
import 期就把路径固化成模块级常量（如 `opencode.store.DB_PATH`、
`codex.topology._META_CACHE_PATH`）。

* claude：`<home>/.claude/projects/<case>/<id>.jsonl`
* codex ：`<home>/.codex/sessions/<Y>/<M>/<D>/rollout-<id>.jsonl`，
          并按 fixture 的 `registration.json` 合成 `<home>/.codex/state_5.sqlite`
          （threads + thread_spawn_edges 两张表）
* opencode：fixture 的 `session.json` 本身就是 SQLite 三张表（session/message/
          part）的导出形状，按 `engine/adapters/opencode/store.py` 声明的当前列
          集合合成只读库到 `FERRY_OPENCODE_DB`
* pi   ：`PI_CODING_AGENT_SESSION_DIR/<case>/<id>.jsonl`（fixture 无 manifest）
* grok ：`<home>/.grok/sessions/<case>/<id>/{summary,updates,chat_history}`

每个 case 单独物化、单独扫描，因此扫描行天然只包含该 case 自己的会话树。
"""
from __future__ import annotations

import argparse
import dataclasses
import json
import os
import shutil
import sqlite3
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures" / "agent_formats"
GOLDEN = ROOT / "tests" / "golden"
AGENTS = ("claude", "codex", "opencode", "pi", "grok")

# 所有物化文件统一使用的 mtime（2026-07-25T00:00:00Z），让扫描行里的 updated
# 只取决于 fixture 内容而不是运行时刻。
FIXED_MTIME = 1784937600
SANDBOX_MARKER = "<home>"

# scanner 行中由运行环境（而非 fixture 内容）决定的字段。
ENVIRONMENT_DEPENDENT = {
    "claude": ["path", "updated", "own_updated", "size", "own_size"],
    "codex": ["path", "updated", "own_updated", "size", "own_size"],
    # opencode 扫描行不带文件路径（path 恒为 ""、size 恒为 0）；
    # updated/created 来自 SQLite 列，而 fixture 未提供时间列。
    "opencode": ["updated", "own_updated"],
    "pi": ["path", "updated", "own_updated", "size", "own_size"],
    # grok 的 updated 优先取 summary.updated_at，只有缺失时才回落 mtime；
    # size 取的是 summary.json 的字节数。
    "grok": ["path", "updated", "own_updated", "size", "own_size"],
}

# codex `state_5.sqlite` 的当前结构，与 tests/test_codex_additional_cases.py 一致。
# `IF NOT EXISTS`：黄金 dump 每个 case 前都会 reset，库总是空的；但
# `scripts/engine-equivalence.py` 把全部 case 物化进**同一个**沙箱，同一份
# state_5.sqlite 会被建第二次，裸 CREATE TABLE 会在第二个 case 上炸掉。
CODEX_STATE_SCHEMA = """
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
    title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
    approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
    agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
    recency_at INTEGER NOT NULL DEFAULT 0, history_mode TEXT NOT NULL DEFAULT 'legacy'
);
CREATE TABLE IF NOT EXISTS thread_spawn_edges (
    parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL PRIMARY KEY,
    status TEXT NOT NULL
);
"""

# opencode SQLite 的当前列集合，取自 engine/adapters/opencode/store.py 的
# _CURRENT_DB_COLUMNS；`export_from_database` 会 SELECT * 并逐列取值。
OPENCODE_SESSION_COLUMNS = (
    "id", "slug", "project_id", "directory", "path", "title", "version",
    "summary_additions", "summary_deletions", "summary_files", "cost",
    "tokens_input", "tokens_output", "tokens_reasoning", "tokens_cache_read",
    "tokens_cache_write", "time_created", "time_updated", "parent_id",
    "agent", "model", "permission", "share_url", "revert", "time_archived",
    "time_compacting",
)


# --------------------------------------------------------------------------
# 沙箱：必须在 import engine 之前完成
# --------------------------------------------------------------------------

class Sandbox:
    """一次运行共用的假 HOME；每个 case 前把对应 agent 的存储清空重建。"""

    def __init__(self, home: Path):
        self.home = home
        self.opencode_db = home / "opencode" / "storage.db"

    def apply_env(self) -> None:
        os.environ["HOME"] = str(self.home)
        os.environ["USERPROFILE"] = str(self.home)
        os.environ["XDG_DATA_HOME"] = str(self.home / ".local" / "share")
        os.environ["XDG_CONFIG_HOME"] = str(self.home / ".config")
        os.environ["FERRY_DATA_DIR"] = str(self.home / ".ferry")
        os.environ["FERRY_BACKUP_DIR"] = str(self.home / ".ferry" / "backups")
        os.environ["FERRY_OPENCODE_DB"] = str(self.opencode_db)
        os.environ["GROK_HOME"] = str(self.home / ".grok")
        os.environ["PI_CODING_AGENT_SESSION_DIR"] = str(self.home / "pi-sessions")
        # Pi 的 settings 探测与 codex 的 registry 都可能读到用户真实目录，
        # 这两个显式变量确保不会。
        os.environ.pop("PI_CODING_AGENT_DIR", None)
        os.environ.pop("CODEX_HOME", None)
        self.opencode_db.parent.mkdir(parents=True, exist_ok=True)
        (self.home / ".ferry").mkdir(parents=True, exist_ok=True)

    def store_root(self, agent: str) -> Path:
        return {
            "claude": self.home / ".claude",
            "codex": self.home / ".codex",
            "opencode": self.opencode_db.parent,
            "pi": self.home / "pi-sessions",
            "grok": self.home / ".grok",
        }[agent]

    def reset(self, agent: str) -> None:
        root = self.store_root(agent)
        if root.exists():
            shutil.rmtree(root)
        root.mkdir(parents=True, exist_ok=True)

    def normalize(self, value):
        """把输出里残留的沙箱绝对路径换成稳定字面量。"""
        home = str(self.home)
        if isinstance(value, str):
            return value.replace(home, SANDBOX_MARKER)
        if isinstance(value, list):
            return [self.normalize(item) for item in value]
        if isinstance(value, dict):
            return {key: self.normalize(item) for key, item in value.items()}
        return value


def _freeze(path: Path) -> None:
    """把物化产物的 mtime 钉死，保证扫描行里的 updated 稳定。"""
    targets = [path]
    if path.is_dir():
        targets = [path, *sorted(path.rglob("*"))]
    for target in targets:
        os.utime(target, (FIXED_MTIME, FIXED_MTIME))


def _copy(src: Path, dest: Path) -> Path:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dest)
    _freeze(dest)
    return dest


def _manifest(case_dir: Path) -> dict:
    path = case_dir / "manifest.json"
    return json.loads(path.read_text()) if path.exists() else {}


def _native_stem(case_dir: Path, fallback: str) -> str:
    """优先复用 manifest 记录的原生文件名，让物化布局贴近真实 capture。"""
    manifest = _manifest(case_dir)
    sources = manifest.get("source_paths") or []
    if sources and isinstance(sources[0], str):
        return Path(sources[0]).stem
    return manifest.get("session_id") or fallback


# --------------------------------------------------------------------------
# 通用 dataclass → JSON 形状转换
# --------------------------------------------------------------------------

def to_jsonable(value):
    """递归展开 dataclass；不手写字段清单，避免漏字段。"""
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: to_jsonable(getattr(value, field.name))
            for field in dataclasses.fields(value)
        }
    if isinstance(value, dict):
        return {str(key): to_jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [to_jsonable(item) for item in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    raise TypeError(f"canonical dump 遇到不可序列化的值: {type(value).__name__}")


def write_json(path: Path, payload) -> str:
    text = json.dumps(payload, sort_keys=True, ensure_ascii=False, indent=2) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return text


# --------------------------------------------------------------------------
# 各 agent 的物化 + 读取
# --------------------------------------------------------------------------

def _jsonl_records(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").split("\n")
        if line.strip()
    ]


def prepare_claude(sandbox: Sandbox, case_dir: Path) -> str:
    stem = _native_stem(case_dir, case_dir.name)
    target = sandbox.store_root("claude") / "projects" / case_dir.name / f"{stem}.jsonl"
    _copy(case_dir / "session.jsonl", target)
    return str(target)


def prepare_codex(sandbox: Sandbox, case_dir: Path) -> str:
    stem = _native_stem(case_dir, f"rollout-{case_dir.name}")
    home = sandbox.store_root("codex")
    target = home / "sessions" / "2026" / "07" / "25" / f"{stem}.jsonl"
    _copy(case_dir / "session.jsonl", target)
    _write_codex_registry(home / "state_5.sqlite", case_dir, target)
    return str(target)


def _write_codex_registry(db_path: Path, case_dir: Path, rollout: Path) -> None:
    """按 fixture 的 registration.json 合成 codex 会话注册库。

    engine 只从这里读 `thread_spawn_edges`（父子边）与 `threads`（closure 指纹），
    fixture 未提供边，所以表建出来但为空；`rollout_path` 重写成物化后的真实路径。
    """
    registration = case_dir / "registration.json"
    if not registration.exists():
        return
    threads = (json.loads(registration.read_text()).get("threads") or [])
    with sqlite3.connect(db_path) as db:
        db.executescript(CODEX_STATE_SCHEMA)
        for thread in threads:
            db.execute(
                "INSERT OR REPLACE INTO threads (id, rollout_path, created_at,"
                " updated_at, source, model_provider, cwd, title, sandbox_policy,"
                " approval_mode, cli_version, first_user_message)"
                " VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                (
                    thread.get("id", ""), str(rollout), FIXED_MTIME, FIXED_MTIME,
                    "cli", "openai", thread.get("cwd", ""), thread.get("title", ""),
                    "workspace-write", "on-request",
                    thread.get("cli_version", ""),
                    thread.get("first_user_message", ""),
                ),
            )
    _freeze(db_path)


def prepare_opencode(sandbox: Sandbox, case_dir: Path) -> str:
    """fixture 的 session.json 就是三张表的行；按当前列集合还原成 SQLite 库。"""
    fixture = json.loads((case_dir / "session.json").read_text())
    db_path = sandbox.opencode_db
    if db_path.exists():
        db_path.unlink()
    columns = ", ".join(f'"{name}"' for name in OPENCODE_SESSION_COLUMNS)
    with sqlite3.connect(db_path) as db:
        db.execute(f"CREATE TABLE session ({columns})")
        db.execute(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT,"
            " data TEXT, time_created INTEGER)")
        db.execute(
            "CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT,"
            " session_id TEXT, data TEXT, time_created INTEGER)")
        session = fixture["session"]
        db.execute(
            f"INSERT INTO session ({columns}) VALUES"
            f" ({','.join('?' for _ in OPENCODE_SESSION_COLUMNS)})",
            tuple(session.get(name) for name in OPENCODE_SESSION_COLUMNS),
        )
        # time_created 用 fixture 里的下标，保证 store 的 ORDER BY 复现原顺序。
        for index, row in enumerate(fixture.get("messages") or []):
            db.execute(
                "INSERT INTO message (id, session_id, data, time_created)"
                " VALUES (?,?,?,?)",
                (row["id"], row["session_id"], row["data"], index),
            )
        for index, row in enumerate(fixture.get("parts") or []):
            db.execute(
                "INSERT INTO part (id, message_id, session_id, data, time_created)"
                " VALUES (?,?,?,?,?)",
                (row["id"], row["message_id"], row["session_id"],
                 row["data"], index),
            )
    _freeze(db_path)
    return str(fixture["session"]["id"])


def prepare_pi(sandbox: Sandbox, case_dir: Path) -> str:
    # Pi fixture 无 manifest，会话 id 只能从 v3 头部记录里取。
    header = _jsonl_records(case_dir / "session.jsonl")[0]
    stem = header.get("id") or case_dir.name
    target = sandbox.store_root("pi") / case_dir.name / f"{stem}.jsonl"
    _copy(case_dir / "session.jsonl", target)
    return str(target)


def prepare_grok(sandbox: Sandbox, case_dir: Path) -> str:
    summary = json.loads((case_dir / "summary.json").read_text())
    bundle_id = (summary.get("info") or {}).get("id") or case_dir.name
    target = sandbox.store_root("grok") / "sessions" / case_dir.name / bundle_id
    target.mkdir(parents=True, exist_ok=True)
    for member in sorted(case_dir.iterdir()):
        if member.is_file():
            _copy(member, target / member.name)
    _freeze(target)
    return str(target)


PREPARE = {
    "claude": prepare_claude,
    "codex": prepare_codex,
    "opencode": prepare_opencode,
    "pi": prepare_pi,
    "grok": prepare_grok,
}


class Cache:
    """scanner 的最小缓存端口；黄金 dump 不需要跨 case 复用。"""

    def get(self, *_args):
        return None

    def put(self, *_args):
        return None

    def flush(self):
        return None


def load_engine():
    """延迟到环境变量就位之后再 import，模块级路径常量才会落在沙箱里。"""
    if str(ROOT) not in sys.path:
        sys.path.insert(0, str(ROOT))
    from engine.adapters.claude import reader as claude_reader
    from engine.adapters.claude import scanner as claude_scanner
    from engine.adapters.codex import reader as codex_reader
    from engine.adapters.codex import scanner as codex_scanner
    from engine.adapters.opencode import reader as opencode_reader
    from engine.adapters.opencode import scanner as opencode_scanner
    from engine.adapters.pi import reader as pi_reader
    from engine.adapters.pi import scanner as pi_scanner
    from engine.adapters.grok import reader as grok_reader
    from engine.adapters.grok import scanner as grok_scanner

    return {
        "claude": (claude_reader.read, claude_scanner.scan),
        "codex": (codex_reader.read, codex_scanner.scan),
        "opencode": (opencode_reader.read, opencode_scanner.scan),
        "pi": (pi_reader.read, pi_scanner.scan),
        "grok": (grok_reader.read, grok_scanner.scan),
    }


# --------------------------------------------------------------------------
# 主流程
# --------------------------------------------------------------------------

def cases(agent: str) -> list[Path]:
    root = FIXTURES / agent
    return sorted(path for path in root.iterdir() if path.is_dir())


def dump_case(sandbox, engine, agent: str, case_dir: Path, check: bool):
    sandbox.reset(agent)
    ref = PREPARE[agent](sandbox, case_dir)
    read, scan = engine[agent]

    session = read(ref)
    canonical = sandbox.normalize(to_jsonable(session))

    rows = sandbox.normalize([dict(row) for row in scan(Cache())])
    scan_payload = {
        "_normalized": {
            "sandbox_root_marker": SANDBOX_MARKER,
            "fixed_mtime_seconds": FIXED_MTIME,
            "environment_dependent_fields": ENVIRONMENT_DEPENDENT[agent],
            "note": (
                "path 中的沙箱根已替换为 <home>；updated/own_updated 之所以稳定，"
                "是因为物化 fixture 时把 mtime 统一设成 fixed_mtime_seconds。"
                "Rust 侧对照真实环境时，这些字段应按各自环境重新计算。"
            ),
        },
        "rows": rows,
    }

    outputs = [
        (GOLDEN / "canonical" / agent / f"{case_dir.name}.json", canonical),
        (GOLDEN / "scan" / agent / f"{case_dir.name}.json", scan_payload),
    ]
    stale = []
    for path, payload in outputs:
        text = json.dumps(payload, sort_keys=True, ensure_ascii=False, indent=2) + "\n"
        if check:
            if not path.exists() or path.read_text(encoding="utf-8") != text:
                stale.append(path)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
    return [path for path, _ in outputs], stale


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check", action="store_true",
        help="只校验现有黄金文件是否与当前 Python 引擎一致，不写盘",
    )
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="ferry-golden-") as tmp:
        sandbox = Sandbox(Path(tmp).resolve())
        sandbox.apply_env()
        engine = load_engine()

        written, skipped, stale = [], [], []
        for agent in AGENTS:
            for case_dir in cases(agent):
                try:
                    paths, case_stale = dump_case(
                        sandbox, engine, agent, case_dir, args.check)
                except Exception as error:  # noqa: BLE001 - 逐 case 隔离失败
                    skipped.append((agent, case_dir.name,
                                    f"{type(error).__name__}: {error}"))
                    continue
                written.extend(paths)
                stale.extend(case_stale)

    for path in written:
        print(("checked " if args.check else "wrote   ")
              + str(path.relative_to(ROOT)))
    if skipped:
        print("\n跳过的 case（无法离线构造或读取失败）：")
        for agent, case, reason in skipped:
            print(f"  - {agent}/{case}: {reason}")
    if stale:
        print("\n与当前 Python 引擎不一致的黄金文件：")
        for path in stale:
            print(f"  - {path.relative_to(ROOT)}")
        return 1
    total = len(written) // 2
    print(f"\n覆盖 {total} 个 case，跳过 {len(skipped)} 个。")
    return 1 if skipped else 0


if __name__ == "__main__":
    sys.exit(main())
