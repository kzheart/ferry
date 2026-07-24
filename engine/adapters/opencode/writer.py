"""OpenCode 当前原生结构的 import 与失败回滚。"""

from pathlib import Path

from ...sessions.model import Session
from . import payload as payload_builder
from . import store as native_store
from .native_schema import templates


def write(
    sess: Session,
    cwd: str | None = None,
    tool_decider=None,
    native_payloads: dict[str, dict] | None = None,
) -> tuple[str, Path]:
    sessions = list(sess.walk())
    sid_map = {node.source_id: payload_builder.new_id("ses") for node in sessions}
    parent_map = {}
    for parent in sessions:
        for child in parent.children:
            parent_map[id(child)] = sid_map[parent.source_id]

    target_cwd = str(Path(cwd or sess.cwd).resolve())
    tpl = None
    prepared = []
    for node in sessions:
        sid = sid_map[node.source_id]
        node_cwd = (
            target_cwd
            if cwd is not None
            else str(Path(node.cwd or target_cwd).resolve())
        )
        parent_sid = parent_map.get(id(node))
        explicit_payload = (native_payloads or {}).get(node.source_id)
        payload = (
            payload_builder.clone(explicit_payload)
            if isinstance(explicit_payload, dict)
            else None
        )
        has_native_payload = payload is not None
        if payload is not None:
            if node.children:
                if tpl is None:
                    tpl = templates()
                # 原生 payload 尚未重映射时，edge.spawn_message_id 仍可精确定位。
                payload_builder.ensure_task_links(
                    payload,
                    node,
                    sid,
                    sid_map,
                    tpl,
                )
            payload = payload_builder.remap_payload(
                payload,
                sid,
                node_cwd,
                parent_sid,
                sid_map,
            )
        else:
            if tpl is None:
                tpl = templates()
            payload = payload_builder.canonical_payload(
                node,
                sid,
                node_cwd,
                parent_sid,
                tpl,
                sid_map=sid_map,
                tool_decider=tool_decider,
            )
        if node.children and not has_native_payload:
            if tpl is None:
                tpl = templates()
            payload_builder.ensure_task_links(
                payload,
                node,
                sid,
                sid_map,
                tpl,
            )
        prepared.append((payload, sid, node_cwd))

    imported = []
    try:
        for payload, sid, node_cwd in prepared:
            # import 可能先插入 session 再因消息 schema 失败；调用前登记，
            # 确保半写入的当前会话也进入回滚。
            imported.append(sid)
            native_store.import_payload(payload, sid, node_cwd)
    except Exception:
        for imported_sid in reversed(imported):
            try:
                native_store.delete_session(imported_sid, cwd=target_cwd)
            except Exception:
                pass
        raise

    return sid_map[sess.source_id], native_store.DB_PATH
