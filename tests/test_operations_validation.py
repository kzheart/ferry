"""operations/validation.py 的形状校验回归网（纯函数，不触发任何 IO）。"""
import pytest

from engine.errors import AgentRequestError, InvalidReplyError
from engine.operations.validation import (
    validate_delete_input,
    validate_cleanup_input,
    validate_edit_input,
    validate_metadata_input,
    validate_migration_input,
    validate_ops,
    validate_restore_delete_input,
)


ADAPTERS = ("claude", "opencode", "codex")
REF = "ref-abcdef0123456789"
CLEANUP_SCOPE_ID = "0123456789abcdef"
CLEANUP_REF = "fsr_0123456789abcdef"


def _edit(**overrides):
    value = {
        "kind": "edit",
        "tool": "claude",
        "ref": REF,
        "ops": [{"op": "delete-turn", "turn": 1}],
    }
    value.update(overrides)
    return value


def _migration(**overrides):
    value = {
        "kind": "migration",
        "source_tool": "claude",
        "ref": REF,
        "target_tool": "opencode",
    }
    value.update(overrides)
    return value


def _metadata(**overrides):
    value = {
        "kind": "metadata",
        "tool": "claude",
        "ref": REF,
        "patch": {"name": "新名称"},
    }
    value.update(overrides)
    return value


def _cleanup(**overrides):
    value = {
        "kind": "cleanup",
        "scope_id": CLEANUP_SCOPE_ID,
        "targets": [{"tool": "claude", "ref": CLEANUP_REF, "reason": "旧会话"}],
    }
    value.update(overrides)
    return value


def test_edit_input_normalizes_and_defaults_probe():
    result = validate_edit_input(_edit())

    assert result == {
        "kind": "edit",
        "tool": "claude",
        "ref": REF,
        "ops": [{"op": "delete-turn", "turn": 1}],
        "probe": False,
    }


def test_edit_input_rejects_unknown_field():
    with pytest.raises(AgentRequestError, match="未知字段"):
        validate_edit_input(_edit(snapshot="yes"))


@pytest.mark.parametrize("overrides", [
    {"tool": ""},
    {"tool": 1},
    {"ref": ""},
    {"ref": None},
    {"probe": "true"},
])
def test_edit_input_rejects_bad_scalars(overrides):
    with pytest.raises(AgentRequestError):
        validate_edit_input(_edit(**overrides))


def test_edit_input_rejects_oversized_ops():
    ops = [
        {"op": "rewrite", "locator": f"loc-{index}", "text": "x" * 20_000}
        for index in range(5)
    ]

    with pytest.raises(AgentRequestError, match="64 KiB"):
        validate_edit_input(_edit(ops=ops))


def test_migration_input_keeps_optional_fields_only_when_present():
    assert validate_migration_input(_migration(), ADAPTERS) == {
        "kind": "migration",
        "source_tool": "claude",
        "ref": REF,
        "target_tool": "opencode",
        "probe": False,
    }
    with_optional = validate_migration_input(
        _migration(max_turn=7, probe=True, probe_model="m-1"), ADAPTERS,
    )
    assert with_optional["max_turn"] == 7
    assert with_optional["probe_model"] == "m-1"
    assert with_optional["probe"] is True


@pytest.mark.parametrize("overrides, message", [
    ({"source_tool": "unknown"}, "Agent 非法"),
    ({"target_tool": "claude"}, "源和目标不能相同"),
    ({"ref": "有 空格"}, "ref 非法"),
    ({"max_turn": 0}, "max_turn 非法"),
    ({"max_turn": True}, "max_turn 非法"),
    ({"probe": "yes"}, "probe 必须是布尔值"),
    ({"probe_model": ""}, "probe_model 非法"),
])
def test_migration_input_rejections(overrides, message):
    with pytest.raises(AgentRequestError, match=message):
        validate_migration_input(_migration(**overrides), ADAPTERS)


def test_migration_input_rejects_unknown_field():
    with pytest.raises(AgentRequestError, match="未知字段"):
        validate_migration_input(_migration(scope="all"), ADAPTERS)


def test_metadata_input_accepts_full_patch():
    result = validate_metadata_input(
        _metadata(patch={
            "name": "n", "pinned": True, "archived": False, "tags": ["a"],
        }),
    )

    assert result["patch"] == {
        "name": "n", "pinned": True, "archived": False, "tags": ["a"],
    }


@pytest.mark.parametrize("patch, message", [
    ({}, "patch 字段非法"),
    ({"color": "red"}, "patch 字段非法"),
    ({"name": 1}, "name 非法"),
    ({"name": "长" * 201}, "name 非法"),
    ({"pinned": "yes"}, "pinned 必须是 boolean"),
    ({"tags": ["a"] * 21}, "tags 非法"),
    ({"tags": [""]}, "tags 非法"),
    ({"tags": "a"}, "tags 非法"),
])
def test_metadata_input_patch_rejections(patch, message):
    with pytest.raises(AgentRequestError, match=message):
        validate_metadata_input(_metadata(patch=patch))


