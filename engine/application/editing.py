"""与工具无关的编辑编排。"""


def preview(editor, ref: str, ops: list[dict]) -> dict:
    doc = editor.load(ref)
    before = editor.stats(doc)
    notes = editor.apply_ops(doc, ops)
    editor.validate(doc)
    return {"before": before, "after": editor.stats(doc), "notes": notes,
            "revision": doc.revision, "capabilities": editor.capabilities()}


def apply(editor, ref: str, ops: list[dict], save_as: bool):
    if not editor.supports_mode(ops, save_as):
        mode = "另存为" if save_as else "原地编辑"
        raise ValueError(f"{editor.name} 不支持以{mode}执行当前操作组合")
    doc = editor.load(ref)
    notes = editor.apply_ops(doc, ops)
    editor.validate(doc)
    snapshot = None if save_as else editor.snapshot(doc)
    try:
        result = editor.save_copy(doc) if save_as else editor.commit(doc)
        result.update(ok=True, notes=notes, revision=doc.revision)
        if snapshot:
            result["snapshot"] = str(snapshot)
        return result, doc, snapshot
    except Exception:
        if snapshot:
            editor.restore_snapshot(snapshot, doc)
        raise
