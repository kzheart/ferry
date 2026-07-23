import json

from engine.adapters.codex import reader as codex_reader
from engine.adapters.codex import writer as codex_writer
from engine.domain.model import (
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
    tool_result_text,
)
from engine.domain.tool_ops import CanonicalOp


def _read(tmp_path, records):
    path = tmp_path / "rollout.jsonl"
    records = [
        {
            "type": "session_meta",
            "payload": {
                "id": "current-rollout",
                "session_id": "current-rollout",
                "cwd": "/workspace",
                "source": "cli",
            },
        },
        *records,
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    return codex_reader._read_one(path)


def _tools(session):
    return [
        block.tool
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool"
    ]


def test_current_function_and_custom_calls_coexist_but_root_items_are_ignored(tmp_path):
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "function_call", "name": "exec_command",
            "arguments": '{"cmd":"pwd","workdir":"/workspace"}',
            "input": 'tools.exec_command({"cmd":"wrong-function-field"})',
            "call_id": "function-call",
        }},
        {"type": "response_item", "payload": {
            "type": "function_call_output", "call_id": "function-call",
            "output": "function output",
        }},
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": 'tools.exec_command({"cmd":"printf custom"})',
            "arguments": '{"cmd":"wrong-custom-field"}',
            "call_id": "custom-call",
        }},
        {"type": "response_item", "payload": {
            "type": "custom_tool_call_output", "call_id": "custom-call",
            "output": "custom output",
        }},
        {"type": "custom_tool_call", "name": "exec",
         "input": 'tools.exec_command({"cmd":"pwd"})',
         "call_id": "root-call"},
    ])

    function, custom = _tools(session)
    assert function.op == CanonicalOp.SHELL_EXEC
    assert function.input == {"command": "pwd", "workdir": "/workspace"}
    assert tool_result_text(function.result) == "function output"
    assert custom.op == CanonicalOp.SHELL_EXEC
    assert custom.input == {"command": "printf custom"}
    assert tool_result_text(custom.result) == "custom output"


def test_reader_keeps_native_records_out_of_the_canonical_session(tmp_path):
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "current input"}],
        }},
        {"type": "response_item", "payload": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "current output"}],
        }},
    ])

    assert session.raw_records == []
    assert [message.source_id for message in session.messages] == [
        "record:1",
        "record:2",
    ]
    assert all(message.raw == [] for message in session.messages)


def test_private_result_extensions_are_not_rehydrated(tmp_path):
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": 'tools.exec_command({"cmd":"printf current"})',
            "call_id": "custom-call",
        }},
        {"type": "response_item", "payload": {
            "type": "custom_tool_call_output", "call_id": "custom-call",
            "output": json.dumps([{
                "type": "input_text",
                "text": json.dumps({
                    "status": "success",
                    "output": "current output",
                    "canonical_blocks": [{
                        "kind": "json",
                        "data": {"private": True},
                    }],
                    "canonical_metadata": {"private": True},
                }),
            }]),
        }},
    ])

    tool, = _tools(session)
    assert tool_result_text(tool.result) == "current output"
    assert [block.kind for block in tool.result.blocks] == ["text"]
    assert tool.result.metadata == {}


def test_writer_emits_only_current_native_result_fields():
    result = ToolResult(
        status="error",
        blocks=[
            ToolResultBlock("text", text="visible"),
            ToolResultBlock("json", data={"not": "native"}),
        ],
        stdout="stdout",
        stderr="stderr",
        exit_code=7,
        truncated=True,
        metadata={"private": True},
    )
    tool = ToolCall("exec", CanonicalOp.SHELL_EXEC, {"command": "false"}, result)

    assert codex_writer._result_payload(tool, "unused") == {
        "status": "error",
        "output": 'visible\n{"not":"native"}',
        "stdout": "stdout",
        "stderr": "stderr",
        "exit_code": 7,
        "truncated": True,
    }


def test_result_statuses_are_mapped_at_the_codex_boundary():
    expected = {
        "success": "success",
        "completed": "success",
        "error": "error",
        "interrupted": "interrupted",
        "running": "running",
        "pending": "pending",
        "unknown": "unknown",
        "complete": "unknown",
        "failed": "unknown",
        "cancelled": "unknown",
        None: "unknown",
    }

    for native_status, canonical_status in expected.items():
        result = codex_reader._parse_result([{
            "type": "input_text",
            "text": json.dumps({
                "status": native_status,
                "output": "fixture",
            }),
        }])
        assert result.status == canonical_status


def test_current_remote_function_call_stays_an_opaque_tool(tmp_path):
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "function_call",
            "name": "mcp__ssh__exec",
            "arguments": json.dumps({
                "host": "example.invalid",
                "command": "uptime",
            }),
            "call_id": "remote-call",
        }},
    ])

    tool, = _tools(session)
    assert tool.op == getattr(CanonicalOp, "TOOL_INVOKE", "tool.invoke")
    assert tool.input["namespace"] == "codex"
    assert tool.input["name"] == "mcp__ssh__exec"
    assert tool.input["input"] == {
        "host": "example.invalid",
        "command": "uptime",
    }


