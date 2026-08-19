//! 适配器图片输入到规范 `ImageAsset` 的统一归一化。

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine as _};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

use crate::model::ImageAsset;

/// 可以原样保留的图片 MIME 白名单。
pub const SUPPORTED_IMAGE_MIME_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// 与 `base64.b64decode(data, validate=True)` 等价的解码器。
///
/// `with_decode_allow_trailing_bits(true)`：Python 只校验字母表与填充长度，
/// 不检查末位符号的冗余比特；Rust 默认更严，会把 Python 接受的串判成非法。
const PYTHON_B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

/// `^data:([^;,]+);base64,([A-Za-z0-9+/=]+)$`
///
/// 尾部的 `\n?` 是 Python `$` 的语义（允许一个结尾换行），regex crate 的 `$`
/// 只匹配文本末尾，必须显式补上才等价。
static DATA_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^data:([^;,]+);base64,([A-Za-z0-9+/=]+)\n?$").expect("data URL 正则是常量")
});

/// base64 图片 → `ImageAsset`；MIME 不在白名单、载荷不是字符串或不是合法
/// base64 时返回 `None`（调用方按"这不是可保留的图片"处理）。
pub fn image_from_base64(
    asset_id: &str,
    mime_type: &str,
    data: &Value,
    filename: Option<&str>,
) -> Option<ImageAsset> {
    if !SUPPORTED_IMAGE_MIME_TYPES.contains(&mime_type) {
        return None;
    }
    let payload = data.as_str()?;
    PYTHON_B64.decode(payload).ok()?;
    Some(ImageAsset {
        id: asset_id.to_string(),
        mime_type: mime_type.to_string(),
        data: payload.to_string(),
        filename: filename.map(str::to_string),
    })
}

/// `data:` URL → `ImageAsset`。MIME 统一小写后再做白名单判定。
pub fn image_from_data_url(
    asset_id: &str,
    url: &Value,
    filename: Option<&str>,
) -> Option<ImageAsset> {
    let raw = url.as_str()?;
    let captures = DATA_URL_RE.captures(raw)?;
    let mime_type = captures.get(1)?.as_str().to_lowercase();
    let data = Value::from(captures.get(2)?.as_str());
    image_from_base64(asset_id, &mime_type, &data, filename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PNG_PIXEL: &str = "iVBORw0KGgo=";

    #[test]
    fn only_whitelisted_mime_types_survive() {
        assert!(image_from_base64("a1", "image/png", &json!(PNG_PIXEL), None).is_some());
        assert!(image_from_base64("a1", "image/svg+xml", &json!(PNG_PIXEL), None).is_none());
        assert!(image_from_base64("a1", "IMAGE/PNG", &json!(PNG_PIXEL), None).is_none());
    }

    #[test]
    fn payloads_must_be_strings_and_valid_base64() {
        assert!(image_from_base64("a1", "image/png", &json!(123), None).is_none());
        assert!(image_from_base64("a1", "image/png", &json!(null), None).is_none());
        // 非字母表字符 / 长度不是 4 的倍数 -> 与 validate=True 一样拒绝。
        assert!(image_from_base64("a1", "image/png", &json!("QQ*="), None).is_none());
        assert!(image_from_base64("a1", "image/png", &json!("QQQ"), None).is_none());
        // 空串是合法 base64（`b64decode("") == b""`），Python 会照样产出资产。
        assert_eq!(
            image_from_base64("a1", "image/png", &json!(""), None)
                .unwrap()
                .data,
            ""
        );
    }

    #[test]
    fn data_urls_lowercase_the_mime_and_keep_the_payload() {
        let asset = image_from_data_url(
            "a1",
            &json!(format!("data:IMAGE/PNG;base64,{PNG_PIXEL}")),
            Some("shot.png"),
        )
        .unwrap();
        assert_eq!(asset.mime_type, "image/png");
        assert_eq!(asset.data, PNG_PIXEL);
        assert_eq!(asset.filename.as_deref(), Some("shot.png"));
        assert_eq!(asset.id, "a1");
    }

    #[test]
    fn malformed_data_urls_are_rejected() {
        for url in [
            json!("data:image/png,QQ=="),
            json!("data:image/png;base64,"),
            json!("http://example.com/a.png"),
            json!("prefix data:image/png;base64,QQ=="),
            json!(42),
        ] {
            assert!(image_from_data_url("a1", &url, None).is_none(), "url={url}");
        }
    }
}
