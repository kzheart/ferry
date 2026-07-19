"""Thinking/reasoning 跨家降级(对齐 OpenCode 换模型策略)。

有可见正文 → 降为普通 text(不带 signature/encrypted 元数据)。
仅有加密/签名、无正文 → 丢弃并记损耗。
"""


def visible_text(text) -> str | None:
    if not isinstance(text, str):
        return None
    return text if text.strip() else None


def codex_summary_text(payload: dict) -> str | None:
    """从 Codex reasoning.summary 提取可读摘要。"""
    summary = payload.get("summary") or []
    if isinstance(summary, str):
        return visible_text(summary)
    if not isinstance(summary, list):
        return None
    parts = []
    for item in summary:
        if isinstance(item, dict):
            t = item.get("text") or ""
            if isinstance(t, str) and t.strip():
                parts.append(t)
        elif isinstance(item, str) and item.strip():
            parts.append(item)
    return "\n".join(parts) if parts else None
