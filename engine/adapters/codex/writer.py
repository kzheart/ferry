"""Codex writer:规范化中间格式 → rollout JSONL(可被 codex exec resume 加载)。

写入 Codex 原生 JSONL 会话记录。核心策略:
- 结构模板来自声明式格式配置档(session_meta / 各类 response_item),
  真实 CLI 样本仅用于测试配置档与原生格式保持一致。
- shell.exec 原生映射为 exec_command;fs.write 映射为 apply_patch(Add File);
  其余工具降级为叙述文本(narration)。
"""
import json
import secrets
import shlex
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

from ...domain.model import Session
from ...domain.tool_ops import CanonicalOp, has_valid_tool_input
from ..base.narration import narrate
from .formats import FORMATS
from .registry import register_tree



def _uuid7() -> str:
    ts = int(time.time() * 1000)
    b = ts.to_bytes(6, "big") + secrets.token_bytes(10)
    b = bytearray(b)
    b[6] = (b[6] & 0x0F) | 0x70
    b[8] = (b[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(b)))


def _now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + \
        f".{int(time.time()*1000)%1000:03d}Z"


def _timestamp(value: str | int | None = None) -> str:
    """保留 canonical 时间；仅在来源缺失时生成当前 UTC 时间。"""
    if isinstance(value, str) and value.strip():
        return value
    if isinstance(value, (int, float)):
        seconds = value / 1000 if value > 10_000_000_000 else value
        return datetime.fromtimestamp(seconds, timezone.utc).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
    return _now_iso()


def _load_templates():
    """Load the latest declarative Codex format profile."""
    return FORMATS.templates()


def _clone(tpl: dict) -> dict:
    return json.loads(json.dumps(tpl))


def _msg(tpl, role: str, text: str, created_at: str | int | None = None) -> dict:
    rec = _clone(tpl[f"message.{role}"])
    rec["timestamp"] = _timestamp(created_at)
    p = rec["payload"]
    p["content"] = [{"type": "input_text" if role == "user" else "output_text",
                     "text": text}]
    # 原生 user message 不携带 id；assistant message 以 msg_* 标识，
    # 供 Codex 恢复时关联响应链路。不要写 null，严格反序列化器会区分
    # “字段缺失”和“字段类型错误”。
    if role == "user":
        p.pop("id", None)
    else:
        p["id"] = "msg_" + secrets.token_hex(25)
    return rec


def _result_payload(tool, output: str, exit_code=None) -> dict:
    result = tool.result
    if result is None:
        payload = {"output": output}
        code = tool.meta.get("exit_code", tool.meta.get("exit", exit_code))
        if isinstance(code, int) and not isinstance(code, bool):
            payload["exit_code"] = code
        if isinstance(tool.meta.get("stderr"), str):
            payload["stderr"] = tool.meta["stderr"]
        if isinstance(tool.meta.get("truncated"), bool):
            payload["truncated"] = tool.meta["truncated"]
        return payload
    payload = {
        "status": result.status,
        "output": result.legacy_output(),
        "canonical_blocks": [
            {
                "kind": block.kind,
                "text": block.text,
                "data": block.data,
                "mime_type": block.mime_type,
                "filename": block.filename,
                "uri": block.uri,
                "metadata": block.metadata,
            }
            for block in result.blocks
        ],
        "attachments": result.attachments,
    }
    if result.stdout is not None:
        payload["stdout"] = result.stdout
    if result.stderr is not None:
        payload["stderr"] = result.stderr
    if result.exit_code is not None:
        payload["exit_code"] = result.exit_code
    if result.truncated is not None:
        payload["truncated"] = result.truncated
    if result.metadata:
        payload["canonical_metadata"] = result.metadata
    return payload


