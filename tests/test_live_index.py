"""活索引:快照秒回、增量 delta 推送、单工具重扫与源变更轮询。"""
import io
import json
import threading
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

from engine.adapters.contracts import filesystem_reference
from engine.context import EngineContext
from engine.server.notify import Notifier
from engine.sessions import scan as scanning
from engine.sessions.index import AgentSessionIndex
from engine.sessions.live import LiveIndexService, _tree_stamp


class _Cache:
    def flush(self):
        pass


class _FileBrowser:
    def __init__(self, root: Path):
        self.root = root

    def scan(self, _cache):
        rows = []
        for path in sorted(self.root.glob("*.jsonl")):
            stat = path.stat()
            rows.append({
                "path": str(path),
                "id": path.stem,
                "updated": stat.st_mtime_ns,
                "size": stat.st_size,
            })
        return rows

    def canonicalize(self, row):
        return filesystem_reference(
            row, str(self.root), self.resolve_ref, kind="file",
        )

    def resolve_ref(self, ref):
        return ref

    def fingerprint(self, _ref):
        return "fp"

    def agent_fingerprint(self, ref):
        return self.fingerprint(ref)


def _ports(tmp_path: Path, tools: dict[str, Path]) -> EngineContext:
    adapters = {
        name: SimpleNamespace(
            browser=_FileBrowser(root),
            manifest=SimpleNamespace(source_path=str(root)),
        )
        for name, root in tools.items()
    }
    return EngineContext(
        adapter=lambda tool: adapters[tool],
        adapters=lambda: tuple(adapters),
        cache_factory=_Cache,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )


def _session_file(root: Path, name: str, content: str = "{}\n") -> Path:
    root.mkdir(parents=True, exist_ok=True)
    path = root / f"{name}.jsonl"
    path.write_text(content)
    return path


def test_snapshot_requires_bootstrap_then_serves_from_memory(tmp_path):
    root = tmp_path / "claude"
    _session_file(root, "one")
    index = AgentSessionIndex(_ports(tmp_path, {"claude": root}))
    assert index.snapshot_with_status() is None

    index.refresh()
    snapshot = index.snapshot_with_status()
    assert snapshot is not None
    tools, records, generation = snapshot
    assert tools["claude"]["ok"] is True
    assert [record.row["id"] for record in records] == ["one"]
    assert generation == 0


def test_scan_serves_snapshot_and_nudges_live_reconcile(tmp_path):
    root = tmp_path / "claude"
    _session_file(root, "one")
    ports = _ports(tmp_path, {"claude": root})
    index = AgentSessionIndex(ports)
    nudges = []
    live = SimpleNamespace(nudge=lambda: nudges.append(True))

    cold = scanning.scan(ports, index, live=live)
    assert [session["id"] for session in cold["sessions"]] == ["one"]
    assert cold["generation"] == 0
    assert not nudges  # 冷启动是阻塞全量,不必再 nudge

    warm = scanning.scan(ports, index, live=live)
    assert warm["sessions"] == cold["sessions"]
    assert nudges == [True]


def test_index_emits_deltas_after_bootstrap(tmp_path):
    root = tmp_path / "claude"
    keep = _session_file(root, "keep")
    gone = _session_file(root, "gone")
    index = AgentSessionIndex(_ports(tmp_path, {"claude": root}))
    deltas = []
    index.on_delta = deltas.append

    index.refresh()
    assert deltas == []  # bootstrap 不推增量

    index.refresh()
    assert deltas == []  # 内容没变,不推

    time.sleep(0.01)  # 保证 mtime_ns 变化
    keep.write_text('{"changed":true}\n')
    gone.unlink()
    _session_file(root, "fresh")
    index.refresh()

    assert len(deltas) == 1
    delta = deltas[0]
    assert delta["generation"] == 1
    upsert_ids = sorted(row["id"] for row in delta["upserts"])
    assert upsert_ids == ["fresh", "keep"]
    assert all(row["ref"].startswith("fsr_") for row in delta["upserts"])
    assert len(delta["removals"]) == 1
    assert index.generation == 1


def test_refresh_tool_scopes_eviction_to_that_tool(tmp_path):
    claude_root = tmp_path / "claude"
    codex_root = tmp_path / "codex"
    _session_file(claude_root, "claude-one")
    codex_gone = _session_file(codex_root, "codex-gone")
    index = AgentSessionIndex(
        _ports(tmp_path, {"claude": claude_root, "codex": codex_root}),
    )
    deltas = []
    index.on_delta = deltas.append
    records = index.refresh()
    claude_ref = next(r for r in records if r.tool == "claude").opaque_ref

    codex_gone.unlink()
    _session_file(codex_root, "codex-new")
    index.refresh_tool("codex")

    # claude 的记录不受 codex 重扫影响(scope 淘汰只作用于 codex)。
    assert index.resolve(
        "claude", claude_ref, pin_content=False,
    ).opaque_ref == claude_ref
    assert len(deltas) == 1
    assert [row["id"] for row in deltas[0]["upserts"]] == ["codex-new"]
    assert len(deltas[0]["removals"]) == 1
    tools, records, _generation = index.snapshot_with_status()
    assert {record.row["id"] for record in records} == {
        "claude-one", "codex-new",
    }
    assert tools["codex"]["count"] == 1


