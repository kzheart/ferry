"""规范化中间格式(canonical model)。

设计要点(见 README「关键决策」):
- 工具调用采用"单块"表示:input 与 output 合在一个 ToolCall 里
  (读取时完成配对),writer 各自展开为目标家的配对/单条形态。
- 每条消息保留 raw(源记录原文),保证往返可还原。
- 所有有损转换都记入 Session.loss。
"""
from dataclasses import dataclass, field
from typing import Any


@dataclass
class RawRecord:
    source: str
    record_type: str
    payload: Any
    ordinal: int = 0
    timestamp: str | int | None = None
    location: str = ""


@dataclass
class ToolCall:
    name: str                    # 源工具名(Bash / exec / bash ...)
    op: str | None               # 规范操作(shell.exec 等);None = 无映射,降级
    input: dict | str            # 源参数(已解析)
    output: str                  # 结果文本
    meta: dict = field(default_factory=dict)   # exit_code 等结构化补充
    source_call_id: str | None = None
    source_result_id: str | None = None
    status: str | None = None
    started_at: str | int | None = None
    ended_at: str | int | None = None


@dataclass
class Block:
    kind: str                    # text | thinking | tool
    text: str = ""
    tool: ToolCall | None = None


@dataclass
class Message:
    role: str                    # user | assistant
    blocks: list[Block] = field(default_factory=list)
    raw: list = field(default_factory=list)   # 来源记录原文(可多条)
    source_id: str | None = None
    parent_ids: list[str] = field(default_factory=list)
    turn_id: str | None = None
    agent_id: str | None = None
    created_at: str | int | None = None


@dataclass
class AgentEdge:
    parent_session_id: str
    child_session_id: str
    source_call_id: str | None = None
    spawn_message_id: str | None = None
    result_message_id: str | None = None
    agent_id: str | None = None
    agent_path: str | None = None
    agent_type: str | None = None
    prompt: str = ""
    status: str | None = None
    meta: dict = field(default_factory=dict)


@dataclass
class Session:
    source_tool: str
    source_id: str
    cwd: str
    title: str = ""
    messages: list[Message] = field(default_factory=list)
    loss: list[str] = field(default_factory=list)
    root_id: str | None = None
    parent_id: str | None = None
    forked_from_id: str | None = None
    agent_id: str | None = None
    agent_path: str | None = None
    agent_type: str | None = None
    children: list["Session"] = field(default_factory=list)
    agent_edges: list[AgentEdge] = field(default_factory=list)
    raw_records: list[RawRecord] = field(default_factory=list)
    meta: dict = field(default_factory=dict)

    def lose(self, what: str):
        self.loss.append(what)

    def walk(self):
        stack = [self]
        seen = set()
        while stack:
            session = stack.pop()
            marker = id(session)
            if marker in seen:
                continue
            seen.add(marker)
            yield session
            stack.extend(reversed(session.children))

    def message_count(self) -> int:
        return sum(len(session.messages) for session in self.walk())