def test_current_custom_apply_patch_preserves_patch_and_change_summary(tmp_path):
    patch = """*** Begin Patch
*** Add File: src/new.txt
+new
*** Update File: src/old.txt
*** Move to: src/moved.txt
@@
-old
+updated
*** Delete File: src/gone.txt
*** End Patch"""
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "apply_patch",
            "input": patch, "call_id": "call-patch",
        }},
    ])

    tool, = _tools(session)
    assert tool.op == getattr(CanonicalOp, "FS_PATCH", "fs.patch")
    assert tool.input["raw_patch"] == patch
    assert tool.input["operations"] == [
        {"operation": "add", "path": "src/new.txt", "hunk_count": 0},
        {"operation": "move", "path": "src/old.txt",
         "destination": "src/moved.txt", "hunk_count": 1},
        {"operation": "delete", "path": "src/gone.txt", "hunk_count": 0},
    ]


def test_custom_js_balanced_scanner_handles_braces_inside_command_string(tmp_path):
    source = (
        'const result = await tools.exec_command({'
        '"cmd":"node -e \\"console.log({x: 1})\\"",'
        '"workdir":"/workspace"}); text(result.output);'
    )
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": source, "call_id": "call-balanced",
        }},
    ])

    tool, = _tools(session)
    assert tool.op == CanonicalOp.SHELL_EXEC
    assert tool.input == {
        "command": 'node -e "console.log({x: 1})"',
        "workdir": "/workspace",
    }


def test_custom_js_apply_patch_resolves_string_variable_without_wrapper_text(tmp_path):
    patch = "*** Begin Patch\n*** Add File: src/a.txt\n+x\n*** End Patch"
    source = (
        f"const patch = {json.dumps(patch)};"
        "text(await tools.apply_patch(patch));"
    )
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": source, "call_id": "call-js-patch",
        }},
    ])

    tool, = _tools(session)
    assert tool.op == getattr(CanonicalOp, "FS_PATCH", "fs.patch")
    assert tool.input["raw_patch"] == patch


def test_composite_custom_exec_stays_one_structured_opaque_call(tmp_path):
    source = (
        'const first = await tools.exec_command({"cmd":"pwd"});'
        'const second = await tools.apply_patch('
        '"*** Begin Patch\\n*** Add File: src/a.txt\\n+x\\n*** End Patch");'
    )
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": source, "call_id": "call-composite",
        }},
    ])

    tool, = _tools(session)
    assert tool.name == "exec"
    assert tool.op == getattr(CanonicalOp, "TOOL_INVOKE", "tool.invoke")
    assert tool.input["name"] == "exec"
    assert tool.input["input"] == source
    assert tool.input["structure_summary"] == {
        "kind": "composite",
        "invocation_count": 2,
        "tool_names": ["exec_command", "apply_patch"],
    }
    assert tool.input["children"] == [
        {"native_name": "exec_command", "input_kind": "object",
         "input_fields": ["cmd"]},
        {"native_name": "apply_patch", "input_kind": "string",
         "input_fields": []},
    ]


def test_nested_custom_tools_are_detected_as_composite(tmp_path):
    source = 'tools.outer(tools.inner({"value":"quoted ) and }"}))'
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": source, "call_id": "call-nested",
        }},
    ])

    tool, = _tools(session)
    assert tool.op == getattr(CanonicalOp, "TOOL_INVOKE", "tool.invoke")
    assert tool.input["structure_summary"]["tool_names"] == ["outer", "inner"]


def test_composite_summary_does_not_copy_sensitive_argument_values(tmp_path):
    secret = "sk-live-super-secret"
    source = (
        'tools.remote({"api_key":"' + secret + '","query":"safe"});'
        'tools.other({"authorization":"Bearer ' + secret + '"})'
    )
    session = _read(tmp_path, [
        {"type": "response_item", "payload": {
            "type": "custom_tool_call", "name": "exec",
            "input": source, "call_id": "call-secret",
        }},
    ])

    tool, = _tools(session)
    summary = json.dumps({
        "structure_summary": tool.input["structure_summary"],
        "children": tool.input["children"],
    })
    assert secret not in summary
    assert tool.input["children"][0]["input_fields"] == ["api_key", "query"]
    assert tool.input["input"] == source


def test_malformed_codex_line_is_reported_without_losing_later_records(tmp_path):
    path = tmp_path / "rollout.jsonl"
    path.write_text(
        '{"type":"session_meta","payload":{"id":"current-rollout","session_id":"current-rollout","cwd":"/workspace","source":"cli"}}\n'
        '{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"before"}]}}\n'
        '{"type":"broken"\n'
        '{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"after"}]}}\n'
    )

    session = codex_reader._read_one(path)

    assert [block.text for message in session.messages
            for block in message.blocks if block.kind == "text"] == [
        "before", "after"]
    assert [loss["code"] for loss in session.loss] == [
        "session.malformed_record"]


def test_sqlite_parent_edge_attaches_child_when_rollout_metadata_lacks_parent(
        monkeypatch, tmp_path):
    root_path = tmp_path / "root.jsonl"
    child_path = tmp_path / "child.jsonl"
    root = Session("codex", "root", "/tmp")
    child = Session("codex", "child", "/tmp")
    index = {
        "root": (root_path, {"parent_id": None}),
        "child": (child_path, {"parent_id": None}),
    }
    monkeypatch.setattr(
        codex_reader, "_rollout_index",
        lambda _rollout, _sessions_dir: index)
    monkeypatch.setattr(
        codex_reader, "_read_one",
        lambda path: root if path == root_path else child)
    monkeypatch.setattr(
        codex_reader, "_registry_edges",
        lambda _root: {"child": ("root", "open")})

    restored = codex_reader.read(str(root_path), sessions_dir=tmp_path)

    assert [value.source_id for value in restored.children] == ["child"]
    edge, = restored.agent_edges
    assert edge.association == "sqlite-parent"
    assert edge.confidence == 0.95
    assert edge.status == "open"
