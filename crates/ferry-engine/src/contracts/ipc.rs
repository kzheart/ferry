// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

pub const FERRY_IPC_PROTOCOL: &str = "ferry-ipc/1";
pub const FERRY_CONTRACT_HASH: &str =
    "sha256:b251f1b2779811c86f55c96a2e765e55c061dd132135a84abb59b64cf3649fb8";

/// 请求信封的字段集合必须精确相等：多一个字段即 rpc.invalid_request。
pub const REQUEST_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "method", "params"];
pub const RESPONSE_SUCCESS_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "ok", "result"];
pub const RESPONSE_FAILURE_REQUIRED_FIELDS: &[&str] = &["protocol", "id", "ok", "error"];
pub const ERROR_REQUIRED_FIELDS: &[&str] = &["code", "category", "retryable", "params"];
pub const EVENT_REQUIRED_FIELDS: &[&str] = &["protocol", "type", "payload"];
pub const EVENT_OPTIONAL_FIELDS: &[&str] = &["correlation_id", "context"];
