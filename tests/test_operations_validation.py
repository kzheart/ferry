"""operations/validation.py 的形状校验回归网（纯函数，不触发任何 IO）。"""
import pytest

from engine.errors import AgentRequestError, InvalidReplyError
from engine.operations.validation import (
    validate_delete_input,
    validate_edit_input,
    validate_metadata_input,
    validate_migration_input,
    validate_ops,
)


ADAPTERS = ("claude", "opencode", "codex")
REF = "ref-abcdef0123456789"
DELETE_REF = "fsr_0123456789abcdef"


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


def test_delete_input_accepts_unique_opaque_refs():
    assert validate_delete_input(
        {"kind": "delete", "tool": "claude", "refs": [DELETE_REF]}, ADAPTERS,
    ) == {"kind": "delete", "tool": "claude", "refs": [DELETE_REF]}


@pytest.mark.parametrize("value, message", [
    ({"kind": "delete", "tool": "nope", "refs": [DELETE_REF]}, "tool 非法"),
    ({"kind": "delete", "tool": "claude", "refs": []}, "1 到 100 项"),
    ({"kind": "delete", "tool": "claude", "refs": [""]}, "ref 非法"),
    ({"kind": "delete", "tool": "claude", "refs": ["not-opaque"]}, "ref 非法"),
    (
        {"kind": "delete", "tool": "claude", "refs": [DELETE_REF, DELETE_REF]},
        "不允许重复",
    ),
    (
        {
            "kind": "delete",
            "tool": "claude",
            "refs": [f"{DELETE_REF}{index:04d}" for index in range(101)],
        },
        "1 到 100 项",
    ),
    (
        {"kind": "delete", "tool": "claude", "refs": [DELETE_REF], "force": True},
        "未知字段",
    ),
])
def test_delete_input_rejections(value, message):
    with pytest.raises(AgentRequestError, match=message):
        validate_delete_input(value, ADAPTERS)


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
