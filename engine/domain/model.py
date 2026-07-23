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

_TOOL_RESULT_STATUS_ALIASES = {
    "complete": "success",
    "completed": "success",
    "failed": "error",
    "failure": "error",
    "cancelled": "interrupted",
    "canceled": "interrupted",
}


def normalize_tool_result_status(value: str | None) -> str:
    """Map native/legacy result states into the canonical status vocabulary."""
    if not isinstance(value, str):
        return "unknown"
    normalized = value.strip().lower()
    normalized = _TOOL_RESULT_STATUS_ALIASES.get(normalized, normalized)
    return normalized if normalized in TOOL_RESULT_STATUSES else "unknown"


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

    def legacy_text(self) -> str:
        """Return the textual compatibility view used by legacy writers."""
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
        original_status = self.status
        self.status = normalize_tool_result_status(self.status)
        if (
            isinstance(original_status, str)
            and original_status.strip()
            and self.status == "unknown"
            and original_status.strip().lower() != "unknown"
        ):
            self.metadata.setdefault("source_status", original_status)
        if isinstance(self.exit_code, bool):
            raise TypeError("exit_code must be an integer, not bool")
        if self.exit_code is not None and not isinstance(self.exit_code, int):
            raise TypeError("exit_code must be an integer or None")
        if self.truncated is not None and not isinstance(self.truncated, bool):
            raise TypeError("truncated must be a boolean or None")

    @classmethod
    def from_legacy(
        cls,
        output: str | None,
        *,
        status: str | None = None,
        metadata: dict | None = None,
    ) -> "ToolResult":
        """Construct a structured result without losing a legacy output/meta pair."""
        legacy_meta = dict(metadata or {})
        text = output or ""
        blocks = [ToolResultBlock("text", text=text)] if text else []
        exit_code = legacy_meta.get("exit_code", legacy_meta.get("exit"))
        if isinstance(exit_code, bool) or not isinstance(exit_code, int):
            exit_code = None
        truncated = legacy_meta.get("truncated")
        if not isinstance(truncated, bool):
            truncated = None
        attachments = legacy_meta.get("attachments")
        if not isinstance(attachments, list):
            attachments = []
        stdout = legacy_meta.get("stdout")
        stderr = legacy_meta.get("stderr")
        return cls(
            status=status or "unknown",
            blocks=blocks,
            stdout=stdout if isinstance(stdout, str) else None,
            stderr=stderr if isinstance(stderr, str) else None,
            exit_code=exit_code,
            truncated=truncated,
            attachments=list(attachments),
            metadata=legacy_meta,
        )

    def legacy_output(self) -> str:
        """Flatten text/JSON blocks for adapters that still consume a string."""
        return "\n".join(
            text for block in self.blocks if (text := block.legacy_text())
        )


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
    output: str | ToolResult     # 结果文本；也接受新的结构化结果
    meta: dict = field(default_factory=dict)   # exit_code 等结构化补充
    source_call_id: str | None = None
    source_result_id: str | None = None
    status: str | None = None
    started_at: str | int | None = None
    ended_at: str | int | None = None
    result: ToolResult | None = None

    def __post_init__(self):
        if isinstance(self.output, ToolResult):
            if self.result is not None and self.result is not self.output:
                raise ValueError("output and result contain different ToolResult values")
            self.result = self.output
            self.output = self.result.legacy_output()
        elif self.output is None:
            self.output = ""
        elif not isinstance(self.output, str):
            self.output = str(self.output)

        if self.result is not None:
            self.output = self.result.legacy_output()
            if self.status is None:
                self.status = self.result.status

    @property
    def tool_result(self) -> ToolResult:
        """Return an explicit result or a fresh view of the current legacy fields."""
        if self.result is not None:
            return self.result
        return ToolResult.from_legacy(
            self.output, status=self.status, metadata=self.meta,
        )

    def set_result(self, result: ToolResult) -> None:
        """Install a structured result and keep legacy consumers operational."""
        if not isinstance(result, ToolResult):
            raise TypeError("result must be a ToolResult")
        self.result = result
        self.output = result.legacy_output()
        self.status = result.status


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
    children: list["Session"] = field(default_factory=list)
    agent_edges: list[AgentEdge] = field(default_factory=list)
    raw_records: list[RawRecord] = field(default_factory=list)
    meta: dict = field(default_factory=dict)

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
