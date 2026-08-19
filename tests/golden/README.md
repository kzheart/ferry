# 黄金基线（canonical + scan）

本目录是 **Python 引擎产出的行为基准**，供 Rust 引擎重写
（`docs/rust-engine-refactor-plan.md` §WP-G / §4）逐字段对照。迁移期内
`engine/` 保持原样、保持绿灯，这里的 JSON 就是「正确答案」。

文件全部由脚本生成，**不要手工编辑**。

## 生成与校验

```bash
# 重新生成（覆盖写入 tests/golden/**）
python3 scripts/dump-canonical-fixtures.py

# 只校验现有文件是否仍与当前 Python 引擎一致（CI 友好，退出码非 0 表示漂移）
python3 scripts/dump-canonical-fixtures.py --check
```

脚本是幂等的：连续运行两次 `git diff` 必须为空。若出现 diff，说明 Python 引擎
的读取语义发生了变化——要么是有意的行为变更（连同 Rust 侧一起更新），要么是回归。

## 目录

```
tests/golden/
  canonical/<agent>/<case>.json   canonical Session 全字段快照
  scan/<agent>/<case>.json        scanner 扫描行 + 归一化说明
```

`<agent>` ∈ {claude, codex, opencode, pi, grok}，`<case>` 与
`tests/fixtures/agent_formats/<agent>/` 下的目录同名，共 13 个 case。

cursor 是 Rust-only agent（只在 `crates/ferry-engine/` 里实现，Python 参考
引擎经 `RUST_ONLY_AGENTS` 豁免），因此这里没有它的黄金基线——黄金文件的作用
是对照两套实现，只有一套实现时无从对照。cursor 的读取语义由 Rust 侧自己的
适配器测试守住。

| agent | case | 覆盖点 |
| --- | --- | --- |
| claude | case-01-plain / case-02-tools | parentUuid 链、tool_use/tool_result 配对 |
| codex | case-01-plain / case-02-tools | rollout 记录流、双 tool_call 子类型、`apply_patch` |
| opencode | case-01-plain / case-02-tools | export 形状（session/message/part 三张表）|
| pi | case-01-plain / case-02-tools / case-03-branch-compaction | v3 append-only 树、活跃分支选择、compaction |
| grok | case-01..04 | updates 为主、rewind 死分支、chat_history v1 回退 |

## canonical 文件格式约定

Rust 侧黄金测试按下面这套约定对照：

* **无包裹层**：文件顶层就是 `Session`，可直接反序列化成 Rust 的 `Session`
  struct。
* **全字段**：dataclass 的每个字段都出现，值为 `None` 的写成 `null`，不省略。
  转换是通用的（走 `dataclasses.fields()`），不维护字段白名单，因此 Python 侧
  新增字段会自动出现在黄金文件里并在 diff 中暴露。
* **递归**：`children`（子会话树）、`messages[].blocks[].tool` / `.image`、
  `tool.result`、`result.blocks[]`、`agent_edges[]`、`context_compactions[]`
  都按同样规则递归展开。
* **自由字典原样输出**：`loss[]`（事件字典）、`ToolCall.input`、
  `ToolResultBlock.data`、`ContextCompaction.metrics` / `source_meta`
  不做任何结构化改写。
* **序列化参数**：`sort_keys=True`、`ensure_ascii=False`、`indent=2`、末尾一个
  换行。键序按字典序，不表达语义；非 ASCII 保留字面量，按 UTF-8 读取。

对应的 Python 结构定义在 `engine/sessions/model.py`（`Session` / `Message` /
`Block` / `ToolCall` / `ToolResult` / `ToolResultBlock` / `ImageAsset` /
`AgentEdge` / `ContextCompaction`）。

## scan 文件格式约定

```jsonc
{
  "_normalized": {
    "sandbox_root_marker": "<home>",
    "fixed_mtime_seconds": 1784937600,
    "environment_dependent_fields": ["path", "updated", "own_updated", "size", "own_size"],
    "note": "..."
  },
  "rows": [ /* scanner 返回的行，已经过 sessions.topology.session_roots 装配 */ ]
}
```

* `rows` 是 scanner 的最终返回值，含 `children` 嵌套与 `own_count` /
  `own_size` / `own_updated` / `child_count` / `tree_count` 等由
  `session_roots` 补出的派生字段。
* `_normalized.environment_dependent_fields` 列出**由运行环境而非 fixture 内容
  决定**的字段。Rust 侧在真实环境对照时，这些字段应按各自环境重新计算，不要
  硬编码这里的值；其余字段应当逐字段相等。
* `path` 中的沙箱根被替换成字面量 `<home>`，保留了各家 agent 的真实存储布局，
  例如 `<home>/.claude/projects/<case>/<id>.jsonl`。
* `updated` / `own_updated` 之所以是稳定值，是因为生成脚本把物化 fixture 的
  mtime 统一钉到 `fixed_mtime_seconds`，而不是事后抹掉；grok 的 `updated`
  优先取 `summary.updated_at`，只有缺失时才回落 mtime。
* opencode 的扫描行不带文件路径（`path` 恒为 `""`、`size` 恒为 `0`），
  `updated` / `created` 来自 SQLite 的时间列，fixture 未提供时为 `0` / `null`。

## Rust 侧如何消费

1. 把 `tests/golden/**` 作为 crate 的测试数据读入（`include_str!` 或运行时按
   路径读均可），用 `serde_json::Value` 或直接反序列化成 canonical struct。
2. 用与 Python 同源的 fixture（`tests/fixtures/agent_formats/<agent>/<case>/`）
   跑 Rust 侧 reader，把结果序列化成同一形状后与黄金文件比对：
   * 序列化参数保持一致（键排序、UTF-8 不转义、`None` 显式为 `null`）；
   * 比对建议用 `serde_json::Value` 的相等性而不是字符串相等，避免浮点格式与
     缩进差异干扰。
3. scan 对照：先按 `_normalized.environment_dependent_fields` 把双方对应字段
   抹平（或按本机环境重算），再比对剩余字段。
4. 适配器工作包 C1..C5 各自只需关心 `canonical/<自己的 agent>/` 与
   `scan/<自己的 agent>/`，彼此无耦合。

fixture 的原生形态（各家的文件布局、opencode 的 SQLite 列、codex 的
`registration.json`）与脚本的物化方式，都写在
`scripts/dump-canonical-fixtures.py` 的 docstring 里。
