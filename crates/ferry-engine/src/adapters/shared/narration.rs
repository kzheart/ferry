//! 版本化 narration 模板：降级工具调用写入目标 Agent 上下文的叙述文本。
//!
//! 语义事实源：`engine/adapters/shared/narration.py`。
//!
//! `content_locale` 属于迁移请求（决定生成内容的语言），与 UI locale 无关；
//! 已生成的目标会话内容不随 UI 切换而变化。
//!
//! **传播方式**：Python 用 `contextvars`，Rust 用 `thread_local!` + RAII 守卫。
//! 迁移事务在 operations 的单 worker 线程内串行执行（方案 §2.1 第 7 条），
//! 整个 plan/preview/write 链路都在同一个线程上，因此 thread_local 与
//! contextvars 的可见范围一致；跨线程时守卫**不会**传播，需要在新线程里重新
//! `enter`（Python 的 contextvars 在 `ThreadPoolExecutor` 里同样不自动传播）。

use std::cell::RefCell;

use crate::model::{tool_result_text, ToolCall};

use super::dialect::python_str;
use super::writing::python_json_dumps;

pub const DEFAULT_TEMPLATE: &str = "historical-tool-call-v1";
/// 降级叙述固定写英文：目标 Agent 读的是上下文而非界面文案，英文最通用，
/// 也避免用户界面语言影响已写入目标会话的内容。
pub const DEFAULT_LOCALE: &str = "en";

/// 入参截断上限（字符数）。
const INPUT_LIMIT: usize = 500;
/// 结果截断上限（字符数）。
const OUTPUT_LIMIT: usize = 2000;

/// `(template, locale)` → 模板文本。文案逐字节对齐 Python，不得改动。
const TEMPLATES: &[(&str, &str, &str)] = &[
    (
        DEFAULT_TEMPLATE,
        "zh-CN",
        "[历史记录:此前通过工具 {name} 执行了操作]\n参数: {input}\n结果:\n{output}",
    ),
    (
        DEFAULT_TEMPLATE,
        "en",
        "[History: tool {name} was previously invoked]\nInput: {input}\nResult:\n{output}",
    ),
];

const EMPTY_OUTPUT: &[(&str, &str)] = &[("zh-CN", "(无输出)"), ("en", "(no output)")];

thread_local! {
    static ACTIVE: RefCell<(Option<String>, Option<String>)> =
        const { RefCell::new((None, None)) };
}

/// 在迁移事务范围内声明 narration 的内容语言与模板版本。
///
/// 返回的守卫析构时恢复上一层设置，等价 Python `contextvars` 的 `reset(token)`。
#[must_use = "守卫析构即退出该 locale 作用域"]
pub struct ContentLocaleGuard {
    previous: (Option<String>, Option<String>),
}

impl Drop for ContentLocaleGuard {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.previous);
        ACTIVE.with(|active| *active.borrow_mut() = previous);
    }
}

/// 对齐 Python 的 `with narration.content_locale(locale)`。
pub fn content_locale(locale: Option<&str>, template: Option<&str>) -> ContentLocaleGuard {
    let next = (locale.map(str::to_string), template.map(str::to_string));
    let previous = ACTIVE.with(|active| active.replace(next));
    ContentLocaleGuard { previous }
}

/// `zh*` → `zh-CN`，其余（含空串与缺省）→ `en`。
fn normalize(locale: Option<&str>) -> &'static str {
    match locale {
        Some(value) if !value.is_empty() && value.to_lowercase().starts_with("zh") => "zh-CN",
        _ => DEFAULT_LOCALE,
    }
}

/// 取前 `limit` 个**字符**（Python 的 `text[:limit]` 按 code point 切）。
fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// 渲染历史工具调用叙述；模板版本未知时返回 `None`。
pub fn try_narrate(
    tool: &ToolCall,
    locale: Option<&str>,
    template: Option<&str>,
) -> Option<String> {
    let (active_locale, active_template) = ACTIVE.with(|active| active.borrow().clone());
    let locale = normalize(
        locale
            .filter(|value| !value.is_empty())
            .or(active_locale.as_deref()),
    );
    let template = template
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or(active_template)
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string());

    let source = if tool.input.is_object() {
        take_chars(&python_json_dumps(&tool.input), INPUT_LIMIT)
    } else {
        take_chars(&python_str(&tool.input), INPUT_LIMIT)
    };
    let rendered_output = tool_result_text(tool.result.as_ref());
    let fallback = EMPTY_OUTPUT
        .iter()
        .find(|(key, _)| *key == locale)
        .map(|(_, text)| *text)
        .unwrap_or("");
    let output = take_chars(
        if rendered_output.is_empty() {
            fallback
        } else {
            &rendered_output
        },
        OUTPUT_LIMIT,
    );

    let body = TEMPLATES
        .iter()
        .find(|(name, key, _)| *name == template && *key == locale)
        .map(|(_, _, body)| *body)?;
    Some(render(body, &tool.name, &source, &output))
}

