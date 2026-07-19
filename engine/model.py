"""规范化中间格式(canonical model)。

设计要点(见 README「关键决策」):
- 工具调用采用"单块"表示:input 与 output 合在一个 ToolCall 里
  (读取时完成配对),writer 各自展开为目标家的配对/单条形态。
- 每条消息保留 raw(源记录原文),保证往返可还原。
- 所有有损转换都记入 Session.loss。
"""
from dataclasses import dataclass, field


@dataclass
class ToolCall:
    name: str                    # 源工具名(Bash / exec / bash ...)
    op: str | None               # 规范操作(shell.exec 等);None = 无映射,降级
    input: dict | str            # 源参数(已解析)
    output: str                  # 结果文本
    meta: dict = field(default_factory=dict)   # exit_code 等结构化补充


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


@dataclass
class Session:
    source_tool: str
    source_id: str
    cwd: str
    title: str = ""
    messages: list[Message] = field(default_factory=list)
    loss: list[str] = field(default_factory=list)

    def lose(self, what: str):
        self.loss.append(what)
