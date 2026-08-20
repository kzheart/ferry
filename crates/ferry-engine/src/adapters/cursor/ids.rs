//! Cursor 写入路径的标识符生成。
//!
//! 每个 adapter 自带一份：`adapters` 之间不互相引用（分层规则见 `adapters/mod.rs`），
//! 所以这里不复用 claude/codex 各自的同名助手。

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand::TryRngCore as _;

/// 小写十六进制编码。
pub fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("系统 CSPRNG 不可用");
    bytes
}

/// `uuid4()`：带连字符的 8-4-4-4-12 形态，Cursor 的 composerId / bubbleId 用它。
pub fn uuid4() -> String {
    let mut bytes = random_bytes::<16>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex_lower(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// 工具调用 id：格式是 `toolu_` + 任意字符串，实测自定义值可用。
pub fn tool_call_id() -> String {
    format!("toolu_{}", hex_lower(&random_bytes::<12>()))
}

/// `blobEncryptionKey` 占位：随机 32 字节的标准 base64。
///
/// 这个键与 `agentKv:blob:` 无关（实测 blob 是明文），但 Cursor 的 composerData
/// 里它一直存在，给一个格式正确的随机值比留空更接近原生形态。
pub fn blob_encryption_key() -> String {
    STANDARD.encode(random_bytes::<32>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid4_carries_the_version_and_variant_bits() {
        let value = uuid4();
        assert_eq!(value.len(), 36);
        assert_eq!(value.as_bytes()[14], b'4');
        assert!(matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(uuid4(), value);
    }

    #[test]
    fn tool_call_ids_use_the_native_prefix() {
        let value = tool_call_id();
        assert!(value.starts_with("toolu_"));
        assert_eq!(value.len(), "toolu_".len() + 24);
        assert_ne!(tool_call_id(), value);
    }

    #[test]
    fn the_encryption_key_placeholder_is_32_random_bytes() {
        let key = blob_encryption_key();
        assert_eq!(STANDARD.decode(&key).unwrap().len(), 32);
        assert_ne!(blob_encryption_key(), key);
    }

    #[test]
    fn hex_is_lower_case_and_zero_padded() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
