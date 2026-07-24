//! Operation IPC 请求编码。
//!
//! 这里固定 Host 到 Engine 的方法名与参数形状，确保应用阶段只能携带已签发的
//! plan id，不能重新注入业务参数或任意 Engine method。

use crate::operation_input::OperationPlanInput;
use serde_json::json;

pub(crate) fn validate_plan_id(plan_id: &str) -> Result<(), String> {
    if !(8..=128).contains(&plan_id.len())
        || !plan_id.starts_with("op_")
        || !plan_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("Operation plan_id 无效".to_owned());
    }
    Ok(())
}

pub(crate) fn operation_plan_request(
    input: &OperationPlanInput,
    validate: impl FnOnce(&OperationPlanInput) -> Result<(), String>,
) -> Result<String, String> {
    validate(input)?;
    serde_json::to_string(&json!({
        "method": "operation.plan",
        "params": {"input": input},
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn operation_plan_id_request(
    method: &'static str,
    plan_id: &str,
) -> Result<String, String> {
    validate_plan_id(plan_id)?;
    if !matches!(
        method,
        "operation.apply" | "operation.status" | "operation.cancel"
    ) {
        return Err("Operation Engine 方法无效".to_owned());
    }
    serde_json::to_string(&json!({
        "method": method,
        "params": {"plan_id": plan_id},
    }))
    .map_err(|error| error.to_string())
}
