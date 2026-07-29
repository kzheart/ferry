import json
import os
import threading
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

from engine.adapters.contracts import (
    NativeSessionReference,
    filesystem_reference,
    id_reference,
)
from engine.context import EngineContext
from engine.contracts import session_ref
from engine.contracts.session_ref import is_opaque_session_ref
from engine.errors import AgentReferenceError
from engine.sessions.index import AgentSessionIndex


ROOT = Path(__file__).resolve().parents[1]


def test_session_ref_contract_is_generated_for_every_runtime():
    """契约源(JSON)必须与生成的 Python 常量同步,各 runtime 的产物必须存在。"""
    contract = json.loads((ROOT / "contracts/session-ref.json").read_text())
    assert contract["opaque_prefix"] == session_ref.OPAQUE_SESSION_REF_PREFIX
    assert contract["minimum_length"] == session_ref.OPAQUE_SESSION_REF_MIN_LENGTH
    assert contract["maximum_length"] == session_ref.OPAQUE_SESSION_REF_MAX_LENGTH
    for path in (
        "app/src/shared/contracts/generated/session-ref.ts",
        "app/src-tauri/src/contracts/session_ref.rs",
        "engine/contracts/session_ref.py",
        "ferry-runtime/src/server/generated/session-ref.ts",
    ):
        assert (ROOT / path).is_file()


def test_opaque_session_ref_uses_one_strict_shape():
    assert is_opaque_session_ref("fsr_valid")
    assert is_opaque_session_ref("fsr_a-b_C9")
    assert not is_opaque_session_ref("native-session-id")
    assert not is_opaque_session_ref("fsr_bad/path")
    assert not is_opaque_session_ref("fsr_\nsecret")
    assert not is_opaque_session_ref("fsr_" + "a" * 125)
    with pytest.raises(ValueError):
        NativeSessionReference("id", "/tmp", "id")
    with pytest.raises(ValueError):
        NativeSessionReference("id", None, "unknown")


def test_filesystem_reference_supports_scoped_files_and_directories(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    source = root / "session.jsonl"
    source.write_text("{}\n")
    bundle = root / "bundle"
    bundle.mkdir()
    (bundle / "summary.json").write_text("{}\n")
    def resolve(value):
        return value

    file_ref = filesystem_reference(
        {"path": str(source)}, str(root), resolve, kind="file",
    )
    directory_ref = filesystem_reference(
        {"path": str(bundle)},
        str(root),
        resolve,
        kind="directory",
        required_name="summary.json",
    )
    assert file_ref.storage_kind == "file"
    assert directory_ref.storage_kind == "directory"
    assert id_reference({"id": "native-id"}).storage_kind == "id"

    outside = tmp_path / "outside.jsonl"
    outside.write_text("{}\n")
    (root / "escape.jsonl").symlink_to(outside)
    assert filesystem_reference(
        {"path": str(root / "escape.jsonl")},
        str(root),
        resolve,
        kind="file",
    ) is None
    assert filesystem_reference(
        {"path": str(bundle)},
        str(root),
        resolve,
        kind="directory",
        required_name="missing.json",
    ) is None
    (bundle / "summary.json").unlink()
    (bundle / "summary.json").symlink_to(outside)
    assert filesystem_reference(
        {"path": str(bundle)},
        str(root),
        resolve,
        kind="directory",
        required_name="summary.json",
    ) is None


class _Cache:
    def flush(self):
        pass


class _DirectoryBrowser:
    def __init__(self, root: Path, bundle: Path):
        self.root = root
        self.bundle = bundle

    def scan(self, _cache):
        return [{
            "path": str(self.bundle),
            "id": "bundle-id",
            "updated": 1,
            "size": 1,
        }]

    def canonicalize(self, row):
        return filesystem_reference(
            row,
            str(self.root),
            self.resolve_ref,
            kind="directory",
            required_name="summary.json",
        )

    def resolve_ref(self, ref):
        return ref

    def fingerprint(self, _ref):
        return "bundle-v1"

    def agent_fingerprint(self, ref):
        return self.fingerprint(ref)

    def authoritative_members(self, _ref):
        return ["summary.json"]


def test_directory_pinned_read_expires_when_authoritative_member_changes(
        tmp_path):
    root = tmp_path / "sessions"
    bundle = root / "bundle"
    bundle.mkdir(parents=True)
    summary = bundle / "summary.json"
    summary.write_text('{"title":"before"}\n')
    browser = _DirectoryBrowser(root, bundle)
    adapter = SimpleNamespace(browser=browser)
    ports = EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ("grok",),
        cache_factory=_Cache,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )
    index = AgentSessionIndex(ports)
    record = index.refresh()[0]
    assert record.storage_kind == "directory"
    assert index.resolve("grok", record.opaque_ref) == record

    original_stat = summary.stat()
    summary.write_text('{"title":"after!"}\n')
    summary.touch()
    summary.chmod(original_stat.st_mode)
    os.utime(
        summary,
        ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns),
    )
    with pytest.raises(AgentReferenceError, match="扫描后已变化"):
        index.resolve("grok", record.opaque_ref)

    # ref 是稳定句柄:重扫后不换发,同一 ref 解析到 revision 更新后的记录。
    current = index.refresh()[0]
    assert current.opaque_ref == record.opaque_ref
    assert current.revision != record.revision
    assert index.resolve("grok", current.opaque_ref) == current

    sidecar = bundle / "events.jsonl"
    sidecar.write_text('{"ignored":true}\n')
    unchanged = index.refresh()[0]
    assert unchanged.opaque_ref == current.opaque_ref
    sidecar.write_text('{"ignored":false}\n')
    assert index.refresh()[0].opaque_ref == current.opaque_ref

    summary.unlink()
    with pytest.raises(AgentReferenceError):
        index.resolve("grok", current.opaque_ref)


def test_concurrent_refreshes_coalesce_into_one_scan(tmp_path):
    """启动预热与 UI 首扫并发时,全量扫库只应真正执行一次。"""
    root = tmp_path / "sessions"
    bundle = root / "bundle"
    bundle.mkdir(parents=True)
    (bundle / "summary.json").write_text('{"title":"one"}\n')
    browser = _DirectoryBrowser(root, bundle)
    scans = []
    gate = threading.Event()
    original_scan = browser.scan

    def slow_scan(cache):
        scans.append(1)
        gate.wait(timeout=5)
        return original_scan(cache)

    browser.scan = slow_scan
    adapter = SimpleNamespace(browser=browser)
    ports = EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ("grok",),
        cache_factory=_Cache,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )
    index = AgentSessionIndex(ports)

    results = []
    barrier = threading.Barrier(3)

    def worker_main():
        barrier.wait(timeout=5)
        results.append(index.refresh())

    workers = [threading.Thread(target=worker_main) for _ in range(3)]
    for worker in workers:
        worker.start()
    # 等首个线程真正进入扫描、其余线程加入飞行后再放行。
    for _ in range(500):
        if scans:
            break
        time.sleep(0.01)
    time.sleep(0.1)
    gate.set()
    for worker in workers:
        worker.join(timeout=5)

    assert len(scans) == 1
    assert len(results) == 3
    refs = {records[0].opaque_ref for records in results}
    assert len(refs) == 1
