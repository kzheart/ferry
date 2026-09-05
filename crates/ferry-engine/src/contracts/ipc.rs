// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

pub const FERRY_IPC_PROTOCOL: &str = "ferry-ipc/1";
pub const FERRY_CONTRACT_HASH: &str =
    "sha256:335daa3550349d6cd0de1af9c52cbf36bc5321c75526f954b284ddbe72f5cc49";

/// 请求信封的字段集合必须精确相等：多一个字段即 rpc.invalid_request。
pub const REQUEST_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "method", "params"];
pub const RESPONSE_SUCCESS_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "ok", "result"];
pub const RESPONSE_FAILURE_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "ok", "error"];
pub const ERROR_REQUIRED_FIELDS: &[&str] = &["code", "category", "retryable", "params"];
pub const EVENT_REQUIRED_FIELDS: &[&str] = &["protocol", "type", "payload"];
pub const EVENT_OPTIONAL_FIELDS: &[&str] = &["correlation_id", "context"];
