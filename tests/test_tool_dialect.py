"""工具方言的契约测试。

守两类事实:
1. 声明本身的完整性——每个方言只声明规范操作、转换器名字合法、
   读名不冲突。
2. 基于本机真实会话调研(2026-07)锁定的映射面——Grok 两代工具、
   codex shell_command、description 不再被静默吞掉。
"""
import pytest

from engine.adapters.shared.dialect import CONVERTERS, get_dialect
from engine.sessions.tool_ops import (
    CANONICAL_OPS, TOOL_OP_SPECS, CanonicalOp, has_valid_tool_input,
)

ADAPTERS = ("claude", "opencode", "pi", "grok", "codex")


@pytest.mark.parametrize("adapter", ADAPTERS)
def test_dialect_declares_only_canonical_ops_and_known_converters(adapter):
    dialect = get_dialect(adapter)
    assert dialect is not None
    for binding in dialect.bindings:
        assert binding.op in CANONICAL_OPS
        spec = TOOL_OP_SPECS[binding.op]
        known = set(spec.required_inputs) | set(spec.optional_inputs)
        for field in binding.fields:
            assert field.canonical in known, (
                f"{adapter}.{binding.name} 映射了规范外字段 {field.canonical}")
            for converter in (field.decode, field.encode):
                assert converter is None or converter in CONVERTERS


@pytest.mark.parametrize("adapter", ADAPTERS)
def test_dialect_read_names_do_not_collide(adapter):
    dialect = get_dialect(adapter)
    seen = {}
    for binding in dialect.bindings:
        for name in binding.all_read_names:
            assert seen.setdefault(name, binding.op) == binding.op, (
                f"{adapter} 读名 {name} 指向了两个不同操作")


@pytest.mark.parametrize("adapter,name,raw,op,expected", [
    # claude:description 不再被静默吞掉,sandbox 旗标双向可逆
    ("claude", "Bash",
     {"command": "ls", "description": "list", "timeout": 500},
     CanonicalOp.SHELL_EXEC,
     {"command": "ls", "description": "list", "timeout_ms": 500}),
    # opencode:bash 的 description 同样保留
    ("opencode", "bash", {"command": "ls", "description": "list"},
     CanonicalOp.SHELL_EXEC, {"command": "ls", "description": "list"}),
    # codex:shell_command 此前未归一,本机会话里有 1.6 万次调用
    ("codex", "shell_command", {"command": "pwd", "workdir": "/w"},
     CanonicalOp.SHELL_EXEC, {"command": "pwd", "workdir": "/w"}),
    ("codex", "read_file", {"path": "/a.md", "start_line": 3, "limit": 40},
     CanonicalOp.FS_READ, {"file_path": "/a.md", "offset": 3, "limit": 40}),
    # grok 当前代:字符串数值纠偏成 int
    ("grok", "read_file", {"target_file": "/a.md", "limit": "150"},
     CanonicalOp.FS_READ, {"file_path": "/a.md", "limit": 150}),
    ("grok", "run_terminal_command",
     {"command": "ls", "description": "list", "timeout": "60000"},
     CanonicalOp.SHELL_EXEC,
     {"command": "ls", "description": "list", "timeout_ms": 60000}),
    ("grok", "search_replace",
     {"file_path": "/f", "old_string": "a", "new_string": "b"},
     CanonicalOp.FS_EDIT, {"file_path": "/f", "old": "a", "new": "b"}),
    # grok 旧代(PascalCase)同样收入
    ("grok", "StrReplace", {"path": "/f", "old_string": "a",
                            "new_string": "b"},
     CanonicalOp.FS_EDIT, {"file_path": "/f", "old": "a", "new": "b"}),
    ("grok", "Shell",
     {"command": "ls", "description": "x", "block_until_ms": "180000.0"},
     CanonicalOp.SHELL_EXEC,
     {"command": "ls", "description": "x", "timeout_ms": 180000}),
    ("grok", "Write", {"path": "/f", "contents": "body"},
     CanonicalOp.FS_WRITE, {"file_path": "/f", "content": "body"}),
    ("grok", "Glob", {"glob_pattern": "*.py", "target_directory": "/w"},
     CanonicalOp.FS_GLOB, {"pattern": "*.py", "path": "/w"}),
])
def test_survey_backed_read_mappings(adapter, name, raw, op, expected):
    parsed = get_dialect(adapter).parse(name, raw)
    assert parsed == (op, expected)
    assert has_valid_tool_input(*parsed)


def test_grok_unknown_flags_fall_back_to_private_call():
    # grep 的 -A/-i 这类旗标没有规范对应,整体保底而不是有损猜测
    assert get_dialect("grok").parse("grep", {"pattern": "x", "-A": "3"}) is None


