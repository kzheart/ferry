# 完整回答后的分支与续接

## 语义

选择的是用户一轮请求对应的**完整回答之后**，包含这轮工具调用及结果。
Fork 创建新的原生会话；原会话保留。新会话沿用项目目录，文件工作区不会回滚。
Ferry Resume 复制 `/ferry-resume <agent> <native-id> --through <fbp_...>`，在接收端只读取边界以内的历史。
未结束、报错、只有工具调用、缺少原生完成证据的轮次不提供入口。
分页尚未加载到回答末尾时不提前显示入口。

## 分层

- `sessions/branch_point.rs`：统一结束位置、持久化 checkpoint 和前缀摘要校验。
  Token 包含协议版本、agent、原生 session ID、排他结束下标和内容摘要，不包含路径或对话正文。
  后续追加可以继续使用；前缀被改写、删除或换了来源会拒绝。它是定位符，不是授权凭据。
- `sessions/agent_read.rs`：先截断 canonical session，再进行分页、搜索、工具结果展开。
  还会移除整场会话标题、子会话和压缩摘要等可能携带后续信息的派生数据。
  `--through` 必须在所有分页和搜索请求中保持不变；绑定不同边界的 cursor 不可混用。
- `operations/fork.rs`：请求收据、跨进程请求锁和分支来源。原生成功 ID 在刷新索引前持久化。
  同一 request ID 只执行一次；超时或崩溃留下 pending，不自动重试创建。
  收据不承担原生会话存储的职责。
- `ferry-runtime/src/native-fork/`：每个 Node/native RPC 引擎一个适配器。
  OpenCode 复用 Rust 已有的本地官方 API 客户端。
- `BranchActions.jsx`：只请求 checkpoint、发起 fork/复制指令和展示状态；不解释原生存储。
  Fork 成功后刷新并打开新会话，来源记录可返回原会话的轮次。
- 契约仍以 `contracts/*.json` 为唯一来源，运行 `scripts/generate-contracts.py` 同步。
  `native_session.fork` 是内部 runtime 方法，WebView 无权直接调用。

## 原生适配及边界

| 引擎 | 使用的原生能力 | 边界说明 |
| --- | --- | --- |
| Claude | 官方 Agent SDK `forkSession(upToMessageId)` | 包含目标 assistant message；读回验证前缀 |
| Codex | app-server `thread/fork(lastTurnId)` | 包含目标 completed turn；读回验证，防旧版忽略参数 |
| OpenCode | `POST /session/:id/fork` | `messageID` 是排他边界，传入下一条消息；末尾省略；读回验证 |
| Pi | 官方 SDK `SessionManager.createBranchedSession(answerId)` | 包含目标 entry；不启动会写配置记录的 agent RPC |
| Grok | ACP `_x.ai/session/fork(targetPromptIndex)` | 包含目标 prompt；同时校验展示历史和模型上下文 |
| Cursor | 无可调用的原生分支接口 | 仅限定范围的 Ferry Resume |

Grok 对旧版或压缩后缺少可靠 `prompt_index` 的模型上下文拒绝原生 Fork，可改用限定范围续接。
Cursor 只接受具有持久化 turn duration、无错误且有正文的最终文本 bubble；未知旧格式不猜测完成状态。
Pi SDK 版本、Claude SDK 版本固定在 package.json/lock，升级需要重跑原生测试。

## 验证

- Rust：`cargo test --manifest-path crates/ferry-engine/Cargo.toml`
- Runtime：`npm --prefix ferry-runtime run typecheck`、`npm --prefix ferry-runtime test`
- UI：`npm --prefix app test`、`npm --prefix app run typecheck`、`npm --prefix app run check:boundaries`
- 生成契约：`python3 scripts/generate-contracts.py --check`
- 本机 OpenCode 集成：`FERRY_TEST_NATIVE_FORKS=1 pytest tests/test_native_fork_integration.py`（先构建引擎）
- 单文件分发：`npm --prefix ferry-runtime run build:sea`

2026-09-26 在隔离的合成会话中验证了 Claude SDK 0.3.283、Codex 0.157.0、
OpenCode 1.18.32、Pi SDK 0.81.1、Grok 1.0.41 的完整回答边界、后续排除和原会话保留。
Claude/Codex/Pi 同时经过 SEA 打包运行测试。没有调用模型生成答案或修改用户的真实会话。
