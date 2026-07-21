"""Codex writer:规范化中间格式 → rollout JSONL(可被 codex exec resume 加载)。

写入 Codex 原生 JSONL 会话记录。核心策略:
- 结构模板取自黄金样本原文(session_meta / 各类 response_item),
  只替换内容字段,不手写结构 —— 版本漂移时重新生成黄金样本即可跟上。
- shell.exec 原生映射为 exec_command;fs.write 映射为 apply_patch(Add File);
  其余工具降级为叙述文本(narration)。
"""
import json
import secrets
import time
import uuid
from pathlib import Path

from ...domain.model import Session
from ...infrastructure.resources import resource_path
from ..base.narration import narrate
from .registry import register_tree

GOLDEN = resource_path("golden", "codex")


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


def _load_templates():
    """从最新版本的黄金样本中取各类记录的原文模板。"""
    versions = sorted(GOLDEN.iterdir()) if GOLDEN.exists() else []
    if not versions:
        raise RuntimeError("缺少生产依赖 golden/codex 样本")
    sample = versions[-1] / "case-02-tools" / "session.jsonl"
    tpl = {}
    for line in sample.read_text().splitlines():
        rec = json.loads(line)
        t = rec["type"]
        pt = (rec.get("payload") or {}).get("type")
        key = f"{t}.{pt}" if pt else t
        if key not in tpl:
            tpl[key] = rec
        if key == "response_item.message":
            tpl.setdefault(f"message.{rec['payload']['role']}", rec)
    return tpl


def _clone(tpl: dict) -> dict:
    return json.loads(json.dumps(tpl))


def _msg(tpl, role: str, text: str) -> dict:
    rec = _clone(tpl[f"message.{role}"])
    rec["timestamp"] = _now_iso()
    p = rec["payload"]
    p["content"] = [{"type": "input_text" if role == "user" else "output_text",
                     "text": text}]
    if "id" in p:
        p["id"] = None
    return rec


