"""适配器图片输入到规范 ImageAsset 的统一归一化。"""
from __future__ import annotations

import base64
import re

from ...sessions.model import ImageAsset


SUPPORTED_IMAGE_MIME_TYPES = {
    "image/png", "image/jpeg", "image/gif", "image/webp",
}
DATA_URL_RE = re.compile(r"^data:([^;,]+);base64,([A-Za-z0-9+/=]+)$")


def image_from_base64(asset_id: str, mime_type: str, data: str,
                      filename: str | None = None) -> ImageAsset | None:
    if mime_type not in SUPPORTED_IMAGE_MIME_TYPES or not isinstance(data, str):
        return None
    try:
        base64.b64decode(data, validate=True)
    except (ValueError, TypeError):
        return None
    return ImageAsset(asset_id, mime_type, data, filename)


def image_from_data_url(asset_id: str, url: str,
                        filename: str | None = None) -> ImageAsset | None:
    if not isinstance(url, str):
        return None
    match = DATA_URL_RE.match(url)
    if match is None:
        return None
    return image_from_base64(asset_id, match.group(1).lower(), match.group(2), filename)
