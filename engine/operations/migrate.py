"""迁移用例。

迁移是 Engine 的写入业务规则：它只依赖显式传入的应用端口，负责预览、
写入后的结构校验、可选影子探针、回滚与历史审计。它不读取全局 composition，
也不承担 RPC/CLI 的兼容门面职责。
"""
from __future__ import annotations

import time
from pathlib import Path

from ..adapters.base import narration
from ..context import EngineContext
from ..sessions import read as sessions
from . import history
from . import verification as probe_mod


class MigrationService:
    def __init__(self, ports: EngineContext):
        self._ports = ports

    def resume_command(self, tool: str, session_id: str, cwd: str) -> dict:
        return self._ports.adapter(tool).lifecycle.resume_descriptor(session_id, cwd)

    def preview(
            self, src: str, dst: str, ref: str,
            cwd: str | None = None, max_turn: int | None = None,
            probe_model: str | None = None, content_locale: str | None = None,
            *, session=None,
    ) -> dict:
        source, target, target_cwd, base = self._prepare(
            src, dst, ref, cwd, max_turn, probe_model, session=session,
        )
        with narration.content_locale(content_locale):
            preview = target.preview(source, target_cwd)
        return {**base, "preview": preview}

    def apply(
            self, src: str, dst: str, ref: str,
            cwd: str | None = None, probe: bool = False,
            max_turn: int | None = None, probe_model: str | None = None,
            content_locale: str | None = None, *, session=None,
    ) -> dict:
        source, target, target_cwd, base = self._prepare(
            src, dst, ref, cwd, max_turn, probe_model, session=session,
        )
        with narration.content_locale(content_locale):
            session_id, destination = target.write(source, target_cwd)
        artifact_active = True
        try:
            base["loss"] = target.plan(source)
            result = {
                **base,
                "session_id": session_id,
                "dest": str(destination),
                "resume": self.resume_command(dst, session_id, target_cwd),
            }
            ok, tree_detail = self.validate_written_tree(
                dst, session_id, destination, _tree_shape(source),
            )
            validation = {
                "structure": {"ok": ok, "detail": tree_detail},
                "runtime": {"status": "skipped"},
            }
            runtime_report = None
            if ok and probe:
                with narration.content_locale(content_locale):
                    runtime_report = self._isolated_probe(
                        dst, source, target_cwd, model=probe_model,
                    )
                validation["runtime"] = {
                    **runtime_report,
                    "model": probe_model or None,
                }
                ok = runtime_report["status"] == "passed"
            result["validation"] = validation
            if probe or not ok:
                result["probe"] = runtime_report or {
                    "status": "passed" if ok else "failed",
                    "code": None if ok else "probe.structure_invalid",
                    "params": {},
                    "diagnostic": {
                        "stdout": tree_detail,
                        "stderr": "",
                        "truncated": False,
                    },
                }
                if probe:
                    result["probe"]["model"] = probe_model or None
            if not ok:
                self._cleanup_artifact(dst, session_id, destination)
                artifact_active = False
                result["rolled_back"] = True
            history.append({**result, "time": int(time.time() * 1000)}, self._ports)
            return result
        except Exception:
            if artifact_active:
                self._cleanup_artifact(dst, session_id, destination)
            raise

    def _prepare(
            self, src: str, dst: str, ref: str, cwd: str | None,
            max_turn: int | None, probe_model: str | None, *, session=None,
    ):
        source = session if session is not None else sessions.read_tree(src, ref, self._ports)
        if max_turn:
            _truncate_rounds(source, int(max_turn))
        target = self._ports.adapter(dst).migration_target
        target_cwd = str(Path(cwd or source.cwd or ".").resolve())
        stats = target.plan(source)
        tree_count, message_count = _migration_counts(source)
        edge_count = sum(len(node.agent_edges) for node in source.walk())
        topology = {
            "nodes": tree_count,
            "edges": max(0, tree_count - 1),
            "agent_edges": edge_count,
            "preserved": True,
            "detail": "父子会话关系将按原拓扑写入" if tree_count > 1
            else "普通单会话,无子会话拓扑",
        }
        return source, target, target_cwd, {
            "src": src,
            "dst": dst,
            "source_id": source.source_id,
            "title": source.title,
            "cwd": target_cwd,
            "loss": stats,
            "tree_count": tree_count,
            "child_count": tree_count - 1,
            "topology": topology,
            "max_turn": max_turn,
            "msg_count": message_count,
            "root_msg_count": len(source.messages),
            "probe_model": probe_model or None,
        }

    def _isolated_probe(
            self, dst: str, source, cwd: str, *, model: str | None = None,
    ) -> dict:
        saved_loss = [(node, list(node.loss)) for node in source.walk()]
        shadow_session_id = shadow_destination = None
        try:
            shadow_session_id, shadow_destination = self._ports.adapter(dst).migration_target.write(source, cwd)
            report = self.run_probe(dst, shadow_session_id, cwd, model=model)
            report.setdefault("isolation", {
                "kind": "shadow_copy",
                "id": shadow_session_id,
                "cleaned": True,
            })
            return report
        finally:
            for node, loss in saved_loss:
                node.loss = loss
            if shadow_session_id is not None:
                self._cleanup_artifact(dst, shadow_session_id, shadow_destination)

    def run_probe(self, tool: str, session_id: str, cwd: str, *, model: str | None = None) -> dict:
        try:
            return probe_mod.run_probe(
                tool,
                session_id,
                self._ports.adapter(tool).lifecycle.probe_cwd(cwd),
                model,
                ports=self._ports,
            )
        except probe_mod.ProbeTimeout as error:
            return probe_mod.timeout_report(tool, error)

    def validate_written_tree(
            self, tool: str, session_id: str, destination, expected_shape: tuple,
    ) -> tuple[bool, str]:
        try:
            adapter = self._ports.adapter(tool)
            ref = adapter.lifecycle.validation_ref(session_id, destination)
            restored = adapter.browser.read(ref)
            nodes = list(restored.walk())
            ids = [node.source_id for node in nodes]
            edge_count = sum(len(node.children) for node in nodes)
            expected = 1 + sum(1 for _ in _shape_nodes(expected_shape))
            shape_matches = _tree_shape(restored) == expected_shape
            ok = (
                len(nodes) == expected
                and len(set(ids)) == expected
                and edge_count == max(0, expected - 1)
                and shape_matches
            )
            detail = (
                f"树结构验收: 节点 {len(nodes)}/{expected}, "
                f"父子边 {edge_count}/{max(0, expected - 1)}, "
                f"层级拓扑 {'一致' if shape_matches else '不一致'}"
            )
            return ok, detail
        except Exception as error:
            return False, f"树结构验收失败: {error}"

    def _cleanup_artifact(self, dst: str, session_id: str, destination) -> None:
        self._ports.adapter(dst).lifecycle.cleanup(session_id, destination)


