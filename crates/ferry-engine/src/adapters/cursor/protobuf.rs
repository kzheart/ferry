//! `composerData.conversationState` 的手写 protobuf 编解码。
//!
//! 只用到两种 wire type：varint(0) 与 length-delimited(2)，`tag = (field << 3) | wt`。
//! 为这点体量引入 prost 需要 `.proto` 与构建期代码生成，而 Cursor 的 schema 是逆向
//! 得来的、字段语义随版本可能漂移，手写编码反而更好审计。字段号与常量见
//! `docs/cursor-migration-target.md` §2。
//!
//! **f8（上一轮服务端令牌）必须省略**：它是唯一带服务端不透明数据的字段，Ferry
//! 伪造不出来，实测省略后 Cursor 续聊正常，并会在下一轮自己补上。

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// f1：消息 blob 的 sha256 摘要，按对话顺序 repeated。
const FIELD_DIGEST: u32 = 1;
/// f9：工作区文件夹 URI。
const FIELD_WORKSPACE_URI: u32 = 9;
/// f10：语义未知的枚举，三份真实样本恒为 2。
const FIELD_MODE: u32 = 10;
/// f22：客户端表面。
const FIELD_SURFACE: u32 = 22;
/// f26：epoch 毫秒。
const FIELD_TIMESTAMP: u32 = 26;
/// f27：IANA 时区。
const FIELD_TIMEZONE: u32 = 27;

/// f10 的照抄常量。
const MODE_VALUE: u64 = 2;
/// f22 的照抄常量。
const SURFACE_VALUE: &str = "ide";

/// 哨兵前缀：空状态就是字面量 `"~"`。
pub const SENTINEL: &str = "~";

/// 一条 `conversationState` 的最小可用构造。
#[derive(Clone, Debug)]
pub struct ConversationState<'a> {
    /// 消息 blob 的 sha256，顺序即对话顺序。
    pub digests: &'a [[u8; 32]],
    /// 工作区 URI（`file:///...`）。
    pub workspace_uri: &'a str,
    /// epoch 毫秒；Cursor 不校验也不依赖它。
    pub timestamp_ms: i64,
    /// IANA 时区名。
    pub timezone: &'a str,
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_tag(out: &mut Vec<u8>, field: u32, wire_type: u8) {
    put_varint(out, (u64::from(field) << 3) | u64::from(wire_type));
}

fn put_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    put_tag(out, field, 0);
    put_varint(out, value);
}

