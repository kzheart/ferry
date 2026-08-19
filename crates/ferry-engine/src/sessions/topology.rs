//! 会话树规则。
//!
//! 语义事实源：`engine/sessions/topology.py`。
//!
//! 分层备注：Python 侧 `adapters/shared/scanner` 反向 import 了本模块，形成
//! `adapters → sessions` 的倒置。Rust 的结构测试禁止这个方向（`tests/structure.rs`），
//! 方案 §1.1 因此把 `session_roots` 下沉到 `adapters/shared/scanner`（WP-B2 已落地）。
//! 本模块只做 re-export，保留 `sessions::topology::session_roots` 这个调用点，
//! 并用一组行为测试守住两侧口径一致。

pub use crate::adapters::shared::scanner::session_roots;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::ScanRow;
    use serde_json::{json, Value};

    fn row(value: Value) -> ScanRow {
        value.as_object().expect("测试行必须是 object").clone()
    }

    #[test]
    fn children_attach_and_aggregate_upwards() {
        let rows = vec![
            row(json!({"id": "a", "parent_id": null, "count": 1, "size": 10, "updated": 100})),
            row(json!({"id": "b", "parent_id": "a", "count": 2, "size": 20, "updated": 300})),
            row(json!({"id": "c", "parent_id": "a", "count": 4, "size": 40, "updated": 200})),
        ];
        let roots = session_roots(rows).unwrap();
        assert_eq!(roots.len(), 1);
        let root = &roots[0];
        assert_eq!(root["id"], Value::from("a"));
        assert_eq!(root["count"], Value::from(7));
        assert_eq!(root["size"], Value::from(70));
        assert_eq!(root["own_count"], Value::from(1));
        assert_eq!(root["updated"], Value::from(300));
        assert_eq!(root["child_count"], Value::from(2));
        assert_eq!(root["tree_count"], Value::from(3));
        // children 按 updated 降序。
        let children = root["children"].as_array().unwrap();
        assert_eq!(children[0]["id"], Value::from("b"));
        assert_eq!(children[1]["id"], Value::from("c"));
        assert_eq!(children[0]["root_id"], Value::from("a"));
    }

    #[test]
    fn cycles_are_forced_to_be_roots() {
        let rows = vec![
            row(json!({"id": "a", "parent_id": "b", "count": 1, "updated": 1})),
            row(json!({"id": "b", "parent_id": "a", "count": 1, "updated": 2})),
        ];
        let roots = session_roots(rows).unwrap();
        assert_eq!(roots.len(), 2);
        // 环内节点强制为根，parent_id 抹平。
        assert!(roots
            .iter()
            .all(|root| root["parent_id"] == Value::Null && root["tree_count"] == 1));
        // 根按 updated 降序。
        assert_eq!(roots[0]["id"], Value::from("b"));
    }

    #[test]
    fn self_parent_is_treated_as_a_root() {
        let roots = session_roots(vec![row(
            json!({"id": "a", "parent_id": "a", "updated": 5}),
        )])
        .unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["parent_id"], Value::Null);
    }

    #[test]
    fn missing_parents_fall_back_to_roots() {
        let roots = session_roots(vec![row(
            json!({"id": "a", "parent_id": "ghost", "updated": 5}),
        )])
        .unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["parent_id"], Value::Null);
        assert_eq!(roots[0]["root_id"], Value::from("a"));
    }

    #[test]
    fn own_fields_win_over_totals() {
        let roots = session_roots(vec![row(json!({
            "id": "a", "count": 9, "own_count": 3, "size": 90, "own_size": 30,
            "updated": 900, "own_updated": 300
        }))])
        .unwrap();
        assert_eq!(roots[0]["count"], Value::from(3));
        assert_eq!(roots[0]["size"], Value::from(30));
        assert_eq!(roots[0]["updated"], Value::from(300));
    }
}