def _truncate_rounds(session, max_turn: int):
    kept, turn = [], 0
    for message in session.messages:
        if message.role == "user":
            turn += 1
        if turn > max_turn:
            break
        kept.append(message)
    dropped = len(session.messages) - len(kept)
    if dropped:
        session.lose("migration.truncated", max_turn=max_turn, dropped=dropped)
    session.messages = kept
    kept_ids = {message.source_id for message in kept if message.source_id}
    children_by_id = {child.source_id: child for child in session.children}
    edges, kept_children = [], set()
    for edge in session.agent_edges:
        if (
            edge.child_session_id not in children_by_id
            or not edge.spawn_message_id
            or edge.spawn_message_id not in kept_ids
            or edge.child_session_id in kept_children
        ):
            continue
        edges.append(edge)
        kept_children.add(edge.child_session_id)
    children = [
        child for child_id, child in children_by_id.items()
        if child_id in kept_children
    ]
    removed = len(session.children) - len(children)
    if removed:
        session.lose("migration.children_not_migrated", count=removed)
    session.children = children
    session.agent_edges = edges
    return session


def _migration_counts(session) -> tuple[int, int]:
    return sum(1 for _ in session.walk()), session.message_count()


def _tree_shape(session) -> tuple:
    return tuple(sorted((_tree_shape(child) for child in session.children), key=repr))


def _shape_nodes(shape):
    for child in shape:
        yield child
        yield from _shape_nodes(child)