def test_grok_updates_stream_transport_noise_is_stripped():
    """updates 流的 rawInput 带 variant 判别符和成套 null 键,不该触发保底。"""
    parsed = get_dialect("grok").parse("run_terminal_command", {
        "variant": "Bash", "command": "ls", "description": "list",
        "is_background": False, "timeout": None,
    })
    assert parsed == (CanonicalOp.SHELL_EXEC, {
        "command": "ls", "description": "list", "background": False,
    })


def test_grok_chat_row_stringified_booleans_are_coerced():
    parsed = get_dialect("grok").parse("search_replace", {
        "file_path": "/f", "old_string": "a", "new_string": "b",
        "replace_all": "False",
    })
    assert parsed == (CanonicalOp.FS_EDIT, {
        "file_path": "/f", "old": "a", "new": "b", "replace_all": False,
    })


def test_pi_bash_seconds_round_trip():
    dialect = get_dialect("pi")
    op, canonical = dialect.parse("bash", {"command": "ls", "timeout": 60})
    assert canonical == {"command": "ls", "timeout_ms": 60000}
    assert dialect.render(op, canonical) == (
        "bash", {"command": "ls", "timeout": 60.0})


@pytest.mark.parametrize("adapter", ("claude", "opencode", "pi", "grok"))
def test_write_bindings_round_trip_their_own_render(adapter):
    """写端渲染出的原生调用,必须能被同一方言的读端重新归一回等价输入。"""
    dialect = get_dialect(adapter)
    samples = {
        CanonicalOp.SHELL_EXEC: {"command": "pwd", "description": "d"},
        CanonicalOp.FS_READ: {"file_path": "/w/f", "limit": 10},
        CanonicalOp.FS_WRITE: {"file_path": "/w/f", "content": "x"},
        CanonicalOp.FS_EDIT: {"file_path": "/w/f", "old": "a", "new": "b"},
        CanonicalOp.FS_SEARCH: {"query": "needle", "path": "/w"},
        CanonicalOp.FS_GLOB: {"pattern": "*.py"},
        CanonicalOp.WEB_FETCH: {"url": "https://example.com"},
        CanonicalOp.WEB_SEARCH: {"query": "example"},
    }
    for op in dialect.write_ops():
        canonical = samples.get(op)
        if canonical is None:
            continue
        binding = dialect.binding_for(op)
        supported = binding.supported_fields()
        canonical = {key: value for key, value in canonical.items()
                     if key in supported}
        if not has_valid_tool_input(op, canonical):
            continue
        name, native = dialect.render(op, canonical)
        parsed = dialect.parse(name, native)
        assert parsed is not None, f"{adapter}.{name} 渲染结果读不回来"
        parsed_op, round_tripped = parsed
        assert parsed_op == op
        for key, value in canonical.items():
            assert round_tripped.get(key) == value, (
                f"{adapter}.{name} 字段 {key} round-trip 失真")


@pytest.mark.parametrize("adapter,command_key", [
    ("claude", "command"), ("pi", "command"), ("grok", "command"),
])
def test_shell_without_workdir_param_inlines_cd_prefix(adapter, command_key):
    """claude/pi/grok 的 shell 没有工作目录参数:workdir 前缀成 cd,不丢信息。"""
    dialect = get_dialect(adapter)
    name, native = dialect.render(CanonicalOp.SHELL_EXEC,
                                  {"command": "ls", "workdir": "/work dir"})
    assert native[command_key].startswith("cd '/work dir' && ls")
    # workdir 计入 supported,迁移预览不再把它标成丢失参数
    assert "workdir" in dialect.supported_fields(CanonicalOp.SHELL_EXEC)


def test_grok_result_envelopes_unwrap_to_semantic_text():
    """rawOutput 类型信封拆出语义文本与退出码,迁移结果不再是 json 投影。"""
    from engine.adapters.grok.reader import _result

    read = _result({"type": "ReadFile",
                    "FileContent": {"content": "line one"}}, "completed")
    assert read.status == "success"
    assert [b.kind for b in read.blocks] == ["text"]
    assert read.blocks[0].text == "line one"

    bash = _result({"type": "Bash", "output": [104, 105, 10],
                    "exit_code": 1, "truncated": False,
                    "output_for_prompt": "exit: 1\nhi\n"}, "completed")
    assert bash.status == "error"
    assert bash.exit_code == 1
    assert bash.blocks[0].text == "hi\n"

    unknown = _result({"type": "Mystery", "payload": 1}, "completed")
    assert [b.kind for b in unknown.blocks] == ["json"]
