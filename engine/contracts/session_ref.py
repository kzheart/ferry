"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""
from __future__ import annotations

OPAQUE_SESSION_REF_PREFIX = 'fsr_'
OPAQUE_SESSION_REF_MIN_LENGTH = 8
OPAQUE_SESSION_REF_MAX_LENGTH = 128

def is_opaque_session_ref(value: object) -> bool:
    return (
        isinstance(value, str)
        and OPAQUE_SESSION_REF_MIN_LENGTH <= len(value) <= OPAQUE_SESSION_REF_MAX_LENGTH
        and value.startswith(OPAQUE_SESSION_REF_PREFIX)
        and all(character.isascii() and (character.isalnum() or character in '_-')
                for character in value)
    )
