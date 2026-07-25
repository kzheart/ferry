from engine.adapters.grok.updates import aggregate_updates


def _event(
    *,
    update_type="ToolCallUpdate",
    prompt_id=None,
    prompt_index=None,
    call_id=None,
    kind=None,
    status=None,
    raw_input=None,
    raw_output=None,
    tool_name=None,
):
    update = {}
    if kind is not None:
        update["kind"] = kind
    if raw_input is not None:
        update["rawInput"] = raw_input
    if raw_output is not None:
        update["rawOutput"] = raw_output
    if tool_name is not None:
        update["_meta"] = {"x.ai/tool": {"name": tool_name}}
    meta = {"updateType": update_type}
    if prompt_id is not None:
        meta["promptId"] = prompt_id
    if prompt_index is not None:
        meta["promptIndex"] = prompt_index
    update_params = {}
    if call_id is not None:
        update_params["toolCallId"] = call_id
    if kind is not None:
        update_params["kind"] = kind
    if status is not None:
        update_params["status"] = status
    if update_params:
        meta["updateParams"] = update_params
    return {"method": "session/update", "params": {
        "update": update,
        "_meta": meta,
    }}


def test_missing_prompt_updates_merge_into_the_anchored_tool_call():
    prompts = aggregate_updates([
        _event(call_id="shell-1", raw_output="first"),
        _event(
            update_type="ToolCall",
            prompt_id="p1",
            prompt_index=0,
            call_id="shell-1",
            raw_input={"command": "pwd"},
            tool_name="Shell",
            status="Pending",
        ),
        _event(
            prompt_id="p1",
            call_id="shell-1",
            raw_output="complete",
            status="Completed",
        ),
        _event(call_id="shell-1", raw_output="complete\nlate", status="InProgress"),
    ])

    assert len(prompts) == 1
    assert prompts[0]["blocks"] == [{"kind": "tool", "id": "shell-1"}]
    assert prompts[0]["tools"]["shell-1"] == {
        "id": "shell-1",
        "name": "Shell",
        "input": {"command": "pwd"},
        "output": "complete\nlate",
        "status": "completed",
    }
    assert prompts[0]["unknown"] == []


def test_parallel_missing_prompt_updates_follow_call_id_not_event_order():
    prompts = aggregate_updates([
        _event(
            update_type="ToolCall",
            prompt_id="p1",
            call_id="a",
            kind="Shell",
            raw_input={"command": "one"},
        ),
        _event(
            update_type="ToolCall",
            prompt_id="p2",
            call_id="b",
            kind="Shell",
            raw_input={"command": "two"},
        ),
        _event(call_id="b", raw_output="two output"),
        _event(call_id="a", raw_output="one output"),
    ])

    assert [prompt["id"] for prompt in prompts] == ["p1", "p2"]
    assert prompts[0]["tools"]["a"]["output"] == "one output"
    assert prompts[1]["tools"]["b"]["output"] == "two output"
    assert "b" not in prompts[0]["tools"]
    assert "a" not in prompts[1]["tools"]


def test_orphan_tool_update_is_diagnostic_only():
    prompts = aggregate_updates([
        _event(call_id="orphan", raw_output="cannot place this safely"),
    ])

    assert len(prompts) == 1
    assert prompts[0]["id"] == "prompt:unassigned"
    assert prompts[0]["blocks"] == []
    assert prompts[0]["tools"] == {}
    assert len(prompts[0]["unknown"]) == 1
