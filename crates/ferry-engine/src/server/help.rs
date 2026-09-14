//! CLI 参数定义同时用于解析与离线帮助，避免新增参数只出现在某一份文档中。

use serde_json::{json, Value};

use super::args::{self, Parsed};

pub struct OptionSpec {
    pub name: &'static str,
    pub value: Option<&'static str>,
    pub description: &'static str,
}

macro_rules! value {
    ($name:literal, $value:literal, $description:literal) => {
        OptionSpec {
            name: $name,
            value: Some($value),
            description: $description,
        }
    };
}

macro_rules! switch {
    ($name:literal, $description:literal) => {
        OptionSpec {
            name: $name,
            value: None,
            description: $description,
        }
    };
}

pub const SEARCH_OPTIONS: &[OptionSpec] = &[
    value!("agent", "a,b", "按 agent 过滤"),
    value!("project", "PATH", "会话工作目录精确匹配，可重复"),
    value!("session-id", "ID", "原生会话 ID 精确匹配，可重复"),
    value!("since", "TIME", "UTC 时间、7d/24h 或 @epoch毫秒"),
    value!("until", "TIME", "UTC 截止时间"),
    value!("limit", "N", "每页 1–50 个会话，默认 20"),
    value!("pattern", "TEXT", "多个 pattern 按 OR 合并，可重复"),
    value!("scope", "metadata|content|any", "默认 any"),
    value!("cursor", "TOKEN", "使用上一页 next_cursor 继续"),
    switch!("regex", "位置参数为正则表达式"),
    switch!("exhaustive", "与 --regex 同用，跳过索引预筛；仍有扫描预算"),
    switch!("tool-outputs", "检索工具输出"),
];

pub const READ_OPTIONS: &[OptionSpec] = &[
    value!("from", "N", "起始原始消息号，1-based"),
    value!("limit", "N", "每页 1–50 条消息或命中，默认 20"),
    value!("roles", "user,assistant", "仅 --terms 搜索模式按角色过滤"),
    value!("terms", "a,b", "搜索消息中的关键词"),
    value!("max-bytes", "N", "每页预算 1024–65536 字节，默认 24576"),
    value!(
        "cursor",
        "TOKEN",
        "使用上一页 next_cursor 继续，含长消息分片"
    ),
    switch!("tool-outputs", "包含工具输出正文"),
    switch!("inert", "移除可识别脚手架，历史仅作为证据"),
];

pub const TITLE_OPTIONS: &[OptionSpec] = &[
    value!(
        "file",
        "PATH",
        "apply 读改名清单；style 读风格 JSON，`-` 表示 stdin"
    ),
    switch!("include-manual", "连 title_source=manual 的会话一起重命名"),
    switch!("apply", "reset 专用：把生成的标题写回原 agent"),
];

pub fn parse_options(argv: &[String], options: &[OptionSpec]) -> Result<Parsed, String> {
    let values: Vec<_> = options
        .iter()
        .filter(|item| item.value.is_some())
        .map(|item| item.name)
        .collect();
    let switches: Vec<_> = options
        .iter()
        .filter(|item| item.value.is_none())
        .map(|item| item.name)
        .collect();
    args::parse(argv, &values, &switches)
}

struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    options: &'static [OptionSpec],
    notes: &'static str,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "search", usage: "ferry search [query...] [flags]", description: "跨 agent 搜索或列出会话", options: SEARCH_OPTIONS,
        notes: "一个 pattern 的词必须在同一条消息内匹配；多个 --pattern 表达 OR。\n引号表示短语；不支持裸 AND/OR/NOT。下一页沿用查询条件并传 --cursor。\n相对时间续页改用 resolved_time_range 的 @epoch毫秒，避免窗口随时钟移动。\n检查 coverage 与 next_cursor；索引 ready 不等于查询完整。游标失效时重新搜索。",
    },
    CommandSpec {
        name: "read", usage: "ferry read <tool> <ref> [flags]", description: "分页读取会话或定位关键词", options: READ_OPTIONS,
        notes: "--terms 开启搜索模式；--from 不用于搜索分页，使用 --cursor。\nnext_cursor 支持消息与 block 分片续读；kind=fragment 的 text 拼接后按 JSON 解析。\n原始消息编号不变。origin 与 duplicate_key 是审计提示，重复候选不等于同一事件。\n游标绑定 revision 和读取条件；context 续页保留最初 --from，允许调整 limit/max-bytes。",
    },
    CommandSpec { name: "usage", usage: "ferry usage [--agent a,b] [--project PATH] [--since TIME] [--until TIME]", description: "Token 与估算费用", options: &[], notes: "费用是估算值；未定价模型不计费用。项目路径精确匹配，时间为 UTC。" },
    CommandSpec { name: "resume", usage: "ferry resume <tool> <ref>", description: "返回在原 agent 续聊的终端命令", options: &[], notes: "此命令只返回描述，不执行目标 agent。跨 agent 接续使用 ferry-resume skill。" },
    CommandSpec { name: "migrate", usage: "ferry migrate plan <tool> <ref> --to <target> [--max-turn N] [--full]\nferry migrate apply <plan_id>\nferry migrate status <plan_id>\nferry migrate cancel <plan_id>", description: "预览、执行和查询原生迁移", options: &[], notes: "plan 不写源会话；检查影响并得到明确确认后 apply。计划十分钟过期。" },
    CommandSpec { name: "rename", usage: "ferry rename <tool> <ref> <title...> [--plan]", description: "改会话标题并写回原 agent 的存储", options: &[], notes: "支持 claude/codex/opencode/pi/grok；cursor 只读，请用桌面端的本地重命名。\n默认直接执行；--plan 只打印 before/after 预览。结果 native.notes 说明对方是否需重启才显示新标题。" },
    CommandSpec {
        name: "title",
        usage: "ferry title evidence <tool> <ref>... [<tool> <ref>...]\nferry title suggest <tool> <ref>... [--include-manual]\nferry title reset <tool> <ref>... [--include-manual] [--apply]\nferry title apply --file <path|->\nferry title style [--file <path>]",
        description: "用 AI 按用户风格重置会话标题",
        options: TITLE_OPTIONS,
        notes: "位置参数按 agent 分组：命中 agent id 的 token 切换当前 agent，其余都是它的 ref。\nevidence 只取证据（标题、前 3 条用户消息、最后一条助手回复、涉及文件）供你自己命名。\nsuggest/reset 会拉起本地 Ferry Runtime 调模型，未配置模型时报 provider_unavailable。\n默认跳过 title_source=manual 的会话；消息数 < 3 的由生成器标 skip。\nreset 不带 --apply 只打印预览；--apply 与 apply 逐条写回，claude/codex/opencode/pi/grok 写原生存储，cursor 写 Ferry 本地 name。\napply 的 --file 是 [{\"tool\",\"ref\",\"title\"}] 数组。任一条写回失败退出码为 1。",
    },
    CommandSpec { name: "scan", usage: "ferry scan [--wait] [--timeout SEC] [--full]", description: "刷新会话索引", options: &[], notes: "--wait 等待内容索引就绪，默认超时 600 秒；--full 原始全库 DTO 无界。" },
    CommandSpec { name: "daemon", usage: "ferry daemon status|stop", description: "检查或停止 CLI 后台引擎", options: &[], notes: "不自动启动引擎；stop 不能停止桌面 App 的引擎。" },
    CommandSpec { name: "history", usage: "ferry history", description: "列出迁移历史", options: &[], notes: "返回 JSON 数组。" },
    CommandSpec { name: "env", usage: "ferry env", description: "检查 agent 可执行文件", options: &[], notes: "不表示会话索引就绪。" },
    CommandSpec { name: "health", usage: "ferry health", description: "检查引擎连接", options: &[], notes: "必要时自动启动 daemon。" },
    CommandSpec { name: "version", usage: "ferry version", description: "显示本地 CLI 版本", options: &[], notes: "不连接或启动引擎。" },
];

pub fn render(topic: Option<&str>, machine: bool) -> Result<String, String> {
    let selected: Vec<_> = match topic {
        Some(topic) => vec![COMMANDS
            .iter()
            .find(|item| item.name == topic)
            .ok_or_else(|| format!("未知帮助主题: {topic}"))?],
        None => COMMANDS.iter().collect(),
    };
    if machine {
        let commands: Vec<Value> = selected.iter().map(|command| json!({
            "name": command.name, "usage": command.usage, "description": command.description,
            "options": command.options.iter().map(|item| json!({"name": item.name, "value": item.value, "description": item.description})).collect::<Vec<_>>(),
            "notes": command.notes,
        })).collect();
        return serde_json::to_string_pretty(&json!({"schema_version": 1, "commands": commands}))
            .map_err(|error| error.to_string());
    }
    let mut out = String::from("Ferry — 本地会话检索与迁移\n\n");
    for command in selected {
        out.push_str(&format!("{}\n  {}\n", command.usage, command.description));
        if topic.is_some() {
            for option in command.options {
                let value = option
                    .value
                    .map(|value| format!(" <{value}>"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "  --{}{value}  {}\n",
                    option.name, option.description
                ));
            }
            out.push_str(command.notes);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("帮助：ferry help [command] [--json] 或 ferry <command> --help\n退出码：0 成功；1 用法/业务失败；2 连接失败；3 等待超时。\n");
    Ok(out)
}
