"""与工具无关的会话变更事务。"""

from ..domain.errors import ConcurrentModificationError, OperationUnsupportedError


def preview_mutation(editor, ref: str, mutate) -> dict:
    doc = editor.load(ref)
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    return {"before": before, "after": editor.stats(doc), "changes": changes,
            "revision": doc.revision}


def apply_mutation(editor, ref: str, mutate, save_as: bool,
                   expected_revision: str | None = None):
    doc = editor.load(ref)
    if expected_revision is not None and doc.revision != expected_revision:
        raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    # 快照记下它救的是哪次编辑，还原界面才能说清「会失去什么」
    snapshot = None if save_as else editor.snapshot(
        doc, extra={"changes": changes, "before": before, "after": editor.stats(doc)})
    try:
        result = editor.save_copy(doc) if save_as else editor.commit(doc)
        result.update(ok=True, changes=changes,
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
        raise OperationUnsupportedError(
            editor.name, ",".join(op.get("op", "?") for op in ops),
            "saveas" if save_as else "inplace")
    return apply_mutation(editor, ref, lambda doc: editor.apply_ops(doc, ops), save_as)