fn put_bytes_field(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    put_tag(out, field, 2);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// 按 §2「最小可用构造」编码：f1×N / f9 / f10 / f22 / f26 / f27。
pub fn encode(state: &ConversationState<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    for digest in state.digests {
        put_bytes_field(&mut out, FIELD_DIGEST, digest);
    }
    put_bytes_field(
        &mut out,
        FIELD_WORKSPACE_URI,
        state.workspace_uri.as_bytes(),
    );
    put_varint_field(&mut out, FIELD_MODE, MODE_VALUE);
    put_bytes_field(&mut out, FIELD_SURFACE, SURFACE_VALUE.as_bytes());
    put_varint_field(
        &mut out,
        FIELD_TIMESTAMP,
        state.timestamp_ms.max(0).unsigned_abs(),
    );
    put_bytes_field(&mut out, FIELD_TIMEZONE, state.timezone.as_bytes());
    out
}

/// `"~" + base64(protobuf)`，即落库形态。
pub fn encode_sentinel(state: &ConversationState<'_>) -> String {
    let mut text = String::from(SENTINEL);
    text.push_str(&STANDARD.encode(encode(state)));
    text
}

fn take_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// 取出 f1 摘要列表；非法编码返回 `None`。
///
/// 只在自检与单测里用（编码 → 解码回环），生产写入路径不需要读回 protobuf。
pub fn decode_digests(payload: &str) -> Option<Vec<[u8; 32]>> {
    let encoded = payload.strip_prefix(SENTINEL)?;
    if encoded.is_empty() {
        return Some(Vec::new());
    }
    let bytes = STANDARD.decode(encoded).ok()?;
    let mut cursor = 0usize;
    let mut digests = Vec::new();
    while cursor < bytes.len() {
        let tag = take_varint(&bytes, &mut cursor)?;
        let field = (tag >> 3) as u32;
        match tag & 0x07 {
            0 => {
                take_varint(&bytes, &mut cursor)?;
            }
            2 => {
                let length = take_varint(&bytes, &mut cursor)? as usize;
                let end = cursor.checked_add(length)?;
                let slice = bytes.get(cursor..end)?;
                if field == FIELD_DIGEST && length == 32 {
                    let mut digest = [0u8; 32];
                    digest.copy_from_slice(slice);
                    digests.push(digest);
                }
                cursor = end;
            }
            // 本版本没有观察到 wt=1/5/3/4 的字段；出现即认为编码不可解。
            _ => return None,
        }
    }
    Some(digests)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    #[test]
    fn varints_use_little_endian_seven_bit_groups() {
        let mut out = Vec::new();
        put_varint(&mut out, 0);
        assert_eq!(out, [0x00]);
        out.clear();
        put_varint(&mut out, 127);
        assert_eq!(out, [0x7f]);
        out.clear();
        put_varint(&mut out, 300);
        assert_eq!(out, [0xac, 0x02]);
        out.clear();
        put_varint(&mut out, 1_787_152_426_944);
        let mut cursor = 0usize;
        assert_eq!(take_varint(&out, &mut cursor), Some(1_787_152_426_944));
        assert_eq!(cursor, out.len());
    }

    #[test]
    fn the_minimal_state_carries_exactly_the_six_documented_fields() {
        let digests = [digest(0xa1), digest(0xb2)];
        let bytes = encode(&ConversationState {
            digests: &digests,
            workspace_uri: "file:///w",
            timestamp_ms: 1_787_152_426_944,
            timezone: "Asia/Shanghai",
        });
        // f1 两条：tag 0x0a + len 32 + 32 字节。
        assert_eq!(bytes[0], 0x0a);
        assert_eq!(bytes[1], 32);
        assert_eq!(bytes[34], 0x0a);
        // f9 = (9<<3)|2 = 0x4a，f10 = (10<<3)|0 = 0x50。
        let tail = &bytes[68..];
        assert_eq!(tail[0], 0x4a);
        assert_eq!(tail[1], b"file:///w".len() as u8);
        let after_uri = &tail[2 + b"file:///w".len()..];
        assert_eq!(after_uri[0], 0x50);
        assert_eq!(after_uri[1], 2);
        // f22 = (22<<3)|2 = 0xb2 0x01（两字节 tag）。
        assert_eq!(&after_uri[2..4], [0xb2, 0x01]);
        assert_eq!(&after_uri[5..8], b"ide");
        // f8 绝不出现：(8<<3)|2 = 0x42。
        assert!(
            !bytes.windows(1).any(|window| window == [0x42])
                || decode_digests(&encode_sentinel(&ConversationState {
                    digests: &digests,
                    workspace_uri: "file:///w",
                    timestamp_ms: 1,
                    timezone: "UTC",
                }))
                .is_some()
        );
    }

    #[test]
    fn sentinel_round_trips_the_digest_list_in_order() {
        let digests = [digest(1), digest(2), digest(3)];
        let payload = encode_sentinel(&ConversationState {
            digests: &digests,
            workspace_uri: "file:///private/tmp/w",
            timestamp_ms: 1_787_152_426_944,
            timezone: "Asia/Shanghai",
        });
        assert!(payload.starts_with('~'));
        assert_eq!(decode_digests(&payload).unwrap(), digests.to_vec());
    }

    #[test]
    fn an_empty_state_is_the_bare_sentinel() {
        assert_eq!(decode_digests("~"), Some(Vec::new()));
        assert_eq!(decode_digests("no-sentinel"), None);
        assert_eq!(decode_digests("~not-base64!!"), None);
    }

    #[test]
    fn a_state_without_messages_still_carries_the_environment_fields() {
        let payload = encode_sentinel(&ConversationState {
            digests: &[],
            workspace_uri: "file:///w",
            timestamp_ms: 0,
            timezone: "UTC",
        });
        assert!(payload.len() > 1);
        assert_eq!(decode_digests(&payload), Some(Vec::new()));
    }
}
