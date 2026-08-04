import shutil
import sqlite3
from pathlib import Path

import pytest

from engine.adapters.grok import lifecycle as grok_lifecycle
from engine.adapters.grok import probe as grok_probe
from engine.adapters.grok.writer import write as write_grok
from engine.operations import edit as editing
from engine.operations import migrate as migration
from engine.sessions.model import Block, Message, Session
from engine.system import executables


class Target:
    def plan(self, _session):
        return {"lossless": True}

    def write(self, _session, _cwd):
        return "new-session", "/tmp/new-session"


class Adapter:
    id = "fake"
    browser = object()
    migration_source = object()
    migration_target = Target()
    lifecycle = object()

    def supports(self, capability):
        return capability in {
            "browse", "resume", "migration-source", "migration-target",
        }

    def require(self, capability, component):
        if not self.supports(capability):
            raise ValueError(capability)
        return getattr(self, component)


def test_migration_cleans_artifact_when_post_write_audit_fails(monkeypatch):
    cleaned = []
    ports = type("Ports", (), {"adapter": lambda _self, _tool: Adapter()})()
    monkeypatch.setattr(
        migration.MigrationService, "__init__",
        lambda instance, _ports: setattr(instance, "_ports", ports),
    )
    monkeypatch.setattr(
        migration.MigrationService, "resume_command", lambda *_: {"command": "resume"})
    monkeypatch.setattr(
        migration.MigrationService, "validate_written_tree", lambda *_: (True, "ok"))
    monkeypatch.setattr(
        migration.MigrationService, "_cleanup_artifact",
        lambda _self, tool, sid, dest: cleaned.append((tool, sid, dest)),
    )
    monkeypatch.setattr(
        migration.history, "append",
        lambda *_: (_ for _ in ()).throw(RuntimeError("audit failed")),
    )

    session = Session("claude", "source", "/tmp/project")
    with pytest.raises(RuntimeError, match="audit failed"):
        migration.MigrationService(None).apply(
            "claude", "codex", "unused", session=session)
    assert cleaned == [("codex", "new-session", "/tmp/new-session")]


class InplaceEditor:
    name = "fake"

    def __init__(self):
        self.restored = False

    def load(self, _ref):
        return type("Doc", (), {"revision": "r1"})()

    def stats(self, _doc):
        return {"count": 1}

    def validate(self, _doc):
        pass

    def snapshot(self, _doc, extra=None):
        return "snapshot"

    def commit(self, _doc):
        return {"session_id": "source"}

    def saved_revision(self, _result, _doc):
        raise RuntimeError("write verification failed")

    def restore_snapshot(self, _snapshot, _doc):
        self.restored = True


def test_failed_inplace_verification_restores_snapshot():
    editor = InplaceEditor()

    def mutate(_doc):
        return []

    with pytest.raises(RuntimeError, match="verification failed"):
        editing.apply_mutation(
            editor, "source", mutate, expected_revision="r1")
    assert editor.restored is True


def _grok_session(tmp_path, title="mutation-sentinel"):
    return Session(
        "fixture", "source", str(tmp_path), title=title,
        messages=[Message("user", [Block("text", title)])],
    )


def _index_ids(sessions):
    database = sqlite3.connect(sessions / "session_search.sqlite")
    ids = {
        row[0] for row in database.execute(
            "SELECT session_id FROM session_docs"
        )
    }
    database.close()
    return ids


def test_grok_cleanup_removes_only_new_tree_and_its_index_rows(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle", lambda _path: {"status": "passed"},
    )
    sessions = tmp_path / "sessions"
    unrelated = sessions / "unrelated" / "existing"
    unrelated.mkdir(parents=True)
    marker = unrelated / "summary.json"
    marker.write_bytes(b'{"existing":true}\n')
    source = _grok_session(tmp_path)
    source.children = [_grok_session(tmp_path, "child-sentinel")]
    session_id, destination = write_grok(
        source, str(tmp_path), sessions,
    )
    assert len(_index_ids(sessions)) == 2

    grok_lifecycle.GrokLifecycle().cleanup(session_id, destination)

    assert marker.read_bytes() == b'{"existing":true}\n'
    assert list(
        path for path in sessions.rglob("summary.json") if path != marker
    ) == []
    assert _index_ids(sessions) == set()


