"""与工具无关的会话变更事务。"""

from ..domain.errors import ConcurrentModificationError


def preview_mutation(editor, ref: str, mutate) -> dict:
    doc = editor.load(ref)
    before = editor.stats(doc)
    notes = mutate(doc)
    editor.validate(doc)
    return {"before": before, "after": editor.stats(doc), "notes": notes,
            "revision": doc.revision}


def apply_mutation(editor, ref: str, mutate, save_as: bool,
                   expected_revision: str | None = None):
    doc = editor.load(ref)
    if expected_revision is not None and doc.revision != expected_revision:
        raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
    notes = mutate(doc)
    editor.validate(doc)
    snapshot = None if save_as else editor.snapshot(doc)
    try:
        result = editor.save_copy(doc) if save_as else editor.commit(doc)
        result.update(ok=True, notes=notes,
                      revision=editor.saved_revision(result, doc))
        if snapshot:
            result["snapshot"] = str(snapshot)
        return result, doc, snapshot
    except ConcurrentModificationError:
        raise
    except Exception:
        if snapshot:
            editor.restore_snapshot(snapshot, doc)
        raise


def preview(editor, ref: str, ops: list[dict]) -> dict:
    result = preview_mutation(editor, ref, lambda doc: editor.apply_ops(doc, ops))
    result["capabilities"] = editor.capabilities()
    return result


def apply(editor, ref: str, ops: list[dict], save_as: bool):
    if not editor.supports_mode(ops, save_as):
        mode = "另存为" if save_as else "原地编辑"
        raise ValueError(f"{editor.name} 不支持以{mode}执行当前操作组合")
    return apply_mutation(editor, ref, lambda doc: editor.apply_ops(doc, ops), save_as)
