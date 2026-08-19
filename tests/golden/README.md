# 黄金基线（canonical + scan）

本目录是**冻结的会话读取基线**：每个 fixture case 一份 canonical Session 快照
和一份 scanner 扫描行快照。适配器改动一旦改变读取语义，这里就会出现 diff——
要么是有意的行为变更（连同基线一起更新并在评审里说明），要么是回归。

文件全部由测试生成，**不要手工编辑**。

## 生成与校验

```bash
# 只校验（默认；CI 跑的就是这条）
cargo test -p ferry-engine --test golden_regen

# 重新生成（覆盖写入 tests/golden/**）
FERRY_GOLDEN_REGEN=1 cargo test -p ferry-engine --test golden_regen
```

再生是幂等的：连续跑两次 `git diff tests/golden/` 必须为空。

## 目录

```
tests/golden/
  canonical/<agent>/<case>.json   canonical Session 全字段快照
  scan/<agent>/<case>.json        scanner 扫描行 + 归一化说明
```

`<agent>` ∈ {claude, codex, opencode, pi, grok}，`<case>` 与
`tests/fixtures/agent_formats/<agent>/` 下的目录同名，共 13 个 case。

cursor 没有黄金基线：它的会话存储是 Cursor 自己的 SQLite 库，离线物化成本远高
于其余五家。新增 agent 一律靠自身适配器测试守语义，黄金基线只覆盖这五家的
存量 fixture。

| agent | case | 覆盖点 |
| --- | --- | --- |
| claude | case-01-plain / case-02-tools | parentUuid 链、tool_use/tool_result 配对 |
| codex | case-01-plain / case-02-tools | rollout 记录流、双 tool_call 子类型、`apply_patch` |
| opencode | case-01-plain / case-02-tools | export 形状（session/message/part 三张表）|
| pi | case-01-plain / case-02-tools / case-03-branch-compaction | v3 append-only 树、活跃分支选择、compaction |
| grok | case-01..04 | updates 为主、rewind 死分支、chat_history v1 回退 |

## canonical 文件格式约定

* **无包裹层**：文件顶层就是 `Session`，可直接反序列化成
  `crates/ferry-engine/src/model.rs` 里的 `Session`。
* **全字段**：每个字段都出现，值为 `None` 的写成 `null`，不省略。转换走 serde
  的整体序列化，不维护字段白名单，因此新增字段会自动出现在基线里并在 diff 中
  暴露。
* **递归**：`children`（子会话树）、`messages[].blocks[].tool` / `.image`、
  `tool.result`、`result.blocks[]`、`agent_edges[]`、`context_compactions[]`
  都按同样规则递归展开。
* **自由字典原样输出**：`loss[]`（事件字典）、`ToolCall.input`、
  `ToolResultBlock.data`、`ContextCompaction.metrics` / `source_meta`
  不做任何结构化改写。
* **序列化参数**：键按字典序、非 ASCII 不转义、缩进 2 空格、末尾一个换行。
  键序不表达语义。

## scan 文件格式约定

```jsonc
{
  "_normalized": {
    "sandbox_root_marker": "<home>",
    "fixed_mtime_seconds": 1784937600,
    "environment_dependent_fields": ["path", "updated", "own_updated", "size", "own_size"],
    "note": "..."
  },
  "rows": [ /* scanner 返回的行，已经过 session_roots 树装配 */ ]
}
```

* `rows` 是 scanner 的最终返回值，含 `children` 嵌套与 `own_count` /
  `own_size` / `own_updated` / `child_count` / `tree_count` 等由
  `session_roots` 补出的派生字段。
* `_normalized.environment_dependent_fields` 列出**由运行环境而非 fixture 内容
  决定**的字段。对照时这些字段应按各自环境重新计算，不要硬编码这里的值；其余
  字段应当逐字段相等。
* `path` 中的沙箱根被替换成字面量 `<home>`，保留了各家 agent 的真实存储布局，
  例如 `<home>/.claude/projects/<case>/<id>.jsonl`。
* `updated` / `own_updated` 之所以是稳定值，是因为物化 fixture 时把 mtime 统一
  钉到 `fixed_mtime_seconds`，而不是事后抹掉；grok 的 `updated` 优先取
  `summary.updated_at`，只有缺失时才回落 mtime。
* opencode 的扫描行不带文件路径（`path` 恒为 `""`、`size` 恒为 `0`），
  `updated` / `created` 来自 SQLite 的时间列，fixture 未提供时为 `0` / `null`。

## 谁在消费这份基线

| 消费者 | 覆盖 |
| --- | --- |
| `crates/ferry-engine/tests/golden_regen.rs` | 全部 26 个文件，逐字节；也是唯一的再生入口 |
| `crates/ferry-engine/tests/claude_golden.rs` | claude 的 canonical + scan |
| `crates/ferry-engine/tests/codex_golden.rs` | codex 的 canonical + scan |
| `crates/ferry-engine/tests/grok_golden.rs` | grok 的 canonical + scan |
| `crates/ferry-engine/src/adapters/pi/adapter.rs` | pi 的 canonical + scan（模块内单测）|
| `crates/ferry-engine/src/adapters/opencode/{reader,scanner}.rs` | opencode 的 canonical / scan（模块内单测）|
| `crates/ferry-engine/src/sessions/index.rs::golden_tests` | 物化 `scan/**` 供索引、搜索、agent_read 复用 |

fixture 的原生形态（各家的文件布局、opencode 的 SQLite 列、codex 的
`registration.json`）与物化方式，写在 `tests/golden_regen.rs` 里。