def _exec_pair(tpl, cmd: str, workdir: str, stdout: str, exit_code) -> list:
    call = _clone(tpl["response_item.custom_tool_call"])
    out = _clone(tpl["response_item.custom_tool_call_output"])
    call_id = "call_" + secrets.token_urlsafe(18)[:24]
    call["timestamp"] = out["timestamp"] = _now_iso()
    cp, op = call["payload"], out["payload"]
    cp["id"] = "ctc_" + secrets.token_hex(25)
    cp["call_id"] = op["call_id"] = call_id
    cp["name"] = "exec"
    args = json.dumps({"cmd": cmd, "workdir": workdir,
                       "yield_time_ms": 10000, "max_output_tokens": 1000})
    cp["input"] = (f"const r = await tools.exec_command({args});\n"
                   "text(JSON.stringify(r));\n")
    inner = json.dumps({"chunk_id": secrets.token_hex(3),
                        "wall_time_seconds": 0.01,
                        "exit_code": exit_code if exit_code is not None else 0,
                        "original_token_count": max(1, len(stdout) // 4),
                        "output": stdout})
    op["id"] = "fco_" + _uuid7()
    op["output"] = json.dumps([
        {"type": "input_text",
         "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
        {"type": "input_text", "text": inner}])
    op.pop("internal_chat_message_metadata_passthrough", None)
    return [call, out]


def _apply_patch_pair(tpl, patch: str, output: str = "{}") -> list:
    call, outrec = _exec_pair(tpl, "", "", "{}", 0)
    call["payload"]["input"] = (f"const patch = {json.dumps(patch)};\n"
                                "text(await tools.apply_patch(patch));\n")
    outrec["payload"]["output"] = json.dumps([
        {"type": "input_text",
         "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
        {"type": "input_text", "text": output}])
    return [call, outrec]


def _native_records(tpl, t, cwd) -> list | None:
    """规范操作 → Codex 原生记录;无法映射返回 None(由调用方降级)。

    常用工具必须原生映射:目标缺同类工具时用等价 shell 形式(如 fs.read → cat)。
    """
    i = t.input if isinstance(t.input, dict) else {}
    if t.op == "shell.exec" and i.get("command"):
        return _exec_pair(tpl, i["command"], cwd,
                          t.meta.get("stdout", t.output), None)
    if t.op == "fs.read" and i.get("file_path"):
        return _exec_pair(tpl, f"cat {i['file_path']}", cwd,
                          t.output or "", 0)
    if t.op == "fs.write" and i.get("file_path"):
        body = str(i.get("content", ""))
        patch = "*** Begin Patch\n*** Add File: {}\n{}\n*** End Patch".format(
            i["file_path"], "\n".join("+" + l for l in body.splitlines()))
        return _apply_patch_pair(tpl, patch)
    if t.op == "fs.edit" and i.get("file_path"):
        hunk = "\n".join(["@@"]
                         + ["-" + l for l in str(i.get("old", "")).splitlines()]
                         + ["+" + l for l in str(i.get("new", "")).splitlines()])
        patch = "*** Begin Patch\n*** Update File: {}\n{}\n*** End Patch".format(
            i["file_path"], hunk)
        return _apply_patch_pair(tpl, patch)
    return None


def _session_records(tpl, sess: Session, cwd: str, sid: str, root_id: str,
                     parent_id: str | None, depth: int,
                     agent_path: str | None) -> list[dict]:
    now = _now_iso()
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
        mp["agent_path"] = agent_path

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
                if t.op == "agent.spawn":
                    continue
                native = _native_records(tpl, t, cwd)
                if native:
                    if texts:
                        out_lines.append(_msg(tpl, role, "\n\n".join(texts)))
                        texts = []
                    out_lines += native
                else:
                    sess.lose("migration.tool_degraded", tool_name=t.name)
                    texts.append(narrate(t))
        if texts:
            out_lines.append(_msg(tpl, role, "\n\n".join(texts)))
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


def _child_link_records(parent: Session, child: Session, child_id: str,
                        agent_path: str) -> list[dict]:
    edge = _edge_for(parent, child)
    call_id = (edge.source_call_id if edge and edge.source_call_id else
               "call_" + secrets.token_urlsafe(18)[:24])
    prompt = edge.prompt if edge else ""
    agent_type = (edge.agent_type if edge and edge.agent_type else
                  child.agent_type)
    arguments = {"message": prompt}
    if agent_type:
        arguments["agent_type"] = agent_type
    now = _now_iso()
    spawn = {
        "timestamp": now,
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "spawn_agent",
            "arguments": json.dumps(arguments, ensure_ascii=False),
            "call_id": call_id,
        },
    }
    activity = {
        "timestamp": now,
        "type": "event_msg",
        "payload": {
            "type": "sub_agent_activity",
            "agent_thread_id": child_id,
            "agent_path": agent_path,
            "kind": "completed",
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
          state_db: str | Path | None = None) -> tuple[str, Path]:
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

    def emit(node: Session, parent: Session | None, depth: int,
             agent_path: str | None, ordinal: int):
        sid = ids[id(node)]
        node_cwd = cwd or node.cwd or base_cwd
        records = _session_records(tpl, node, node_cwd, sid, root_id,
                                   ids[id(parent)] if parent else None,
                                   depth, agent_path)
        for child_index, child in enumerate(node.children):
            child_path = (child.agent_path or
                          f"{agent_path or '/root'}/{child.agent_id or child_index + 1}")
            records.extend(_child_link_records(node, child, ids[id(child)],
                                               child_path))
        dest = _destination(output_root, sid, ordinal)
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_suffix(".tmp")
        tmp.write_text("\n".join(json.dumps(line, ensure_ascii=False)
                                  for line in records) + "\n")
        tmp.rename(dest)
        paths[id(node)] = dest
        parents[id(node)] = ids[id(parent)] if parent else None
        working_dirs[id(node)] = node_cwd

        next_ordinal = ordinal + 1
        for child_index, child in enumerate(node.children):
            child_path = (child.agent_path or
                          f"{agent_path or '/root'}/{child.agent_id or child_index + 1}")
            emit(child, node, depth + 1, child_path, next_ordinal)
            next_ordinal += sum(1 for _ in child.walk())

    try:
        emit(sess, None, 0, sess.agent_path or "/root", 0)
        registry = (Path(state_db).expanduser() if state_db else
                    output_root.parent / "state_5.sqlite")
        register_tree(registry, [
            (node, ids[id(node)], paths[id(node)], parents[id(node)],
             working_dirs[id(node)])
            for node in nodes
        ], cli_version=str(tpl["session_meta"]["payload"].get("cli_version", "")))
    except Exception:
        for path in paths.values():
            path.unlink(missing_ok=True)
        raise
    return root_id, paths[id(sess)]
