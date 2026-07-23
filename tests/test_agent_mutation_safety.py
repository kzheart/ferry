import pytest

from engine.application import editing, services
from engine.domain.model import Session


class Target:
    def plan(self, _session):
        return {"lossless": True}

    def write(self, _session, _cwd):
        return "new-session", "/tmp/new-session"


class Adapter:
    def require(self, capability):
        if capability == "migration_target":
            return Target()
        raise AssertionError(capability)


def test_migration_cleans_artifact_when_post_write_audit_fails(monkeypatch):
    cleaned = []
    monkeypatch.setattr(services, "adapter", lambda _tool: Adapter())
    monkeypatch.setattr(
        services, "resume_command", lambda *_args: {"command": "resume"})
    monkeypatch.setattr(
        services, "validate_written_tree", lambda *_args: (True, "ok"))
    monkeypatch.setattr(
        services, "_cleanup_artifact",
        lambda tool, sid, dest: cleaned.append((tool, sid, dest)),
    )
    monkeypatch.setattr(
        services, "_append_history",
        lambda _entry: (_ for _ in ()).throw(RuntimeError("audit failed")),
    )

    session = Session("claude", "source", "/tmp/project")
    with pytest.raises(RuntimeError, match="audit failed"):
        services.migrate(
            "claude", "codex", "unused", _session=session)
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
