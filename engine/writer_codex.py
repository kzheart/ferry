"""Codex writer:规范化中间格式 → rollout JSONL(可被 codex exec resume 加载)。

格式规格见 spec/formats/codex.md。核心策略:
- 结构模板取自黄金样本原文(session_meta / turn_context / 各类 response_item),
  只替换内容字段,不手写结构 —— 版本漂移时重新生成黄金样本即可跟上。
- shell.exec 原生映射为 exec_command;fs.write 映射为 apply_patch(Add File);
  其余工具降级为叙述文本(narration)。
"""
import json
import secrets
import time
import uuid
from pathlib import Path

from .model import Session

GOLDEN = Path(__file__).resolve().parent.parent / "golden" / "codex"


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
        raise RuntimeError("缺少 golden/codex 样本,先运行 harness/gen_golden.py")
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


def _narration(tool) -> str:
    inp = json.dumps(tool.input, ensure_ascii=False)[:500] \
        if isinstance(tool.input, dict) else str(tool.input)[:500]
    out = (tool.output or "(无输出)")[:2000]
    return (f"[历史记录:此前通过工具 {tool.name} 执行了操作]\n"
            f"参数: {inp}\n结果:\n{out}")


def write(sess: Session, cwd: str | None = None) -> tuple[str, Path]:
    """写出 rollout 文件,返回 (新 session_id, 文件路径)。"""
    tpl = _load_templates()
    sid = _uuid7()
    cwd = cwd or sess.cwd
    now = _now_iso()

    meta = _clone(tpl["session_meta"])
    meta["timestamp"] = now
    mp = meta["payload"]
    mp["id"] = mp["session_id"] = sid
    mp["timestamp"] = now
    mp["cwd"] = cwd

    tc = _clone(tpl["turn_context"])
    tc["timestamp"] = now
    tc["payload"]["cwd"] = cwd
    tc["payload"]["turn_id"] = _uuid7()

    out_lines = [meta, tc]
    for m in sess.messages:
        texts = []
        for b in m.blocks:
            if b.kind == "text":
                texts.append(b.text)
            elif b.kind == "tool":
                t = b.tool
                if t.op == "shell.exec" and isinstance(t.input, dict) \
                        and t.input.get("command"):
                    if texts:
                        out_lines.append(_msg(tpl, m.role, "\n\n".join(texts)))
                        texts = []
                    out_lines += _exec_pair(
                        tpl, t.input["command"], cwd,
                        t.meta.get("stdout", t.output), None)
                elif t.op == "fs.write" and isinstance(t.input, dict) \
                        and t.input.get("file_path"):
                    if texts:
                        out_lines.append(_msg(tpl, m.role, "\n\n".join(texts)))
                        texts = []
                    body = str(t.input.get("content", ""))
                    patch = "*** Begin Patch\n*** Add File: {}\n{}\n*** End Patch".format(
                        t.input["file_path"],
                        "\n".join("+" + l for l in body.splitlines()))
                    call, outrec = _exec_pair(tpl, "", cwd, "{}", 0)
                    call["payload"]["input"] = (
                        f"const patch = {json.dumps(patch)};\n"
                        "text(await tools.apply_patch(patch));\n")
                    outrec["payload"]["output"] = json.dumps([
                        {"type": "input_text",
                         "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
                        {"type": "input_text", "text": "{}"}])
                    out_lines += [call, outrec]
                else:
                    sess.lose(f"工具 {t.name} 降级为叙述文本")
                    texts.append(_narration(t))
        if texts:
            out_lines.append(_msg(tpl, m.role, "\n\n".join(texts)))

    day = time.strftime("%Y/%m/%d")
    stamp = time.strftime("%Y-%m-%dT%H-%M-%S")
    dest = Path.home() / ".codex" / "sessions" / day / \
        f"rollout-{stamp}-{sid}.jsonl"
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(".tmp")
    tmp.write_text("\n".join(json.dumps(l, ensure_ascii=False)
                             for l in out_lines) + "\n")
    tmp.rename(dest)
    return sid, dest
