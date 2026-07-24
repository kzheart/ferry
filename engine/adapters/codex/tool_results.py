"""Codex 工具结果包络解析。"""

from __future__ import annotations

import json

from ...sessions.model import ToolResult, ToolResultBlock

_RESULT_STATUS = {
    "success": "success",
    "completed": "success",
    "error": "error",
    "interrupted": "interrupted",
    "running": "running",
    "pending": "pending",
    "unknown": "unknown",
}


def _result_status(value) -> str:
    if not isinstance(value, str):
        return "unknown"
    return _RESULT_STATUS.get(value, "unknown")


def parse_result(raw) -> ToolResult:
    """Decode Codex output envelopes without flattening status or rich blocks."""
    blocks = []
    stdout = stderr = None
    exit_code = None
    truncated = None
    attachments = []
    explicit_status = None
    structured_envelope = False
    wrapper_blocks = []
    try:
        native_blocks = raw if isinstance(raw, list) else json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        native_blocks = raw
    if isinstance(native_blocks, dict):
        native_blocks = [native_blocks]
    elif not isinstance(native_blocks, list):
        native_blocks = [
            {
                "type": "input_text",
                "text": (
                    native_blocks
                    if isinstance(native_blocks, str)
                    else str(native_blocks)
                ),
            }
        ]

    for native_block in native_blocks:
        if not isinstance(native_block, dict):
            blocks.append(ToolResultBlock("json", data=native_block))
            continue
        kind = native_block.get("type")
        if kind in {"input_text", "output_text", "text"}:
            text = native_block.get("text", "")
            try:
                inner = json.loads(text)
            except (json.JSONDecodeError, TypeError):
                inner = None
            if isinstance(inner, dict) and any(
                key in inner
                for key in (
                    "output",
                    "stdout",
                    "stderr",
                    "exit_code",
                    "status",
                    "truncated",
                    "attachments",
                )
            ):
                structured_envelope = True
                output = inner.get("output")
                stdout_value = inner.get("stdout", output)
                if isinstance(stdout_value, str):
                    stdout = stdout_value
                if isinstance(output, str) and output:
                    blocks.append(ToolResultBlock("text", text=output))
                elif output is not None:
                    blocks.append(ToolResultBlock("json", data=output))
                if isinstance(inner.get("stderr"), str):
                    stderr = inner["stderr"]
                code = inner.get("exit_code")
                if isinstance(code, int) and not isinstance(code, bool):
                    exit_code = code
                if isinstance(inner.get("truncated"), bool):
                    truncated = inner["truncated"]
                if isinstance(inner.get("attachments"), list):
                    attachments = inner["attachments"]
                explicit_status = inner.get("status")
            elif text:
                block = ToolResultBlock("text", text=text)
                blocks.append(block)
                if text.startswith("Script completed\nWall time "):
                    wrapper_blocks.append(block)
        elif kind in {"input_image", "output_image", "image"}:
            blocks.append(
                ToolResultBlock(
                    "image",
                    uri=native_block.get("image_url") or native_block.get("url"),
                    data=native_block.get("data"),
                    mime_type=native_block.get("mime_type"),
                )
            )
        elif kind == "file":
            blocks.append(
                ToolResultBlock(
                    "file",
                    uri=native_block.get("url"),
                    filename=native_block.get("filename"),
                    mime_type=native_block.get("mime_type"),
                )
            )
        else:
            blocks.append(ToolResultBlock("json", data=native_block))

    status = _result_status(explicit_status)
    if structured_envelope:
        wrapper_ids = {id(block) for block in wrapper_blocks}
        blocks = [block for block in blocks if id(block) not in wrapper_ids]
    if status == "unknown" and exit_code is not None:
        status = "success" if exit_code == 0 else "error"
    if stderr and status == "unknown":
        status = "error"
    return ToolResult(
        status=status,
        blocks=blocks,
        stdout=stdout,
        stderr=stderr,
        exit_code=exit_code,
        truncated=truncated,
        attachments=attachments,
    )