def test_grok_cleanup_restores_quarantined_tree_when_index_delete_fails(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle", lambda _path: {"status": "passed"},
    )
    sessions = tmp_path / "sessions"
    source = _grok_session(tmp_path)
    source.children = [_grok_session(tmp_path, "child-sentinel")]
    session_id, destination = write_grok(
        source, str(tmp_path), sessions,
    )
    monkeypatch.setattr(
        grok_lifecycle, "delete_index_rows",
        lambda *_args: (_ for _ in ()).throw(
            RuntimeError("index delete failed")
        ),
    )

    with pytest.raises(RuntimeError, match="index delete failed"):
        grok_lifecycle.GrokLifecycle().cleanup(session_id, destination)

    assert destination.is_dir()
    assert len(list(sessions.rglob("summary.json"))) == 2
    assert _index_ids(sessions) == {
        path.parent.name for path in sessions.rglob("summary.json")
    }


def test_failed_grok_official_delete_keeps_bundle_and_index(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle", lambda _path: {"status": "passed"},
    )
    sessions = tmp_path / "sessions"
    session_id, destination = write_grok(
        _grok_session(tmp_path), str(tmp_path), sessions,
    )
    before = {
        path.name: path.read_bytes() for path in destination.iterdir()
        if path.is_file()
    }
    result = type("Result", (), {
        "returncode": 2, "stdout": "", "stderr": "delete failed",
    })()
    monkeypatch.setattr(
        grok_lifecycle.subprocess, "run",
        lambda *_args, **_kwargs: result,
    )
    browser = type("Browser", (), {
        "resolve_ref": lambda _self, _ref: str(destination),
    })()
    adapter = type("Adapter", (), {"browser": browser})()

    with pytest.raises(RuntimeError, match="delete failed"):
        grok_lifecycle.GrokLifecycle().delete(adapter, session_id)

    assert {
        path.name: path.read_bytes() for path in destination.iterdir()
        if path.is_file()
    } == before
    assert _index_ids(sessions) == {session_id}


def test_successful_grok_delete_is_permanent_and_clears_index(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle", lambda _path: {"status": "passed"},
    )
    sessions = tmp_path / "sessions"
    session_id, destination = write_grok(
        _grok_session(tmp_path), str(tmp_path), sessions,
    )

    def delete_bundle(*_args, **_kwargs):
        shutil.rmtree(destination)
        return type("Result", (), {
            "returncode": 0, "stdout": "deleted", "stderr": "",
        })()

    monkeypatch.setattr(grok_lifecycle.subprocess, "run", delete_bundle)
    browser = type("Browser", (), {
        "resolve_ref": lambda _self, _ref: str(destination),
    })()
    adapter = type("Adapter", (), {"browser": browser})()

    result = grok_lifecycle.GrokLifecycle().delete(adapter, session_id)

    assert result == {"ok": True, "session_id": session_id}
    assert not destination.exists()
    assert _index_ids(sessions) == set()


@pytest.mark.skipif(
    not Path(executables.argv("grok", "--version")[0]).exists(),
    reason="未安装 Grok Build CLI",
)
def test_current_grok_cli_permanently_deletes_isolated_bundle(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle", lambda _path: {"status": "passed"},
    )
    monkeypatch.setenv("GROK_HOME", str(tmp_path))
    sessions = tmp_path / "sessions"
    session_id, destination = write_grok(
        _grok_session(tmp_path), str(tmp_path), sessions,
    )
    browser = type("Browser", (), {
        "resolve_ref": lambda _self, _ref: str(destination),
    })()
    adapter = type("Adapter", (), {"browser": browser})()

    result = grok_lifecycle.GrokLifecycle().delete(adapter, session_id)

    assert result["ok"] is True
    assert not destination.exists()
    assert _index_ids(sessions) == set()
