"""Apply Grok rewind markers in file order."""


def _prompt_index(envelope):
    params = envelope.get("params") or {}
    meta = params.get("_meta") or {}
    value = meta.get("promptIndex")
    return value if isinstance(value, int) else None


def filter_rewind_updates(envelopes):
    visible = []
    for envelope in envelopes:
        params = envelope.get("params") or {}
        update = params.get("update") or {}
        kind = update.get("kind")
        if kind == "rewind_marker" or envelope.get("method") == "session/rewind":
            target = update.get("targetPromptIndex", params.get("promptIndex"))
            if isinstance(target, int):
                visible = [
                    item for item in visible
                    if _prompt_index(item) is None or _prompt_index(item) < target
                ]
            continue
        visible.append(envelope)
    return visible
