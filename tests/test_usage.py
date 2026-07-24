"""扫描阶段 token 用量归一化与三工具解析口径。"""

from engine.adapters.claude.scanner import _usage_tokens
from engine.adapters.codex.scanner import _tokens_from_usage
from engine.adapters.opencode.scanner import _msg_tokens
from datetime import datetime, timezone

from engine.sessions.usage import (
    add_tokens, dominant_model, empty_tokens, has_tokens, iso_ms,
)


def test_claude_usage_maps_cache_fields():
    usage = {"input_tokens": 10, "output_tokens": 20,
             "cache_read_input_tokens": 30, "cache_creation_input_tokens": 40}
    assert _usage_tokens(usage) == {"input": 10, "output": 20,
                                    "cache_read": 30, "cache_write": 40}


def test_codex_usage_splits_cached_from_input():
    # input_tokens 含缓存命中,需拆出 cache_read;reasoning 计入 output
    usage = {"input_tokens": 19016, "cached_input_tokens": 11008,
             "output_tokens": 140, "reasoning_output_tokens": 60,
             "cache_write_input_tokens": 5}
    assert _tokens_from_usage(usage) == {"input": 8008, "output": 200,
                                         "cache_read": 11008, "cache_write": 5}


def test_codex_usage_never_negative_input():
    usage = {"input_tokens": 100, "cached_input_tokens": 500}
    assert _tokens_from_usage(usage)["input"] == 0


def test_opencode_message_tokens():
    data = {"tokens": {"input": 3, "output": 4, "reasoning": 6,
                       "cache": {"read": 7, "write": 8}}}
    assert _msg_tokens(data) == {"input": 3, "output": 10,
                                 "cache_read": 7, "cache_write": 8}


def test_add_and_sum_tokens():
    acc = empty_tokens()
    add_tokens(acc, {"input": 1, "output": 2, "cache_read": 3, "cache_write": 4})
    add_tokens(acc, {"input": 10})
    assert acc == {"input": 11, "output": 2, "cache_read": 3, "cache_write": 4}
    assert has_tokens(acc) is True
    assert has_tokens(empty_tokens()) is False


def test_dominant_model_picks_highest_total():
    by_model = {
        "a": {"input": 1, "output": 1, "cache_read": 0, "cache_write": 0},
        "b": {"input": 100, "output": 0, "cache_read": 0, "cache_write": 0},
    }
    assert dominant_model(by_model) == "b"
    assert dominant_model({}) == ""


def test_iso_ms_parses_z_suffix_and_passthrough():
    expected = int(datetime(2026, 7, 20, 17, 18, 13, 140000,
                            tzinfo=timezone.utc).timestamp() * 1000)
    assert iso_ms("2026-07-20T17:18:13.140Z") == expected
    assert iso_ms(expected) == expected
    assert iso_ms(None) is None
    assert iso_ms("not-a-date") is None