def test_identity_race_keeps_record_and_ref(tmp_path, monkeypatch):
    """活跃会话在哈希时被追加(身份竞态)不得被当成消失:
    不淘汰、不换发 ref、不推增量,等下一轮安静扫描收敛。"""
    from engine.sessions import index as index_module

    root = tmp_path / "claude"
    _session_file(root, "busy")
    index = AgentSessionIndex(_ports(tmp_path, {"claude": root}))
    deltas = []
    index.on_delta = deltas.append
    ref = index.refresh()[0].opaque_ref

    real_identity = index_module._path_identity
    monkeypatch.setattr(
        index_module, "_path_identity",
        lambda *args, **kwargs: (_ for _ in ()).throw(
            index_module._IdentityRaceError("会话正被追加"),
        ),
    )
    records = index.refresh()
    assert deltas == []
    assert [record.opaque_ref for record in records] == [ref]
    assert index.resolve("claude", ref, pin_content=False).opaque_ref == ref

    monkeypatch.setattr(index_module, "_path_identity", real_identity)
    index.refresh()
    assert index.resolve("claude", ref, pin_content=False).opaque_ref == ref


def test_removed_session_reappears_with_same_ref(tmp_path):
    """会话被一轮扫描判为消失后又出现(误判或原地重建):ref 必须复用,
    否则拿着旧 ref 的 UI 会闪现 reference_invalid。"""
    root = tmp_path / "claude"
    path = _session_file(root, "flappy")
    index = AgentSessionIndex(_ports(tmp_path, {"claude": root}))
    ref = index.refresh()[0].opaque_ref

    path.unlink()
    assert index.refresh() == []
    _session_file(root, "flappy")
    records = index.refresh()
    assert [record.opaque_ref for record in records] == [ref]


def test_live_service_polls_sources_and_pushes_deltas(tmp_path):
    root = tmp_path / "claude"
    _session_file(root, "one")
    index = AgentSessionIndex(_ports(tmp_path, {"claude": root}))
    deltas = []
    arrived = threading.Event()

    def capture(delta):
        deltas.append(delta)
        arrived.set()

    index.on_delta = capture
    index.refresh()
    live = LiveIndexService(
        index, poll_interval=0.05, reconcile_interval=9999,
    )
    live.start()
    try:
        time.sleep(0.2)  # 让首轮探测先把令牌基线记下来
        assert deltas == []
        _session_file(root, "two")
        assert arrived.wait(5), "轮询应发现新会话并推送增量"
    finally:
        live.stop()
    assert [row["id"] for row in deltas[0]["upserts"]] == ["two"]


def test_live_service_prefers_adapter_watch_stamp(tmp_path):
    root = tmp_path / "opencode"
    _session_file(root, "one")
    ports = _ports(tmp_path, {"opencode": root})
    browser = ports.adapter("opencode").browser
    stamps = []
    browser.watch_stamp = lambda: stamps.append(True) or "stamp-1"
    index = AgentSessionIndex(ports)
    index.refresh()
    live = LiveIndexService(
        index, poll_interval=0.05, reconcile_interval=9999,
    )
    live.start()
    try:
        deadline = time.monotonic() + 5
        while not stamps and time.monotonic() < deadline:
            time.sleep(0.02)
    finally:
        live.stop()
    assert stamps, "声明了 watch_stamp 的 adapter 应走廉价探针"


def test_tree_stamp_tracks_file_changes(tmp_path):
    root = tmp_path / "sessions"
    _session_file(root, "one")
    first = _tree_stamp(str(root))
    assert first == _tree_stamp(str(root))
    time.sleep(0.01)
    _session_file(root, "two")
    assert _tree_stamp(str(root)) != first
    assert _tree_stamp(str(tmp_path / "missing")) is None


def test_notifier_frames_follow_event_envelope():
    notifier = Notifier()
    # 未绑定时静默丢弃
    notifier.emit("sessions.changed", {"generation": 1})

    lines = []
    notifier.bind(lines.append)
    notifier.emit("sessions.changed", {"generation": 2, "upserts": []})
    frame = json.loads(lines[0])
    assert frame["type"] == "sessions.changed"
    assert frame["payload"]["generation"] == 2
    assert "id" not in frame
    assert frame["protocol"]

    with pytest.raises(ValueError):
        notifier.emit("run.started", {})  # 非引擎来源事件不得从引擎发出


def test_serve_binds_notifier_to_shared_output(tmp_path):
    from engine.server.cli import serve

    notifier = Notifier()
    output = io.StringIO()
    input_stream = io.StringIO("")  # 无请求,serve 立即返回
    serve(
        input_stream=input_stream,
        output_stream=output,
        handler=lambda request: {"ok": True},
        notifier=notifier,
    )
    notifier.emit("sessions.changed", {"generation": 1})
    frame = json.loads(output.getvalue().strip())
    assert frame["type"] == "sessions.changed"
