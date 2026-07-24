import pytest

from engine.operations import edit as editing
from engine.operations import migrate as migration
from engine.sessions.model import Session


class Target:
    def plan(self, _session):
        return {"lossless": True}

    def write(self, _session, _cwd):
        return "new-session", "/tmp/new-session"


class Adapter:
    migration_target = Target()


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
