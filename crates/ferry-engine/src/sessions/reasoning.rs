//! Thinking/reasoning 跨家降级（对齐 OpenCode 换模型策略）。
//!
//! 语义事实源：`engine/sessions/reasoning.py`。
//!
//! 有可见正文 → 降为普通 text（不带 signature/encrypted 元数据）。
//! 仅有加密/签名、无正文 → 丢弃并记损耗。

use serde_json::Value;

/// 取 thinking 块的可见正文：非字符串或全空白都返回 `None`。
///
/// Python 侧形参标注是 `text`（任意对象），非 `str` 直接判空，所以这里
/// 用 `&Value` 而不是 `&str`——调用点拿到的正是原生 JSON 值。
pub fn visible_text(text: &Value) -> Option<&str> {
    text.as_str().filter(|value| !value.trim().is_empty())
}

/// 已知是字符串时的便捷形态。
pub fn visible_str(text: &str) -> Option<&str> {
    Some(text).filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_non_blank_strings_count_as_visible() {
        assert_eq!(visible_text(&json!("hi")), Some("hi"));
        // Python 的 `text.strip()` 判空后返回的仍是**原串**，不是 strip 结果。
        assert_eq!(visible_text(&json!(" hi ")), Some(" hi "));
        assert_eq!(visible_text(&json!("  \n\t")), None);
        assert_eq!(visible_text(&json!("")), None);
        assert_eq!(visible_text(&json!(null)), None);
        assert_eq!(visible_text(&json!(1)), None);
        assert_eq!(visible_str("x"), Some("x"));
        assert_eq!(visible_str(" "), None);
    }
}
