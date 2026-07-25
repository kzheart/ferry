from engine.adapters.grok.rewind import filter_rewind_updates


def _row(index, text):
    return {"method": "session/update", "params": {
        "update": {"kind": "message", "content": text},
        "_meta": {"promptIndex": index},
    }}


def test_rewind_removes_dead_tail_and_keeps_later_updates():
    rows = [
        _row(0, "root"), _row(1, "dead"),
        {"method": "session/rewind", "params": {
            "promptIndex": 1,
            "update": {"kind": "rewind_marker", "targetPromptIndex": 1},
        }},
        _row(0, "live"),
    ]
    visible = filter_rewind_updates(rows)
    assert [row["params"]["update"]["content"] for row in visible] == [
        "root", "live",
    ]
