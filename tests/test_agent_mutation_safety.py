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


class CopyEditor:
    name = "fake"

    def __init__(self):
        self.discarded = False

    def load(self, _ref):
        return type("Doc", (), {"revision": "r1"})()

    def stats(self, _doc):
        return {"count": 1}

    def validate(self, _doc):
        pass

    def save_copy(self, _doc):
        return {"session_id": "copy"}

    def saved_revision(self, _result, _doc):
        raise RuntimeError("write verification failed")

    def discard(self, result):
        self.discarded = True


def test_failed_save_copy_verification_discards_artifact():
    editor = CopyEditor()

    def mutate(_doc):
        return []

    with pytest.raises(RuntimeError, match="verification failed"):
        editing.apply_mutation(editor, "source", mutate, save_as=True,
                               expected_revision="r1")
    assert editor.discarded is True
