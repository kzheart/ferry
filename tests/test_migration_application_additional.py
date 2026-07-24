from types import SimpleNamespace

import pytest

from engine.operations import migrate as migration
from engine.sessions.model import AgentEdge, Block, Message, Session


def _scoped_tree():
    root = Session("claude", "root", "/tmp/project")
    root.messages = [
        Message("user", [Block("text", "first")], source_id="user-1"),
        Message("assistant", [Block("text", "spawn")], source_id="assistant-1"),
        Message("user", [Block("text", "second")], source_id="user-2"),
    ]
    linked = Session("claude", "linked", "/tmp/project")
    linked.messages = [Message("assistant", [Block("text", "result")])]
    duplicate = Session("claude", "linked", "/tmp/project")
    duplicate.messages = [Message("assistant", [Block("text", "duplicate")])]
    unlinked = Session("claude", "unlinked", "/tmp/project")
    unlinked.messages = [Message("assistant", [Block("text", "orphan")])]
    root.children = [linked, duplicate, unlinked]
    root.agent_edges = [
        AgentEdge("root", "linked", spawn_message_id="assistant-1"),
        AgentEdge("root", "linked", spawn_message_id="assistant-1"),
        AgentEdge("root", "unlinked", spawn_message_id=None),
    ]
    return root


def _migrate_target(write):
    class Target:
        def plan(self, session):
            return {"messages": session.message_count()}

        def preview(self, session, _cwd=None):
            return self.plan(session)

        def write(self, session, cwd):
            return write(session, cwd)

    return Target()


def _install_target(monkeypatch, target):
    ports = SimpleNamespace(adapter=lambda _name: SimpleNamespace(
        migration_target=target))
    monkeypatch.setattr(
        migration.MigrationService, "__init__",
        lambda instance, _ports: setattr(instance, "_ports", ports),
    )
    monkeypatch.setattr(
        migration.MigrationService, "resume_command",
        lambda *_: {"kind": "test"},
    )
    monkeypatch.setattr(migration.history, "append", lambda *_: None)


def test_truncate_boundary_deduplicates_children_and_edges():
    session = _scoped_tree()

    migration._truncate_rounds(session, 1)

    assert [message.source_id for message in session.messages] == ["user-1", "assistant-1"]
    assert [child.source_id for child in session.children] == ["linked"]
    assert [edge.child_session_id for edge in session.agent_edges] == ["linked"]
    assert session.message_count() == 3


def test_truncate_requires_a_nonempty_message_source_id_for_spawn_link():
    root = Session("claude", "root", "/tmp/project")
    root.messages = [Message("user", [Block("text", "first")], source_id="")]
    child = Session("claude", "child", "/tmp/project")
    child.messages = [Message("assistant", [Block("text", "result")])]
    root.children = [child]
    root.agent_edges = [AgentEdge("root", "child", spawn_message_id="")]

    migration._truncate_rounds(root, 1)

    assert root.children == []
    assert root.agent_edges == []
    assert root.message_count() == 1


def test_preview_and_apply_report_identical_scope_counts(monkeypatch, tmp_path):
    writes = []
    target = _migrate_target(lambda session, _cwd: (
        writes.append(session) or ("written", tmp_path / "written")))
    _install_target(monkeypatch, target)
    monkeypatch.setattr(
        migration.MigrationService, "validate_written_tree", lambda *_: (True, "ok"))

    preview = migration.MigrationService(None).preview(
        "claude", "opencode", "ignored", cwd=str(tmp_path),
        max_turn=1, session=_scoped_tree(),
    )
    actual = migration.MigrationService(None).apply(
        "claude", "opencode", "ignored", cwd=str(tmp_path),
        max_turn=1, session=_scoped_tree(),
    )

    assert writes
    assert (
        preview["msg_count"],
        preview["root_msg_count"],
        preview["tree_count"],
    ) == (
        actual["msg_count"], actual["root_msg_count"], actual["tree_count"])


def test_validation_failure_rolls_back_the_written_artifact(monkeypatch, tmp_path):
    target = _migrate_target(lambda _session, _cwd: ("written", tmp_path / "written"))
    _install_target(monkeypatch, target)
    removed = []
    monkeypatch.setattr(
        migration.MigrationService, "validate_written_tree", lambda *_: (False, "invalid"))
    monkeypatch.setattr(
        migration.MigrationService, "_cleanup_artifact", lambda _self, _dst, sid, _dest:
        removed.append(sid))

    result = migration.MigrationService(None).apply(
        "claude", "opencode", "ignored", cwd=str(tmp_path),
        session=_scoped_tree(),
    )

    assert result["rolled_back"] is True
    assert removed == ["written"]


def test_probe_shadow_write_failure_restores_the_original_loss_state(monkeypatch):
    session = Session("claude", "root", "/tmp/project")
    target = _migrate_target(lambda value, _cwd: (
        value.lose("probe.shadow_write"),
        (_ for _ in ()).throw(RuntimeError("shadow write failed")),
    )[1])
    instance = migration.MigrationService(SimpleNamespace(
        adapter=lambda _name: SimpleNamespace(migration_target=target)))

    with pytest.raises(RuntimeError, match="shadow write failed"):
        instance._isolated_probe("opencode", session, "/tmp/project")

    assert session.loss == []


def test_probe_exception_cleans_both_shadow_and_actual_artifacts(monkeypatch, tmp_path):
    session = _scoped_tree()
    calls = []
    target = _migrate_target(lambda _session, _cwd: (
        calls.append("write") or
        ("actual" if len(calls) == 1 else "shadow", tmp_path / calls[-1])))
    _install_target(monkeypatch, target)
    removed = []
    monkeypatch.setattr(
        migration.MigrationService, "validate_written_tree", lambda *_: (True, "ok"))
    monkeypatch.setattr(migration.MigrationService, "run_probe", lambda *_args, **_kwargs:
                        (_ for _ in ()).throw(RuntimeError("probe failed")))
    monkeypatch.setattr(
        migration.MigrationService, "_cleanup_artifact", lambda _self, _dst, sid, _dest:
        removed.append(sid))

    with pytest.raises(RuntimeError, match="probe failed"):
        migration.MigrationService(None).apply(
            "claude", "opencode", "ignored", cwd=str(tmp_path),
            probe=True, session=session,
        )

    assert removed == ["shadow", "actual"]


def test_preview_reports_same_scope_counts_as_migration():
    session = _scoped_tree()
    target = _migrate_target(lambda *_: (_ for _ in ()).throw(AssertionError()))
    ports = SimpleNamespace(
        adapter=lambda _name: SimpleNamespace(migration_target=target),
    )
    preview = migration.MigrationService(ports).preview(
        "claude",
        "opencode",
        "ignored",
        max_turn=1,
        session=session,
    )

    assert (preview["msg_count"], preview["root_msg_count"],
            preview["tree_count"]) == (3, 2, 2)
