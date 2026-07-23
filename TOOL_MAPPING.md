# 工具映射架构

工具迁移采用单向流水线：

```text
Agent 原生记录
  → Reader 解码
  → Canonical ToolCall / ToolResult
  → MigrationTarget.evaluate_tool()
  → RenderDecision
  → Preview / Plan / Writer
  → 目标 Agent 原生记录
```

## Canonical 契约

`engine/domain/tool_ops.py` 是工具操作的唯一词表和参数 schema。目前包含：

- `shell.exec`
- `fs.read`、`fs.write`、`fs.edit`、`fs.patch`
- `fs.search`、`fs.glob`
- `web.fetch`、`web.search`
- `tool.invoke`
- `agent.spawn`

Reader 必须先规范化字段名和单位，例如 `timeout` 统一为
`timeout_ms`，但不能只保留最小必需字段。来源参数只要有明确语义，就应进入
canonical input。无法稳定映射的原生工具使用 `tool.invoke`，其中保留
`namespace`、原生工具名和原始 input；复合调用只额外记录不含参数值的结构摘要。

工具结果使用 `ToolResult`，不能只压成一个字符串。它保留：

- `status`：`success`、`error`、`interrupted`、`running`、`pending`、`unknown`
- 有序的 text / json / image / file / tool-reference blocks
- stdout、stderr、exit code、truncated
- attachments 和来源 metadata

Reader 不得从“存在 output”推断成功。来源没有明确状态时使用 `unknown`。

## 调用级判定

`MigrationTarget.evaluate_tool()` 是 plan、preview 和 write 共用的唯一判定入口，
返回 `RenderDecision`：

- `exact`：语义和结构均可原样表达
- `transformed`：语义保留，但目标工具形态改变
- `lossy`：仍写成工具调用，但部分字段或内容已损失
- `narrated`：不写成工具调用，改为明确的历史叙述
- `dropped`：目标端没有可见内容

每个非 `exact` 判定必须带 `reason_codes`。参数判定同时列出
`consumed_fields` 和 `ignored_fields`；存在 ignored fields 时不得标为 exact。
显式 `unknown` 或目标不支持的结果状态必须 narrated，不能伪造成成功。

静态 `OP_FIDELITY` 只描述目标格式的基础能力；最终保真度必须按具体调用的参数、
结果状态和会话拓扑计算。

## 三端 Reader 规则

- Claude：保留 Bash/Read/Edit/Agent 的可选参数、tool result blocks、
  interrupted/error、stdout/stderr、截断标记和 agent id 别名。
- Codex：同时接受 Responses wrapper 和旧版顶层记录；只有明确的本地 shell
  名称才映射为 `shell.exec`，远程/MCP 调用保持 `tool.invoke`；直接或 JS 包装的
  apply_patch 映射为 `fs.patch`；多个 `tools.*` 调用保持为一个复合 opaque call。
- OpenCode：保留 tool state 的 pending/running/completed/error、时间、metadata、
  attachments，以及 bash/read 的可选参数。

格式错误的单条记录应形成 loss event，并继续读取其余会话；不得因为一行损坏而
清空整个会话。

## Writer 规则

- Writer 只能消费 `RenderDecision`，不得另建一套“支持/不支持”分支。
- `decision.rendered` 为空时必须 narrated 或 dropped，并记录 loss event。
- 结果状态、退出码、截断、附件和 rich blocks 应写入目标原生字段；目标没有直接
  字段时可放入可重读的 canonical metadata，不能静默丢失。
- `shell.exec` 生成命令时必须正确 shell quote 路径和参数。
- `agent.spawn` 只有在能关联真实 child edge 时才原生写入；否则 narrated。

## 验证规则

新增或修改映射时至少覆盖：

1. canonical schema 的合法与非法输入；
2. 来源 Reader 的真实形状、旧形状和 malformed record；
3. 每个目标的 RenderDecision；
4. plan / preview / write 对同一调用的结果一致；
5. ToolResult 经目标 Writer 再 Reader 后的状态和结构化字段；
6. 复合调用的结构摘要不复制凭据值；
7. UI 能展示五级保真度及 reason code。
