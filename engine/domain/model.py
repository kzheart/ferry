"""规范化中间格式(canonical model)。

设计要点(见 README「关键决策」):
- 工具调用采用"单块"表示:input 与 output 合在一个 ToolCall 里
  (读取时完成配对),writer 各自展开为目标家的配对/单条形态。
- 每条消息保留 raw(源记录原文),保证往返可还原。
- 所有有损转换都记入 Session.loss。
"""
import json
from dataclasses import dataclass, field
from typing import Any

from .events import event


TOOL_RESULT_STATUSES = frozenset({
    "success", "error", "interrupted", "running", "pending", "unknown",
})
TOOL_RESULT_BLOCK_KINDS = frozenset({
    "text", "json", "image", "file", "tool_reference",
})


@dataclass
class ToolResultBlock:
    """One structured block emitted by a tool result."""
    kind: str
    text: str = ""
    data: Any = None
    mime_type: str | None = None
    filename: str | None = None
    uri: str | None = None
    metadata: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.kind not in TOOL_RESULT_BLOCK_KINDS:
            raise ValueError(f"unsupported tool result block kind: {self.kind}")

    def text_projection(self) -> str:
        """Project a block to text for text-only consumers."""
        if self.text:
            return self.text
        if self.kind == "json" and self.data is not None:
            return json.dumps(self.data, ensure_ascii=False, separators=(",", ":"))
        return ""


@dataclass
class ToolResult:
    """Canonical result that retains status, streams and non-text blocks."""
    status: str = "unknown"
    blocks: list[ToolResultBlock] = field(default_factory=list)
    stdout: str | None = None
    stderr: str | None = None
    exit_code: int | None = None
    truncated: bool | None = None
    attachments: list[Any] = field(default_factory=list)
    metadata: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.status not in TOOL_RESULT_STATUSES:
            raise ValueError(
                f"unsupported canonical tool result status: {self.status}"
            )
        if isinstance(self.exit_code, bool):
            raise TypeError("exit_code must be an integer, not bool")
        if self.exit_code is not None and not isinstance(self.exit_code, int):
            raise TypeError("exit_code must be an integer or None")
        if self.truncated is not None and not isinstance(self.truncated, bool):
            raise TypeError("truncated must be a boolean or None")


def tool_result_text(result: ToolResult | None) -> str:
    """Project a structured result to text without creating another data source."""
    if result is None:
        return ""
    return "\n".join(
        text for block in result.blocks if (text := block.text_projection())
    )


def text_tool_result(text: str, *, status: str = "success", **fields) -> ToolResult:
    """Build a structured result for native tools that emit plain text."""
    blocks = [ToolResultBlock("text", text=text)] if text else []
    return ToolResult(status=status, blocks=blocks, **fields)


@dataclass
class ToolCall:
    name: str                    # 源工具名(Bash / exec / bash ...)
    op: str | None               # 规范操作(shell.exec 等);None = 无映射,降级
    input: dict | str            # 源参数(已解析)
    result: ToolResult | None = None
    source_call_id: str | None = None
    source_result_id: str | None = None
    source_message_id: str | None = None
    agent_id: str | None = None
    started_at: str | int | None = None
    ended_at: str | int | None = None

    def __post_init__(self):
        if self.result is not None and not isinstance(self.result, ToolResult):
            raise TypeError("result must be a ToolResult or None")


@dataclass
class ImageAsset:
    """规范图片块的私有源数据；DTO 仅暴露 id 与元数据。"""
    id: str
    mime_type: str
    data: str
    filename: str | None = None


@dataclass
class Block:
    kind: str                    # text | thinking | tool | image
    text: str = ""
    tool: ToolCall | None = None
    image: ImageAsset | None = None


@dataclass
class Message:
    role: str                    # user | assistant
    blocks: list[Block] = field(default_factory=list)
    source_id: str | None = None
    parent_ids: list[str] = field(default_factory=list)
    turn_id: str | None = None
    agent_id: str | None = None
    created_at: str | int | None = None


@dataclass
class ContextCompaction:
    id: str
    source: str
    after_message_id: str | None = None
    event_locator: str | None = None
    created_at: str | int | None = None
    trigger: str = "unknown"
    state: str = "completed"
    summary_status: str = "missing"
    summary_text: str = ""
    summary_message_id: str | None = None
    tail_status: str = "unknown"
    tail_start_locator: str | None = None
    tail_start_message_index: int | None = None
    metrics: dict = field(default_factory=dict)
    source_meta: dict = field(default_factory=dict)


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
    association: str = "explicit"
    confidence: float = 1.0

    def __post_init__(self):
        if not 0 <= self.confidence <= 1:
            raise ValueError("agent edge confidence must be between 0 and 1")


@dataclass
class Session:
    source_tool: str
    source_id: str
    cwd: str
    title: str = ""
    messages: list[Message] = field(default_factory=list)
    loss: list[dict] = field(default_factory=list)
    root_id: str | None = None
    parent_id: str | None = None
    forked_from_id: str | None = None
    agent_id: str | None = None
    agent_path: str | None = None
    agent_type: str | None = None
    agent_nickname: str | None = None
    agent_role: str | None = None
    model_provider: str | None = None
    model: str | None = None
    depth: int | None = None
    parent_association: str | None = None
    children: list["Session"] = field(default_factory=list)
    agent_edges: list[AgentEdge] = field(default_factory=list)
    context_compactions: list[ContextCompaction] = field(default_factory=list)

    def lose(self, code: str, **params):
        self.loss.append(event(code, **params))

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