def _exec_pair(tpl, cmd: str, workdir: str, stdout: str, exit_code,
               started_at: str | int | None = None,
               ended_at: str | int | None = None,
               result_payload: dict | None = None) -> list:
    call = _clone(tpl["response_item.custom_tool_call"])
    out = _clone(tpl["response_item.custom_tool_call_output"])
    call_id = "call_" + secrets.token_urlsafe(18)[:24]
    call["timestamp"] = _timestamp(started_at)
    out["timestamp"] = _timestamp(ended_at or started_at)
    cp, op = call["payload"], out["payload"]
    cp["id"] = "ctc_" + secrets.token_hex(25)
    cp["call_id"] = op["call_id"] = call_id
    cp["name"] = "exec"
    args = json.dumps({"cmd": cmd, "workdir": workdir,
                       "yield_time_ms": 10000, "max_output_tokens": 1000})
    cp["input"] = (f"const r = await tools.exec_command({args});\n"
                   "text(JSON.stringify(r));\n")
    inner_value = {
        "chunk_id": secrets.token_hex(3),
        "wall_time_seconds": 0.01,
        "original_token_count": max(1, len(stdout) // 4),
        "output": stdout,
    }
    if exit_code is not None:
        inner_value["exit_code"] = exit_code
    if result_payload:
        inner_value.update(result_payload)
    inner = json.dumps(inner_value, ensure_ascii=False)
    op["id"] = "fco_" + _uuid7()
    op["output"] = json.dumps([
        {"type": "input_text",
         "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
        {"type": "input_text", "text": inner}])
    op.pop("internal_chat_message_metadata_passthrough", None)
    return [call, out]


def _apply_patch_pair(tpl, patch: str, output: str = "{}",
                      result_payload: dict | None = None) -> list:
    call, outrec = _exec_pair(tpl, "", "", "{}", 0)
    call["payload"]["input"] = (f"const patch = {json.dumps(patch)};\n"
                                "text(await tools.apply_patch(patch));\n")
    payload = result_payload or {"output": output}
    outrec["payload"]["output"] = json.dumps([
        {"type": "input_text",
         "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
        {"type": "input_text", "text": json.dumps(payload, ensure_ascii=False)}])
    return [call, outrec]


def _write_shell_exec(tpl, t, cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    if i.get("command"):
        return _exec_pair(tpl, i["command"], i.get("workdir") or cwd,
                          t.meta.get("stdout", t.output),
                          t.meta.get("exit_code"),
                          t.started_at, t.ended_at,
                          _result_payload(t, t.output))


def _write_fs_read(tpl, t, cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    if i.get("file_path"):
        command = f"cat {shlex.quote(str(i['file_path']))}"
        return _exec_pair(tpl, command, cwd, t.output or "",
                          t.tool_result.exit_code,
                          t.started_at, t.ended_at,
                          _result_payload(t, t.output))
    return None


def _write_fs_write(tpl, t, _cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    if i.get("file_path"):
        body = str(i.get("content", ""))
        patch = "*** Begin Patch\n*** Add File: {}\n{}\n*** End Patch".format(
            i["file_path"], "\n".join("+" + l for l in body.splitlines()))
        return _apply_patch_pair(tpl, patch, t.output or "{}",
                                 _result_payload(t, t.output))
    return None


def _write_fs_edit(tpl, t, _cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    if i.get("file_path"):
        hunk = "\n".join(["@@"]
                          + ["-" + l for l in str(i.get("old", "")).splitlines()]
                          + ["+" + l for l in str(i.get("new", "")).splitlines()])
        patch = "*** Begin Patch\n*** Update File: {}\n{}\n*** End Patch".format(
            i["file_path"], hunk)
        return _apply_patch_pair(tpl, patch, t.output or "{}",
                                 _result_payload(t, t.output))
    return None


def _write_fs_patch(tpl, t, _cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    patch = i.get("raw_patch")
    if not patch:
        return None
    return _apply_patch_pair(tpl, str(patch), t.output or "{}",
                             _result_payload(t, t.output))


def _write_fs_search(tpl, t, cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    query = i.get("query")
    if not query:
        return None
    command = ["rg", "--line-number", "--color", "never"]
    if i.get("glob"):
        command.extend(["-g", str(i["glob"])])
    command.extend(["--", str(query), str(i.get("path") or ".")])
    quoted = " ".join(shlex.quote(part) for part in command)
    return _exec_pair(tpl, quoted, i.get("workdir") or cwd,
                      t.output or "", t.tool_result.exit_code,
                      t.started_at, t.ended_at,
                      _result_payload(t, t.output))


def _write_fs_glob(tpl, t, cwd) -> list | None:
    i = t.input if isinstance(t.input, dict) else {}
    pattern = i.get("pattern")
    if not pattern:
        return None
    command = ["rg", "--files", "-g", str(pattern), "--",
               str(i.get("path") or ".")]
    quoted = " ".join(shlex.quote(part) for part in command)
    return _exec_pair(tpl, quoted, cwd, t.output or "",
                      t.tool_result.exit_code,
                      t.started_at, t.ended_at,
                      _result_payload(t, t.output))


OP_WRITERS = {
    CanonicalOp.SHELL_EXEC: _write_shell_exec,
    CanonicalOp.FS_READ: _write_fs_read,
    CanonicalOp.FS_WRITE: _write_fs_write,
    CanonicalOp.FS_EDIT: _write_fs_edit,
    CanonicalOp.FS_PATCH: _write_fs_patch,
    CanonicalOp.FS_SEARCH: _write_fs_search,
    CanonicalOp.FS_GLOB: _write_fs_glob,
}

OP_FIDELITY = {op: "native" for op in OP_WRITERS} | {
    # Codex has no native read tool. Rendering it as `cat` preserves content
    # but changes the tool semantics, so migration preview must disclose it.
    CanonicalOp.FS_READ: "degrade",
    CanonicalOp.FS_SEARCH: "degrade",
    CanonicalOp.FS_GLOB: "degrade",
    CanonicalOp.WEB_FETCH: "degrade",
    CanonicalOp.WEB_SEARCH: "degrade",
    CanonicalOp.TOOL_INVOKE: "degrade",
    CanonicalOp.AGENT_SPAWN: "native",
}


def _native_records(tpl, t, cwd,
                    message_time: str | int | None = None) -> list | None:
    """Render a canonical operation with the target-specific mapping table."""
    writer = OP_WRITERS.get(t.op)
    if writer is None or not has_valid_tool_input(t.op, t.input):
        return None
    records = writer(tpl, t, cwd)
    if not records:
        return records
    # Claude 等来源通常只有消息时间、没有独立工具时间。此时沿用所属
    # 消息的时间，避免历史工具记录被标成迁移时刻。
    if t.started_at is None and message_time is not None:
        records[0]["timestamp"] = _timestamp(message_time)
    if t.ended_at is None and message_time is not None:
        records[-1]["timestamp"] = _timestamp(t.started_at or message_time)
    return records


def _session_records(tpl, sess: Session, cwd: str, sid: str, root_id: str,
                     parent_id: str | None, depth: int,
                     agent_path: str | None, child_links: dict[str, list],
                     tool_decider=None) -> list[dict]:
    now = _timestamp(next((message.created_at for message in sess.messages
                           if message.created_at is not None), None))
    meta = _clone(tpl["session_meta"])
    meta["timestamp"] = now
    mp = meta["payload"]
    mp["id"] = sid
    mp["session_id"] = root_id
    mp["timestamp"] = now
    mp["cwd"] = cwd
    mp["originator"] = "codex-tui"
    mp["source"] = "cli"
    mp["thread_source"] = "user"
    mp["model_provider"] = "openai"
    mp["memory_mode"] = "enabled"
    mp["history_mode"] = "legacy"
    mp["agent_path"] = agent_path
    if parent_id:
        mp["parent_thread_id"] = parent_id
        mp["forked_from_id"] = parent_id
        mp["source"] = {
            "subagent": {
                "thread_spawn": {
                    "parent_thread_id": parent_id,
                    "agent_path": agent_path,
                    "depth": depth,
                    "agent_nickname": sess.meta.get("agent_nickname"),
                    "agent_role": sess.meta.get("agent_role"),
                },
            }
        }
        mp["thread_source"] = "subagent"

    # 缺省 turn_context 会由 Codex 按当前配置恢复；字段不全的记录会使
    # 新版 TUI 严格反序列化失败。
    out_lines = [meta]
    for m in sess.messages:
        texts = []
        role = m.role if m.role in ("user", "assistant") else "user"
        for b in m.blocks:
            if b.kind == "text":
                text = b.text
                if m.role not in ("user", "assistant"):
                    text = f"[{m.role}]\n{text}"
                texts.append(text)
            elif b.kind == "tool":
                t = b.tool
                decision = tool_decider(t, sess, m) if tool_decider else None
                if t.op == CanonicalOp.AGENT_SPAWN:
                    if decision is not None and decision.rendered is None:
                        sess.lose(
                            "migration.tool_degraded", tool_name=t.name,
                            fidelity=decision.fidelity,
                            reason_codes=list(decision.reason_codes),
                            ignored_fields=sorted(decision.ignored_fields))
                        texts.append(narrate(t))
                    continue
                native = (_native_records(tpl, t, cwd, m.created_at)
                          if decision is None or decision.rendered is not None
                          else None)
                if native:
                    if texts:
                        out_lines.append(_msg(tpl, role, "\n\n".join(texts),
                                              m.created_at))
                        texts = []
                    out_lines += native
                else:
                    params = {"tool_name": t.name}
                    if decision is not None:
                        params.update({
                            "fidelity": decision.fidelity,
                            "reason_codes": list(decision.reason_codes),
                            "ignored_fields": sorted(decision.ignored_fields),
                        })
                    sess.lose("migration.tool_degraded", **params)
                    texts.append(narrate(t))
        if texts:
            out_lines.append(_msg(tpl, role, "\n\n".join(texts), m.created_at))
        # 子会话 spawn 是父消息的一部分；写在对应消息之后而非会话尾部。
        if m.source_id:
            for child_link in child_links.pop(m.source_id, []):
                out_lines.extend(child_link(m.created_at))
    # 缺少可定位消息的旧来源退化为追加；仍保留确定性 child 顺序。
    for source_id in sorted(child_links):
        for child_link in child_links[source_id]:
            out_lines.extend(child_link(None))
    return out_lines


def _assistant_result(sess: Session) -> str:
    for message in reversed(sess.messages):
        if message.role != "assistant":
            continue
        text = "\n".join(block.text for block in message.blocks
                         if block.kind == "text" and block.text)
        if text:
            return text
    return ""


def _edge_for(parent: Session, child: Session):
    return next((edge for edge in parent.agent_edges
                 if edge.child_session_id == child.source_id), None)


def _edge_status(edge) -> str:
    status = (edge.status if edge else None) or "closed"
    if status in {"open", "closed"}:
        return status
    return "closed" if status.lower() in {
        "completed", "complete", "done", "finished", "failed", "cancelled", "canceled",
    } else "open"


def _child_link_records(parent: Session, child: Session, child_id: str,
                        agent_path: str, created_at: str | int | None = None) -> list[dict]:
    edge = _edge_for(parent, child)
    call_id = (edge.source_call_id if edge and edge.source_call_id else
               "call_" + secrets.token_urlsafe(18)[:24])
    prompt = edge.prompt if edge else ""
    agent_type = (edge.agent_type if edge and edge.agent_type else
                  child.agent_type)
    arguments = {"message": prompt}
    if agent_type:
        arguments["agent_type"] = agent_type
    now = _timestamp(created_at)
    status = _edge_status(edge)
    call_status = "completed" if status == "closed" else "in_progress"
    spawn = {
        "timestamp": now,
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "id": "fc_" + secrets.token_hex(25),
            "name": "spawn_agent",
            "arguments": json.dumps(arguments, ensure_ascii=False),
            "call_id": call_id,
            # response_item 使用 Responses API 的状态枚举；SQLite 的
            # thread_spawn_edges 则使用 open/closed，二者不能混写。
            "status": call_status,
        },
    }
    activity = {
        "timestamp": now,
        "type": "event_msg",
        "payload": {
            "type": "sub_agent_activity",
            "agent_thread_id": child_id,
            "agent_path": agent_path,
            "kind": "completed" if status == "closed" else "working",
        },
    }
    result_text = _assistant_result(child)
    result = {
        "timestamp": now,
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": call_id,
            "output": json.dumps({"task_name": agent_path},
                                  ensure_ascii=False),
        },
    }
    agent_message = {
        "timestamp": now,
        "type": "response_item",
        "payload": {
            "type": "agent_message",
            "id": "amsg_" + secrets.token_hex(12),
            "author": agent_path,
            "recipient": "/root",
            "content": [{"type": "input_text", "text": result_text}],
        },
    }
    event_message = {
        "timestamp": now,
        "type": "event_msg",
        "payload": {"type": "agent_message", "message": result_text},
    }
    return [spawn, activity, result, event_message, agent_message]


def _destination(sessions_dir: Path, sid: str, ordinal: int) -> Path:
    day = time.strftime("%Y/%m/%d")
    stamp = time.strftime("%Y-%m-%dT%H-%M-%S")
    suffix = f"-{ordinal}" if ordinal else ""
    return sessions_dir / day / f"rollout-{stamp}{suffix}-{sid}.jsonl"


def write(sess: Session, cwd: str | None = None,
          sessions_dir: str | Path | None = None,
          state_db: str | Path | None = None,
          tool_decider=None) -> tuple[str, Path]:
    """写出整棵 rollout 树,返回根会话的 (session_id, 文件路径)。"""
    tpl = _load_templates()
    root_id = _uuid7()
    base_cwd = cwd or sess.cwd
    output_root = (Path(sessions_dir).expanduser() if sessions_dir else
                   Path.home() / ".codex" / "sessions")
    nodes = list(sess.walk())
    ids = {id(node): root_id if node is sess else _uuid7() for node in nodes}
    paths = {}
    parents = {}
    working_dirs = {}
    pending_paths = set()
    agent_paths = {id(sess): sess.agent_path or "/root"}
    edge_statuses = {id(sess): None}

    def assign_tree_fields(node: Session, agent_path: str):
        agent_paths[id(node)] = agent_path
        used_paths = set()
        for child_index, child in enumerate(node.children):
            base_path = child.agent_path or f"{agent_path}/{child.agent_id or child_index + 1}"
            child_path = base_path
            suffix = 2
            while child_path in used_paths:
                child_path = f"{base_path}-{suffix}"
                suffix += 1
            used_paths.add(child_path)
            edge_statuses[id(child)] = _edge_status(_edge_for(node, child))
            assign_tree_fields(child, child_path)

    assign_tree_fields(sess, agent_paths[id(sess)])

    def emit(node: Session, parent: Session | None, depth: int,
             agent_path: str | None, ordinal: int):
        sid = ids[id(node)]
        node_cwd = cwd or node.cwd or base_cwd
        child_links = {}
        for child in node.children:
            edge = _edge_for(node, child)
            key = edge.spawn_message_id if edge and edge.spawn_message_id else ""
            child_links.setdefault(str(key), []).append(
                lambda created_at, child=child: _child_link_records(
                    node, child, ids[id(child)], agent_paths[id(child)], created_at))
        records = _session_records(tpl, node, node_cwd, sid, root_id,
                                   ids[id(parent)] if parent else None,
                                   depth, agent_path, child_links, tool_decider)
        dest = _destination(output_root, sid, ordinal)
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_suffix(".tmp")
        pending_paths.add(tmp)
        tmp.write_text("\n".join(json.dumps(line, ensure_ascii=False)
                                  for line in records) + "\n")
        tmp.rename(dest)
        pending_paths.discard(tmp)
        pending_paths.add(dest)
        paths[id(node)] = dest
        parents[id(node)] = ids[id(parent)] if parent else None
        working_dirs[id(node)] = node_cwd

        next_ordinal = ordinal + 1
        for child_index, child in enumerate(node.children):
            child_path = agent_paths[id(child)]
            emit(child, node, depth + 1, child_path, next_ordinal)
            next_ordinal += sum(1 for _ in child.walk())

    try:
        emit(sess, None, 0, sess.agent_path or "/root", 0)
        registry = (Path(state_db).expanduser() if state_db else
                    output_root.parent / "state_5.sqlite")
        register_tree(registry, [
            (node, ids[id(node)], paths[id(node)], parents[id(node)],
             working_dirs[id(node)], agent_paths[id(node)], edge_statuses[id(node)])
            for node in nodes
        ], cli_version=str(tpl["session_meta"]["payload"].get("cli_version", "")))
    except Exception:
        for path in pending_paths:
            path.unlink(missing_ok=True)
        raise
    return root_id, paths[id(sess)]
