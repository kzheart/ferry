import json

from engine.adapters.claude.writer import write
from engine.domain.model import AgentEdge, Block, Message, Session, ToolCall
from engine.domain.tool_ops import CanonicalOp


def test_claude_child_forks_from_agent_call_after_text_in_same_message(tmp_path):
    root = Session("opencode", "root", str(tmp_path))
    root.messages = [
        Message("user", [Block("text", "start")], source_id="user-1"),
        Message("assistant", [
            Block("text", "I will delegate this."),
            Block("tool", tool=ToolCall(
                "task", CanonicalOp.AGENT_SPAWN,
                {"description": "review", "prompt": "check it",
                 "subagent_type": "general"},
                "review complete", source_call_id="call-1")),
        ], source_id="spawn-message"),
    ]
    child = Session("opencode", "child", str(tmp_path), parent_id="root")
    child.messages = [Message("assistant", [Block("text", "review complete")])]
    root.children = [child]
    root.agent_edges = [AgentEdge(
        "root", "child", source_call_id="call-1",
        spawn_message_id="spawn-message", status="completed")]

    _sid, root_path = write(root, dest_root=tmp_path / "claude")

    root_records = [json.loads(line) for line in root_path.read_text().splitlines()]
    agent_call = next(record for record in root_records
                      if record.get("type") == "assistant" and
                      any(item.get("type") == "tool_use" and
                          item.get("name") == "Agent"
                          for item in record["message"]["content"]))
    child_path = next((tmp_path / "claude" / root_path.stem /
                       "subagents").glob("agent-*.jsonl"))
    fork = json.loads(child_path.read_text().splitlines()[0])

    assert fork["type"] == "fork-context-ref"
    assert fork["parentLastUuid"] == agent_call["uuid"]


def test_claude_synthetic_missing_task_link_also_updates_fork_anchor(tmp_path):
    root = Session("opencode", "root", str(tmp_path))
    root.messages = [Message(
        "assistant", [Block("text", "delegating")], source_id="spawn-message")]
    child = Session("opencode", "child", str(tmp_path), parent_id="root")
    child.messages = [Message("assistant", [Block("text", "done")])]
    root.children = [child]
    root.agent_edges = [AgentEdge(
        "root", "child", spawn_message_id="spawn-message",
        prompt="check it", status="completed")]

    _sid, root_path = write(root, dest_root=tmp_path / "claude")

    root_records = [json.loads(line) for line in root_path.read_text().splitlines()]
    agent_call = next(record for record in root_records
                      if record.get("type") == "assistant" and
                      any(item.get("type") == "tool_use" and
                          item.get("name") == "Agent"
                          for item in record["message"]["content"]))
    child_path = next((tmp_path / "claude" / root_path.stem /
                       "subagents").glob("agent-*.jsonl"))
    fork = json.loads(child_path.read_text().splitlines()[0])

    assert fork["parentLastUuid"] == agent_call["uuid"]
