"""版本化 narration 模板：降级工具调用写入目标 Agent 上下文的叙述文本。

content_locale 属于迁移请求（决定生成内容的语言），与 UI locale 无关；
已生成的目标会话内容不随 UI 切换而变化。
"""
from __future__ import annotations

import contextvars
import json
from contextlib import contextmanager

DEFAULT_TEMPLATE = "historical-tool-call-v1"
DEFAULT_LOCALE = "zh-CN"

_TEMPLATES = {
    ("historical-tool-call-v1", "zh-CN"):
        "[历史记录:此前通过工具 {name} 执行了操作]\n参数: {input}\n结果:\n{output}",
    ("historical-tool-call-v1", "en"):
        "[History: tool {name} was previously invoked]\n"
        "Input: {input}\nResult:\n{output}",
}
_EMPTY_OUTPUT = {"zh-CN": "(无输出)", "en": "(no output)"}

_ACTIVE = contextvars.ContextVar("narration_content", default=(None, None))


@contextmanager
def content_locale(locale: str | None, template: str | None = None):
    """在迁移事务范围内声明 narration 的内容语言与模板版本。"""
    token = _ACTIVE.set((locale, template))
    try:
        yield
    finally:
        _ACTIVE.reset(token)


def _normalize(locale: str | None) -> str:
    if locale and locale.lower().startswith("en"):
        return "en"
    return DEFAULT_LOCALE


def narrate(tool, locale: str | None = None,
            template: str | None = None) -> str:
    active_locale, active_template = _ACTIVE.get()
    locale = _normalize(locale or active_locale)
    template = template or active_template or DEFAULT_TEMPLATE
    source = json.dumps(tool.input, ensure_ascii=False)[:500] \
        if isinstance(tool.input, dict) else str(tool.input)[:500]
    output = (tool.output or _EMPTY_OUTPUT[locale])[:2000]
    return _TEMPLATES[(template, locale)].format(
        name=tool.name, input=source, output=output)
