"""Apply Grok 0.2.106 rewind markers in file order."""


def _user_run_starts(envelopes):
    starts, saw_index, active_key, in_unindexed_run = [], False, None, False
    for position, envelope in enumerate(envelopes):
        params = envelope.get("params") or {}
        update = params.get("update") or {}
        nested = update.get("_meta") or {}
        is_user = (
            envelope.get("method") == "session/update"
            and update.get("sessionUpdate") == "user_message_chunk"
            and nested.get("hostTurn") is not True
        )
        if not is_user:
            in_unindexed_run = False
            continue
        prompt_index = nested.get("promptIndex")
        if isinstance(prompt_index, int):
            saw_index = True
            key = ("indexed", prompt_index)
            if key != active_key:
                starts.append(position)
                active_key = key
            in_unindexed_run = False
        elif not saw_index and not in_unindexed_run:
            starts.append(position)
            active_key = None
            in_unindexed_run = True
    return starts


def filter_rewind_updates(envelopes):
    visible = []
    for envelope in envelopes:
        params = envelope.get("params") or {}
        update = params.get("update") or {}
        is_marker = (
            envelope.get("method") == "_x.ai/session/update"
            and update.get("sessionUpdate") == "rewind_marker"
        )
        if not is_marker:
            visible.append(envelope)
            continue
        target = update.get("target_prompt_index")
        starts = _user_run_starts(visible)
        if isinstance(target, int) and 0 <= target < len(starts):
            visible = visible[:starts[target]]
        elif isinstance(target, int) and target == len(starts):
            pass
        else:
            visible.append(envelope)
    return visible
