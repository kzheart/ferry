//! 统一轮次解析契约：`TurnIndex`（读侧）与 `NativeEditCodec`（写侧）。
//!
//! 语义事实源：`engine/adapters/shared/codec.py`。
//!
//! 每个 Agent 只允许存在一份原生会话解析实现；reader、delete-turn、rewrite、
//! replace-reply 全部消费同一个 `TurnIndex`，避免语义漂移。

use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};

/// 一轮对话在原生记录中的区间。
///
/// `ordinal` 从 1 起；`locator` 是对 UI 稳定的定位符；
/// `[start, end)` 是原生记录序列中的半开区间。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnSpan {
    pub ordinal: usize,
    pub locator: String,
    pub start: usize,
    pub end: usize,
}

impl TurnSpan {
    pub fn new(ordinal: usize, locator: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            ordinal,
            locator: locator.into(),
            start,
            end,
        }
    }
}

/// 读侧契约：所有 Agent 必须提供。
///
/// Python 的 `document` 是鸭子类型；Rust 用关联类型让每个 adapter 声明自己的
/// 原生文档与可见消息表示，共享层只依赖 [`TurnSpan`] 序列。
pub trait TurnIndex {
    /// 原生会话文档（通常是 `EditDocument` 或 adapter 私有的记录数组）。
    type Document: ?Sized;
    /// 参与轮次判定的可见原生消息（含索引）。
    type VisibleMessage;

    fn visible_messages(&self, document: &Self::Document) -> Vec<Self::VisibleMessage>;

    /// 按顺序排列的轮次区间。
    fn turns(&self, document: &Self::Document) -> Vec<TurnSpan>;
}

/// 写侧契约：可选能力，只读 Agent 不实现。
pub trait NativeEditCodec {
    type Document: ?Sized;
    /// AI 回复的结构化表示（Python 的 `AssistantReply`）。
    type Reply: ?Sized;
    /// 一次编辑产生的结构化变更项。
    ///
    /// 各 adapter 一律取 `type Change = crate::events::Event`：变更列表原样进
    /// `preview.changes` / `result.changes`，是 wire 面的一部分。
    type Change;

    /// 把 span 对应轮次的 AI 回复替换为 reply。
    fn replace_reply(
        &self,
        document: &mut Self::Document,
        span: &TurnSpan,
        reply: &Self::Reply,
    ) -> DomainResult<Vec<Self::Change>>;

    /// 删除 span 对应的整轮。
    fn delete_turn(
        &self,
        document: &mut Self::Document,
        span: &TurnSpan,
    ) -> DomainResult<Vec<Self::Change>>;

    /// 改写 locator 指向的用户消息文本。
    fn rewrite_message(
        &self,
        document: &mut Self::Document,
        locator: &str,
        text: &str,
    ) -> DomainResult<Vec<Self::Change>>;
}

/// 按 ordinal（正整数）或 locator（字符串）选择轮次。
///
/// 字符串走 locator 精确匹配，匹配不上是 `LocatorStaleError`；
/// 其余走 [`positive_turn`]，越界是 `TurnOutOfRangeError`。
pub fn select_span<'a>(spans: &'a [TurnSpan], selector: &Value) -> DomainResult<&'a TurnSpan> {
    if let Value::String(locator) = selector {
        return spans
            .iter()
            .find(|span| &span.locator == locator)
            .ok_or_else(|| {
                let mut params = Map::new();
                params.insert("locator".into(), Value::String(locator.clone()));
                DomainError::locator_stale(None, params)
            });
    }
    let ordinal = positive_turn(selector)?;
    if ordinal > spans.len() {
        // Python 传的是已归一的 ordinal 而不是原始 selector，params 必须一致。
        return Err(DomainError::turn_out_of_range(
            Value::from(ordinal),
            Some(spans.len() as i64),
        ));
    }
    Ok(&spans[ordinal - 1])
}

/// 把选择符归一成 1 起的轮次序号。
///
/// 对齐 Python 的 `positive_turn`：显式拒绝 bool（`isinstance(True, int)` 为真），
/// 字符串一律拒绝（`int("3") != "3"`），非整数浮点拒绝（`int(3.7) != 3.7`），
/// 整数浮点接受（`3 == 3.0`）。
pub fn positive_turn(turn: &Value) -> DomainResult<usize> {
    let reject = || DomainError::turn_out_of_range(turn.clone(), None);
    let Value::Number(number) = turn else {
        return Err(reject());
    };
    let value = match number.as_i64() {
        Some(value) => value,
        None => {
            let float = number.as_f64().ok_or_else(reject)?;
            if !float.is_finite() || float.fract() != 0.0 || float > i64::MAX as f64 {
                return Err(reject());
            }
            float as i64
        }
    };
    if value < 1 {
        return Err(reject());
    }
    Ok(value as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spans() -> Vec<TurnSpan> {
        vec![
            TurnSpan::new(1, "uuid-a", 0, 2),
            TurnSpan::new(2, "uuid-b", 2, 5),
        ]
    }

    #[test]
    fn locator_selection_is_exact() {
        let spans = spans();
        assert_eq!(select_span(&spans, &json!("uuid-b")).unwrap().ordinal, 2);
        let error = select_span(&spans, &json!("gone")).unwrap_err();
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.params()["locator"], json!("gone"));
        assert_eq!(error.message(), "turn locator 已失效，请刷新会话");
    }

    #[test]
    fn ordinal_selection_is_one_based_and_bounded() {
        let spans = spans();
        assert_eq!(select_span(&spans, &json!(1)).unwrap().locator, "uuid-a");
        assert_eq!(select_span(&spans, &json!(2.0)).unwrap().locator, "uuid-b");
        let error = select_span(&spans, &json!(3)).unwrap_err();
        assert_eq!(error.code, "edit.turn_out_of_range");
        assert_eq!(error.params()["requested_turn"], json!(3));
        assert_eq!(error.params()["turn_count"], json!(2));
        assert_eq!(error.message(), "轮次超界: 共 2 轮");
    }

    #[test]
    fn booleans_and_strings_are_never_turn_numbers() {
        for selector in [json!(true), json!(false), json!(0), json!(-1), json!(3.7)] {
            let error = positive_turn(&selector).unwrap_err();
            assert_eq!(error.code, "edit.turn_out_of_range");
            assert_eq!(error.message(), "turn 必须是正整数");
            assert!(!error.params().contains_key("turn_count"));
        }
        // 字符串走 locator 分支，不会进入 positive_turn。
        assert!(positive_turn(&json!("3")).is_err());
        assert!(positive_turn(&json!(null)).is_err());
        assert!(positive_turn(&json!([1])).is_err());
    }

    #[test]
    fn empty_span_lists_report_zero_turns() {
        let error = select_span(&[], &json!(1)).unwrap_err();
        assert_eq!(error.params()["turn_count"], json!(0));
    }
}
