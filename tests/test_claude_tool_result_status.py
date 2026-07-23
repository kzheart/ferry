import pytest

from engine.adapters.claude.reader import _result_status


@pytest.mark.parametrize(
    ("native_status", "canonical_status"),
    [
        ("success", "success"),
        ("completed", "success"),
        ("teammate_spawned", "success"),
        ("error", "error"),
        ("interrupted", "interrupted"),
        ("running", "running"),
        ("async_launched", "running"),
        ("pending", "pending"),
        ("failed", "unknown"),
        ("future_status", "unknown"),
        (None, "unknown"),
    ],
)
def test_claude_maps_only_current_native_result_statuses(
        native_status, canonical_status):
    assert _result_status({}, {"status": native_status}) == canonical_status


def test_claude_result_without_native_status_is_success():
    assert _result_status({}, {}) == "success"


def test_claude_native_error_and_interruption_flags_take_precedence():
    assert _result_status(
        {"is_error": True}, {"status": "completed"},
    ) == "error"
    assert _result_status(
        {}, {"status": "completed", "interrupted": True},
    ) == "interrupted"
    assert _result_status(
        {}, {"status": "completed", "success": False},
    ) == "error"
