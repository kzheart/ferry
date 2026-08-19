// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

pub const OPAQUE_SESSION_REF_PREFIX: &str = "fsr_";
pub const OPAQUE_SESSION_REF_MIN_LENGTH: usize = 8;
pub const OPAQUE_SESSION_REF_MAX_LENGTH: usize = 128;

pub fn is_opaque_session_ref(value: &str) -> bool {
    (OPAQUE_SESSION_REF_MIN_LENGTH..=OPAQUE_SESSION_REF_MAX_LENGTH).contains(&value.len())
        && value.starts_with(OPAQUE_SESSION_REF_PREFIX)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
