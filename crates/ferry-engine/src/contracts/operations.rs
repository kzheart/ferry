// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

pub const OPERATION_PLAN_ID_PREFIX: &str = "op_";
pub const OPERATION_KINDS: &[&str] = &["edit", "migration", "metadata", "delete"];
pub const EDIT_OPERATION_KINDS: &[&str] = &["delete-turn", "rewrite", "replace-assistant-reply"];
pub const OPERATION_STATUSES: &[&str] = &[
    "planned",
    "queued",
    "applying",
    "applied",
    "failed",
    "cancelled",
    "expired",
];
pub const OPERATION_TERMINAL_STATUSES: &[&str] = &["applied", "failed", "cancelled", "expired"];
pub const OPERATION_SUCCESS_STATUS: &str = "applied";

/// 每行 = (kind, 字段名, 类型描述符)，顺序与契约声明一致。
pub const OPERATION_INPUT_FIELDS: &[(&str, &str, &str)] = &[
    ("edit", "tool", "agent-id"),
    ("edit", "ref", "session-ref"),
    ("edit", "ops", "edit-operation[]"),
    ("migration", "source_tool", "agent-id"),
    ("migration", "ref", "session-ref"),
    ("migration", "target_tool", "agent-id"),
    ("migration", "max_turn", "positive-integer?"),
    ("metadata", "tool", "agent-id"),
    ("metadata", "ref", "session-ref"),
    ("metadata", "patch", "metadata-patch"),
    ("delete", "tool", "agent-id"),
    ("delete", "refs", "session-ref[]"),
];

/// 每行 = (op, 字段名, 类型描述符)。
pub const EDIT_OPERATION_FIELDS: &[(&str, &str, &str)] = &[
    ("delete-turn", "turn", "positive-integer"),
    ("rewrite", "locator", "string"),
    ("rewrite", "text", "string"),
    ("replace-assistant-reply", "turn", "positive-integer|string"),
    ("replace-assistant-reply", "reply", "assistant-reply"),
];

/// 每行 = (item kind, 字段名, 类型描述符)。
pub const ASSISTANT_REPLY_ITEM_FIELDS: &[(&str, &str, &str)] = &[
    ("text", "text", "string"),
    ("tool", "name", "string"),
    ("tool", "input", "json-object|string"),
    ("tool", "output", "string"),
];

/// 每行 = (字段名, 类型描述符)；`?` 结尾表示可选。
pub const METADATA_PATCH_FIELDS: &[(&str, &str)] = &[
    ("name", "string?"),
    ("pinned", "boolean?"),
    ("archived", "boolean?"),
    ("tags", "string[]?"),
];
