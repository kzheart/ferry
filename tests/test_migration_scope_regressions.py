from types import SimpleNamespace

from engine.sessions import catalog as agent_tools
from engine.sessions.index import AgentSessionIndex
from engine.operations import migrate as migration
from engine.sessions.model import AgentEdge, Block, Message, Session


def _tree():
    root = Session("claude", "root", "/tmp/project")
    root.messages = [
        Message("user", [Block("text", "first")], source_id="root-user-1"),
        Message("assistant", [Block("text", "spawned task")],
                source_id="root-assistant-1"),
        Message("user", [Block("text", "second")], source_id="root-user-2"),
        Message("assistant", [Block("text", "second reply")],
                source_id="root-assistant-2"),
    ]
    linked = Session("claude", "linked", "/tmp/project")
    linked.messages = [Message("user", [Block("text", "task")]),
                       Message("assistant", [Block("text", "task result")])]
    nested = Session("claude", "nested", "/tmp/project")
    nested.messages = [Message("assistant", [Block("text", "nested result")])]
    linked.children = [nested]
    linked.agent_edges = [AgentEdge("linked", "nested",
                                    spawn_message_id="child-spawn")]

    unlinked = Session("claude", "unlinked", "/tmp/project")
    unlinked.messages = [Message("assistant", [Block("text", "unrelated")])]

    root.children = [linked, unlinked]
    root.agent_edges = [
        AgentEdge("root", "linked", spawn_message_id="root-assistant-1"),
        AgentEdge("root", "unlinked", spawn_message_id=None,
                  association="directory-fallback"),
    ]
    return root


def test_truncate_rounds_keeps_only_children_spawned_in_retained_root_messages():
    session = _tree()

    migration._truncate_rounds(session, 1)

    assert [message.source_id for message in session.messages] == [
        "root-user-1", "root-assistant-1",
    ]
    assert [child.source_id for child in session.children] == ["linked"]
    assert [edge.child_session_id for edge in session.agent_edges] == ["linked"]
    assert [node.source_id for node in session.walk()] == ["root", "linked", "nested"]
    assert session.message_count() == 5
    assert any(loss["code"] == "migration.children_not_migrated" and
               loss["params"] == {"count": 1} for loss in session.loss)


def test_migration_counts_include_the_retained_subtree(monkeypatch, tmp_path):
    session = _tree()
    written = []

    class Target:
        def plan(self, value):
            return {"messages": value.message_count()}

        def write(self, value, _cwd):
            written.append(value)
            return "destination-session", tmp_path / "destination"

    target = Target()
    ports = SimpleNamespace(adapter=lambda _name: SimpleNamespace(
        migration_target=target))
    monkeypatch.setattr(
        migration.MigrationService, "__init__",
        lambda instance, _ports: setattr(instance, "_ports", ports),
    )
    monkeypatch.setattr(
        migration.MigrationService, "resume_command", lambda *_: {"kind": "test"})
    monkeypatch.setattr(
        migration.MigrationService, "validate_written_tree", lambda *_: (True, "ok"))
    monkeypatch.setattr(migration.history, "append", lambda *_: None)

    result = migration.MigrationService(None).apply(
        "claude", "opencode", "ignored", cwd=str(tmp_path),
        max_turn=1, session=session,
    )

    assert written == [session]
    assert result["msg_count"] == 5
    assert result["root_msg_count"] == 2
    assert result["tree_count"] == 3


def test_preview_migration_counts_the_actual_tree_after_scope_pruning(monkeypatch):
    session = _tree()
    record = SimpleNamespace(revision="revision-1")

    class Target:
        def plan(self, value):
            return {"messages": value.message_count()}

    target = Target()
    ports = SimpleNamespace(
        adapters=lambda: ["opencode"],
        adapter=lambda _name: SimpleNamespace(migration_target=target),
    )
    index = AgentSessionIndex(ports)
    monkeypatch.setattr(index, "resolve", lambda *_: record)
    monkeypatch.setattr(agent_tools, "read_indexed_session", lambda *_: session)

    preview = agent_tools.preview_migration(
        "claude", "opaque", "opencode", max_turn=1, index=index)

    assert preview["message_count"] == 5
    assert preview["root_message_count"] == 2
    assert preview["tree_count"] == 3
