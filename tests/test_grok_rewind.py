from engine.adapters.grok.rewind import filter_rewind_updates


def _row(index, text):
    return {"method": "session/update", "params": {
        "update": {"sessionUpdate": "user_message_chunk",
                   "content": {"type": "text", "text": text},
                   "_meta": {"promptIndex": index}},
        "_meta": {},
    }}


def test_rewind_removes_dead_tail_and_keeps_later_updates():
    rows = [
        _row(0, "root"), _row(1, "dead"),
        {"method": "_x.ai/session/update", "params": {
            "update": {"sessionUpdate": "rewind_marker",
                       "target_prompt_index": 1},
        }},
        _row(0, "live"),
    ]
    visible = filter_rewind_updates(rows)
    assert [row["params"]["update"]["content"]["text"] for row in visible] == [
        "root", "live",
    ]
