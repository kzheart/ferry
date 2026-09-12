# Ferry Runtime

实验性内置助手。会话、历史、角色、运行状态、桌面协议和权限由 Ferry 管理；不使用 `pi-coding-agent`、`AgentHarness` 或 pi 的 `Session` 存储体系。

## 框架与应用边界

- `pi-agent-core.Agent` 负责模型执行循环、工具参数校验、工具调度、steer/follow-up 队列及取消信号。
- `pi-ai` 负责模型、Provider、认证和请求；普通请求启用有界超时与 Provider 重试，不重放整轮工具执行。
- Skills 使用 pi 的独立加载与格式化函数；Ferry 保留导入复制、来源、角色选择及本地资源边界。技能位置指向导入副本，配套文件使用绝对路径。
- 上下文整理使用 pi 的 token 估算、压缩准备和摘要函数。临时条目仅用于调用这些函数，不接管会话。

## 历史与上下文

Engine 是 Ferry 会话的持久层。`runtime_sessions.list` 只读取摘要，`runtime_sessions.load` 在打开时读取指定会话的消息与事件。摘要不包含模型上下文检查点；失效的历史模型在实际调用时才解析，不阻止浏览历史或更换模型。

模型原始消息和聊天文本不在保存时裁剪。模型上下文使用单独的 `context_checkpoint`，恢复后将检查点与新消息组合；编辑重发清除检查点并从对应消息位置继续。压缩与生成都可以取消，无法满足上下文预算时明确失败。输出 token 上限与预留预算保持一致；估算不是精确 tokenizer。

运行在首次异步写入前占用会话，终态持久化后再开放下一次请求。写入按会话串行化，避免并发工具事件覆盖提交进度。编辑使用事件记录的消息位置，不靠 UI 消息数量推测原始历史索引。

## 工具执行

`session_search`、`session_read`、`usage` 可并行；修改、迁移、Shell 和用户交互保持串行。混合批次依照 pi 的调度语义整体串行。

取消信号通过 `tool.cancel` 传给宿主，按 session/run/request 隔离。Rust 负责原生审批、Bash 进程树终止和有界输出读取。正常完成后的待审批提案可继续批准；取消、失败和断线清理对应执行状态。

Engine 尚无正在执行任务的强制停止接口：这类任务取消后停止等待并阻止后续自动 apply，不能保证已开始的 Engine 操作立即停止。Bash 支持真实进程终止。

## 验证

```sh
npm run format:check
npm run typecheck
npm test
npm run build
npm run build:sea
```

常规测试使用离线模型。真实 Provider 集成测试单独执行，不包含在默认测试中。