/// 单趟替换 `{name}` / `{input}` / `{output}`。
///
/// 不能用链式 `replace`：被代入的入参文本里如果含 `{output}` 字面量，
/// 链式替换会二次代入，而 Python 的 `str.format` 只处理模板本身。
fn render(body: &str, name: &str, input: &str, output: &str) -> String {
    let mut out = String::with_capacity(body.len() + input.len() + output.len());
    let mut rest = body;
    while let Some(position) = rest.find('{') {
        out.push_str(&rest[..position]);
        let tail = &rest[position..];
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            return out;
        };
        match &tail[1..end] {
            "name" => out.push_str(name),
            "input" => out.push_str(input),
            "output" => out.push_str(output),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// [`try_narrate`] 的兜底版本：未知模板回落到 [`DEFAULT_TEMPLATE`]。
///
/// Python 在未知模板上抛 `KeyError`（经 RPC 兜底成 `internal.unexpected`）；
/// 目前 `content_locale` 的唯一调用方只传 locale，模板恒为 v1，两者不可区分。
pub fn narrate(tool: &ToolCall) -> String {
    try_narrate(tool, None, None)
        .or_else(|| try_narrate(tool, None, Some(DEFAULT_TEMPLATE)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{text_tool_result, ToolResultStatus};
    use serde_json::json;

    fn tool(name: &str, input: serde_json::Value, output: &str) -> ToolCall {
        let mut call = ToolCall::new(name, Some("shell.exec".into()), input);
        call.result = Some(text_tool_result(output, ToolResultStatus::Success));
        call
    }

    #[test]
    fn english_template_is_byte_exact() {
        let call = tool("Bash", json!({"command": "ls"}), "a\nb");
        assert_eq!(
            narrate(&call),
            "[History: tool Bash was previously invoked]\n\
             Input: {\"command\": \"ls\"}\nResult:\na\nb"
        );
    }

    #[test]
    fn chinese_template_activates_on_zh_locales() {
        let call = tool("Bash", json!({"command": "ls"}), "ok");
        let _guard = content_locale(Some("zh-Hans"), None);
        assert_eq!(
            narrate(&call),
            "[历史记录:此前通过工具 Bash 执行了操作]\n参数: {\"command\": \"ls\"}\n结果:\nok"
        );
    }

    #[test]
    fn empty_results_fall_back_per_locale() {
        let mut call = tool("Bash", json!({"command": "ls"}), "");
        call.result = None;
        assert!(narrate(&call).ends_with("Result:\n(no output)"));
        let _guard = content_locale(Some("zh-CN"), None);
        assert!(narrate(&call).ends_with("结果:\n(无输出)"));
    }

    #[test]
    fn inputs_clip_at_500_chars_and_outputs_at_2000() {
        let long_command = "x".repeat(1000);
        let long_output = "y".repeat(3000);
        let call = tool("Bash", json!({"command": long_command}), &long_output);
        let text = narrate(&call);
        let input_line = text
            .lines()
            .find(|line| line.starts_with("Input: "))
            .unwrap();
        assert_eq!(input_line.chars().count() - "Input: ".len(), 500);
        // 截断按字符计，且不带省略号（与 preview 的 180/2500 规则不同）。
        assert!(input_line.ends_with('x'));
        let output = text.split("Result:\n").nth(1).unwrap();
        assert_eq!(output.chars().count(), 2000);
    }

    #[test]
    fn non_dict_inputs_use_pythons_str() {
        let call = tool("Bash", json!("ls -la"), "ok");
        assert!(narrate(&call).contains("Input: ls -la"));
        let listed = tool("Bash", json!([1, 2]), "ok");
        assert!(narrate(&listed).contains("Input: [1, 2]"));
    }

    #[test]
    fn locale_guard_restores_the_previous_scope() {
        let call = tool("Bash", json!({"command": "ls"}), "ok");
        {
            let _outer = content_locale(Some("zh"), None);
            {
                let _inner = content_locale(Some("en"), None);
                assert!(narrate(&call).starts_with("[History:"));
            }
            assert!(narrate(&call).starts_with("[历史记录:"));
        }
        assert!(narrate(&call).starts_with("[History:"));
    }
}