def test_metadata_input_rejects_oversized_patch():
    with pytest.raises(AgentRequestError, match="4 KiB"):
        validate_metadata_input(_metadata(patch={
            "name": "名" * 200,
            "tags": ["标签" * 32] * 20,
        }))


def test_delete_input_is_minimal_and_unnormalized():
    assert validate_delete_input(
        {"kind": "delete", "tool": "claude", "ref": REF}, ADAPTERS,
    ) == {"kind": "delete", "tool": "claude", "ref": REF}


@pytest.mark.parametrize("value, message", [
    ({"kind": "delete", "tool": "nope", "ref": REF}, "tool 非法"),
    ({"kind": "delete", "tool": "claude", "ref": ""}, "ref 非法"),
    ({"kind": "delete", "tool": "claude", "ref": "a b"}, "ref 非法"),
    (
        {"kind": "delete", "tool": "claude", "ref": REF, "force": True},
        "未知字段",
    ),
])
def test_delete_input_rejections(value, message):
    with pytest.raises(AgentRequestError, match=message):
        validate_delete_input(value, ADAPTERS)


def test_cleanup_input_normalizes_optional_reason():
    assert validate_cleanup_input(_cleanup(), ADAPTERS) == _cleanup()
    assert validate_cleanup_input(_cleanup(
        targets=[{"tool": "claude", "ref": CLEANUP_REF}],
    ), ADAPTERS)["targets"] == [{"tool": "claude", "ref": CLEANUP_REF}]


@pytest.mark.parametrize("value", [
    _cleanup(scope_id="too-short"),
    _cleanup(targets=[]),
    _cleanup(targets=[{"tool": "missing", "ref": CLEANUP_REF}]),
    _cleanup(targets=[{"tool": "claude", "ref": "not-opaque"}]),
    _cleanup(targets=[{
        "tool": "claude", "ref": CLEANUP_REF, "reason": "x" * 301,
    }]),
    _cleanup(targets=[
        {"tool": "claude", "ref": CLEANUP_REF},
        {"tool": "claude", "ref": CLEANUP_REF},
    ]),
    _cleanup(extra=True),
])
def test_cleanup_input_rejects_invalid_samples(value):
    with pytest.raises(AgentRequestError):
        validate_cleanup_input(value, ADAPTERS)


def test_restore_delete_input_accepts_prefixed_id():
    recovery_id = "recovery_0123456789abcdef"

    assert validate_restore_delete_input(
        {"kind": "restore-delete", "recovery_id": recovery_id},
    ) == {"kind": "restore-delete", "recovery_id": recovery_id}


@pytest.mark.parametrize("value", [
    {"kind": "restore-delete"},
    {"kind": "restore-delete", "recovery_id": "recovery_1", "tool": "claude"},
    {"kind": "restore-delete", "recovery_id": "plan_0123456789abcdef"},
    {"kind": "restore-delete", "recovery_id": "recovery_short"},
    {"kind": "restore-delete", "recovery_id": "recovery_bad/chars/here!!"},
])
def test_restore_delete_input_rejections(value):
    with pytest.raises(AgentRequestError):
        validate_restore_delete_input(value)


def test_validate_ops_normalizes_assistant_reply():
    normalized = validate_ops([{
        "op": "replace-assistant-reply",
        "turn": 2,
        "reply": {"items": [{"kind": "text", "text": "答"}]},
    }])

    assert normalized == [{
        "op": "replace-assistant-reply",
        "turn": 2,
        "reply": {"items": [{"kind": "text", "text": "答"}]},
    }]


@pytest.mark.parametrize("ops, message", [
    ([], "1 到 50 项"),
    ("delete", "1 到 50 项"),
    ([{"op": "delete-turn"}] * 51, "1 到 50 项"),
    (["delete-turn"], "必须是 object"),
])
def test_validate_ops_shape_rejections(ops, message):
    with pytest.raises(AgentRequestError, match=message):
        validate_ops(ops)


@pytest.mark.parametrize("turn", [0, -1, True, "", 1.5])
def test_validate_ops_rejects_bad_reply_turn(turn):
    with pytest.raises(AgentRequestError, match="turn 参数非法"):
        validate_ops([{
            "op": "replace-assistant-reply",
            "turn": turn,
            "reply": {"items": [{"kind": "text", "text": "答"}]},
        }])


def test_validate_ops_rejects_duplicate_replaced_turn():
    operation = {
        "op": "replace-assistant-reply",
        "turn": 1,
        "reply": {"items": [{"kind": "text", "text": "答"}]},
    }

    with pytest.raises(AgentRequestError, match="重复替换"):
        validate_ops([operation, dict(operation)])


def test_validate_ops_rejects_malformed_reply():
    with pytest.raises(InvalidReplyError):
        validate_ops([{
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": []},
        }])
