"""与工具无关的会话变更事务。"""

from ..domain.errors import ConcurrentModificationError, OperationUnsupportedError


def preview_mutation(editor, ref: str, mutate, loader=None) -> dict:
    doc = (loader or editor.load)(ref)
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    return {"before": before, "after": editor.stats(doc), "changes": changes,
            "revision": doc.revision}


def apply_mutation(editor, ref: str, mutate,
                   expected_revision: str | None = None):
    doc = editor.load(ref)
    if expected_revision is not None and doc.revision != expected_revision:
        raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    # 快照记下它救的是哪次编辑，还原界面才能说清「会失去什么」
    snapshot = editor.snapshot(
        doc, extra={"changes": changes, "before": before, "after": editor.stats(doc)})
    if not snapshot:
        raise RuntimeError("原地编辑无法创建恢复快照，已取消写入")
    try:
        result = editor.commit(doc)
        result.update(ok=True, changes=changes,
                      revision=editor.saved_revision(result, doc))
        if snapshot:
            result["snapshot"] = str(snapshot)
        return result, doc, snapshot
    except ConcurrentModificationError:
        raise
    except Exception:
        editor.restore_snapshot(snapshot, doc)
        raise


def preview(editor, ref: str, ops: list[dict], loader=None) -> dict:
    result = preview_mutation(
        editor, ref, lambda doc: editor.apply_ops(doc, ops), loader=loader)
    result["capabilities"] = editor.capabilities()
    return result


def apply(editor, ref: str, ops: list[dict],
          expected_revision: str | None = None):
    modes = editor.capabilities().get("operation_modes", {})
    if not all("inplace" in modes.get(op.get("op"), []) for op in ops):
        raise OperationUnsupportedError(
            editor.name, ",".join(op.get("op", "?") for op in ops),
            "inplace")
    return apply_mutation(
        editor, ref, lambda doc: editor.apply_ops(doc, ops),
        expected_revision=expected_revision)
