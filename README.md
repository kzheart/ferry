# Ferry

跨 AI 编码 Agent 的会话互通工具:让 Claude Code、Codex CLI、OpenCode 的聊天会话可以**原生迁移**(在 A 里聊到一半,转到 B 里以原生会话形式无缝 resume)和**原地编辑**(删轮次、裁工具输出、改写)。附带 Tauri + React 桌面端(`app/`)。

## 为什么做

- 限流/配额:一家额度用完,把会话带上上下文迁到另一家继续。
- 各有所长:不同任务适合不同 Agent,切换时不丢历史。
- 会话手术:历史里的巨型工具输出、跑偏的轮次,应该能修剪后再继续。

调研结论(2026-07):已有一批同类项目(ctxmv、session-convert、ai-session-bridge 等),但全部极早期(0–40 star)、硬编码格式、无版本漂移防护。社区高星产品集中在只读浏览(agent-sessions)和单向 handoff(cli-continues)。**空档 = 高保真双向写回 + OpenCode 支持 + 格式漂移防护**。

## 核心设计

系统由三件资产构成,重要性从上到下:

1. **规格(spec/)** —— 知识核心。三家会话格式的字段级规格 + 跨家工具映射表(YAML)。做成独立于代码的公开文档,目标是成为事实标准。
2. **验证器(harness/ + golden/)** —— 让规格持续为真。受控生成的黄金样本会话 + 无头探针(写回后让目标 CLI 自己加载验收)。格式漂移在 CI 里当天暴露,而不是等用户报 issue。
3. **引擎(engine/)** —— 被规格驱动的薄转换器。reader(原生 → 中间格式)、writer(中间格式 → 原生)、编辑操作、不变量校验。

### 关键决策(务必遵守)

- **永远新建,绝不篡改**:写回一律生成新 session ID;原地编辑前先完整快照到 backups/,写入用 temp+rename 原子操作,SQLite 走事务。
- **目标工具当裁判**:写回/编辑完成后,用目标 CLI 无头加载一次刚写的会话,能 resume 才算成功;失败自动回滚。不信自己的 writer,信目标工具的 parser。
- **工具三层策略**:通用工具(bash/read/edit/write 等)按映射表**原生对齐**;各家私有工具**降级为叙述文本**;整体格式不认识时降级为 handoff 摘要。判断标准只有一条:语义是否无损保持。
- **中间格式带无损信封**:每条消息保留 `_raw` 原始 JSON,保证 A→B→A 往返可精确还原。每次转换输出损耗报告(丢了什么、截断了什么)。
- **映射表是数据不是代码**:工具映射规则写在 YAML 里,标注适用版本;格式跟进和社区贡献都只改配置。
- **编辑以"轮"为原子单位**:tool_use/tool_result 必须成对,Claude 的 uuid→parentUuid 链删除后必须重连(注意分支树:一条消息可能有多个子分支)。

## 三家存储结构速览(细节见 spec/formats/)

| 工具 | 位置 | 形态 | 写回难点 |
|---|---|---|---|
| Claude Code | `~/.claude/projects/<项目slug>/<uuid>.jsonl` | JSONL,uuid→parentUuid 链表(树) | 无索引库,目录扫描即发现;链和 tool 配对不变量 |
| Codex | `~/.codex/sessions/<Y>/<M>/<D>/rollout-*.jsonl` + `state_5.sqlite` | 时序 JSONL + SQLite 注册 | SQLite 注册;schema 版本号编在文件名里(state_5),漂移最频繁 |
| OpenCode | `~/.local/share/opencode/opencode.db` + `storage/` | SQLite(含 WAL) | 纯数据库读写,事务与外键 |

## 仓库结构(规划)

```
README.md            本文件:目标、设计决策、路线图
spec/
  formats/           三家格式的字段级规格(由黄金样本佐证)
  mapping/           跨家工具映射表 YAML
golden/              受控生成的黄金样本会话(按 工具/版本/用例 分层)
harness/             样本生成脚本 + 探针验证脚本
engine/              转换/编辑引擎;api.py 是 GUI 的结构化接口层(rpc 桥)
app/                 桌面端:Tauri v2(Rust 壳)+ React/Vite 前端
docs/                gui-features.md 功能说明;research/ 前期调研
```

## 桌面端(app/)

架构:CLI 引擎是核心,Tauri 壳只有两个 command——`engine_rpc`(把前端请求
转发给 `python3 -m engine.api rpc`)和 `open_terminal`(在 Terminal.app 执行接续命令)。
前端不含任何会话格式知识。

```
cd app && npm install        # 首次
npm run tauri dev            # 开发运行
npm run tauri build          # 打包 .app/.dmg
```

引擎仓库位置默认取 app/src-tauri 的上两级;打包分发时用环境变量
`FERRY_REPO=/path/to/resume-harness` 指定。

## 路线图

- [x] 调研现有项目与生态定位
- [x] Git 初始化 + 本文档
- [x] M1 黄金样本:三家 CLI 受控生成样本会话(纯对话 / 带工具调用 / 边界情况),含生成脚本
- [x] M2 格式规格:spec/formats/ 三家字段级文档,逐字段由样本佐证
- [x] M3 映射表:spec/mapping/tools.yaml 初稿(shell/read/edit/write/grep)
- [x] M4 探针:无头加载验收脚本(每家一个"能否 resume 这个会话"的判定器)
- [x] M5 引擎 MVP:Claude→Codex 单方向全链路(读 → 中间格式 → 写回 → 探针通过)
- [x] M6 反方向 + OpenCode + 编辑操作(delete-turn / truncate / redact / rewrite)
- [x] M7 自检:`python3 harness/ci.py [--regen]` 全链路自检(转换矩阵+探针+编辑冒烟+版本漂移告警);定时调度(cron/launchd)由使用者按需配置

## 环境基线(样本生成时的版本)

| CLI | 版本 | 路径 |
|---|---|---|
| Claude Code | 2.1.204 | `~/.local/bin/claude` |
| Codex | 0.144.0 | nvm node v24.15.0 |
| OpenCode | 1.18.3 | `~/.opencode/bin/opencode` |
