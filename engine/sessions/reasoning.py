"""Thinking/reasoning 跨家降级(对齐 OpenCode 换模型策略)。

有可见正文 → 降为普通 text(不带 signature/encrypted 元数据)。
仅有加密/签名、无正文 → 丢弃并记损耗。
"""


def visible_text(text) -> str | None:
    if not isinstance(text, str):
        return None
    return text if text.strip() else None
