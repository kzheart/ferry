"""OpenCode reader/writer:走官方 `opencode export` / `opencode import`,不直接碰 SQLite。

格式规格见 spec/formats/opencode.md。export 形状:
    {"info": <session 行>, "messages": [{"info": <message.data>, "parts": [<part.data>...]}]}
"""
import json
import secrets
import subprocess
import time
from pathlib import Path

from .model import Block, Message, Session, ToolCall

TOOL_OPS = {"bash": "shell.exec", "read": "fs.read",
            "write": "fs.write", "edit": "fs.edit"}
GOLDEN = Path(__file__).resolve().parent.parent / "golden" / "opencode"


def _oc(args, **kw):
    r = subprocess.run(["opencode", *args], capture_output=True, text=True,
                       timeout=120, **kw)
    if r.returncode != 0:
        raise RuntimeError(f"opencode {' '.join(args)} 失败: {r.stderr[-400:]}")
    return r.stdout


def _new_id(prefix: str) -> str:
    return f"{prefix}_{secrets.token_hex(6)}{secrets.token_urlsafe(12)[:14]}"


# ---------- reader ----------

def read(session_id: str) -> Session:
    data = json.loads(_oc(["export", session_id]))
    info = data["info"]
    sess = Session(source_tool="opencode", source_id=info["id"],
                   cwd=info.get("directory", ""), title=info.get("title", ""))
    for m in data["messages"]:
        role = m["info"].get("role", "user")
        blocks = []
        for p in m["parts"]:
            pt = p.get("type")
            if pt == "text":
                blocks.append(Block("text", p.get("text", "")))
            elif pt == "reasoning":
                sess.lose("reasoning part 丢弃")
            elif pt == "tool":
                st = p.get("state", {})
                inp = dict(st.get("input") or {})
                if "filePath" in inp:          # 归一化为规范参数名
                    inp["file_path"] = inp.pop("filePath")
                if "oldString" in inp:
                    inp["old"] = inp.pop("oldString")
                if "newString" in inp:
                    inp["new"] = inp.pop("newString")
                blocks.append(Block("tool", tool=ToolCall(
                    name=p.get("tool", "?"),
                    op=TOOL_OPS.get(p.get("tool")),
                    input=inp,
                    output=st.get("output", ""),
                    meta={"exit": (st.get("metadata") or {}).get("exit")})))
            elif pt in ("step-start", "step-finish"):
                continue
            else:
                sess.lose(f"未知 part 类型丢弃: {pt}")
        if blocks:
            sess.messages.append(Message(role=role, blocks=blocks, raw=[m]))
    return sess


# ---------- writer ----------

def _template():
    """用黄金样本会话的官方 export 作为结构模板。"""
    versions = sorted(GOLDEN.iterdir()) if GOLDEN.exists() else []
    if not versions:
        raise RuntimeError("缺少 golden/opencode 样本")
    manifest = json.loads(
        (versions[-1] / "case-02-tools" / "manifest.json").read_text())
    data = json.loads(_oc(["export", manifest["session_id"]]))
    tpl = {"info": data["info"]}
    for m in data["messages"]:
        role = m["info"].get("role")
        tpl.setdefault(f"msg.{role}", m["info"])
        for p in m["parts"]:
            tpl.setdefault(f"part.{p.get('type')}", p)
    return tpl


def _clone(o):
    return json.loads(json.dumps(o))


def write(sess: Session, cwd: str | None = None) -> tuple[str, Path]:
    tpl = _template()
    sid = _new_id("ses")
    cwd = str(Path(cwd or sess.cwd).resolve())
    now = int(time.time() * 1000)

    info = _clone(tpl["info"])
    info.update({"id": sid, "directory": cwd,
                 "title": sess.title or f"migrated from {sess.source_tool}",
                 "time": {"created": now, "updated": now}})
    for k in ("share",):
        info.pop(k, None)

    messages = []
    for m in sess.messages:
        mid = _new_id("msg")
        minfo = _clone(tpl.get(f"msg.{m.role}", tpl["msg.user"]))
        minfo.update({"id": mid, "sessionID": sid})
        if m.role == "assistant":
            # completed + finish 缺失会让 runtime 认为该轮未结束而死循环
            minfo["time"] = {"created": now, "completed": now}
            minfo["finish"] = ("tool-calls" if any(
                b.kind == "tool" for b in m.blocks) else "stop")
        else:
            minfo["time"] = {"created": now}
        parts = []

        def add_part(ptype, fill):
            key = f"part.{ptype}"
            if key not in tpl:
                return False
            p = _clone(tpl[key])
            p.update({"id": _new_id("prt"), "messageID": mid,
                      "sessionID": sid})
            p.update(fill)
            parts.append(p)
            return True

        def add_tool_part(tool, native_input, output, title, metadata):
            st = _clone(tpl["part.tool"]["state"])
            st.update({"status": "completed", "input": native_input,
                       "output": output, "title": title[:80],
                       "metadata": metadata})
            return add_part("tool", {"tool": tool,
                                     "callID": "call-" + secrets.token_hex(8),
                                     "state": st})

        for b in m.blocks:
            if b.kind == "text":
                add_part("text", {"text": b.text})
            elif b.kind == "tool":
                t = b.tool
                i = t.input if isinstance(t.input, dict) else {}
                # 常用工具原生映射(spec/mapping/tools.yaml)
                if t.op == "shell.exec" and i.get("command"):
                    add_tool_part("bash", {"command": i["command"]},
                                  t.output, i["command"],
                                  {"output": t.output,
                                   "exit": t.meta.get("exit", 0) or 0,
                                   "truncated": False})
                elif t.op == "fs.read" and i.get("file_path"):
                    add_tool_part("read", {"filePath": i["file_path"]},
                                  t.output, i["file_path"],
                                  {"truncated": False})
                elif t.op == "fs.write" and i.get("file_path"):
                    add_tool_part("write", {"filePath": i["file_path"],
                                            "content": i.get("content", "")},
                                  t.output or "Wrote file successfully.",
                                  i["file_path"],
                                  {"filepath": i["file_path"],
                                   "exists": False, "truncated": False,
                                   "diagnostics": {}})
                elif t.op == "fs.edit" and i.get("file_path"):
                    add_tool_part("edit", {"filePath": i["file_path"],
                                           "oldString": i.get("old", ""),
                                           "newString": i.get("new", "")},
                                  t.output or "Edited file.",
                                  i["file_path"], {"truncated": False})
                else:
                    sess.lose(f"工具 {t.name} 降级为叙述文本")
                    inp = json.dumps(t.input, ensure_ascii=False)[:500] \
                        if isinstance(t.input, dict) else str(t.input)[:500]
                    add_part("text", {"text":
                             f"[历史记录:此前通过工具 {t.name} 执行了操作]\n"
                             f"参数: {inp}\n结果:\n{(t.output or '(无输出)')[:2000]}"})
        if parts:
            messages.append({"info": minfo, "parts": parts})

    payload = {"info": info, "messages": messages}
    tmp = Path(f"/tmp/rh-import-{sid}.json")
    tmp.write_text(json.dumps(payload, ensure_ascii=False))
    # import 会把会话挂到进程 cwd 对应的项目,JSON 里的 directory 不生效
    out = _oc(["import", str(tmp)], cwd=cwd)
    if sid not in out:
        raise RuntimeError(f"import 结果异常: {out[-300:]}")
    return sid, tmp
