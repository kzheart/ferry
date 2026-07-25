import json
from pathlib import Path

import pytest

from engine.adapters.claude.native_schema import (
    extract_templates as extract_claude,
    templates as claude_templates,
)
from engine.adapters.codex.native_schema import (
    extract_templates as extract_codex,
    templates as codex_templates,
)
from engine.adapters.opencode.native_schema import (
    extract_templates as extract_opencode,
    templates as opencode_templates,
)
from engine.adapters.pi.native_schema import (
    extract_templates as extract_pi,
    templates as pi_templates,
)
from engine.adapters.grok.native_schema import (
    extract_templates as extract_grok,
    templates as grok_templates,
)
from engine.adapters.registry import create_registry


FIXTURES = Path(__file__).parent / "fixtures" / "agent_formats"


def _jsonl(path):
    return [
        json.loads(line)
        for line in path.read_text().splitlines()
        if line.strip()
    ]


@pytest.mark.parametrize(
    ("agent", "template_factory", "extractor", "filename"),
    [
        ("claude", claude_templates, extract_claude, "session.jsonl"),
        ("codex", codex_templates, extract_codex, "session.jsonl"),
        ("opencode", opencode_templates, extract_opencode, "session.json"),
        ("pi", pi_templates, extract_pi, "session.jsonl"),
        ("grok", grok_templates, extract_grok, "."),
    ],
)
def test_native_fixture_matches_current_structure(
    agent, template_factory, extractor, filename
):
    path = FIXTURES / agent / "case-02-tools" / filename
    if agent == "grok":
        capture = {
            "summary": json.loads((path / "summary.json").read_text()),
            "updates": _jsonl(path / "updates.jsonl"),
            "chat": _jsonl(path / "chat_history.jsonl"),
        }
    else:
        capture = json.loads(path.read_text()) if filename.endswith(".json") else _jsonl(path)
    assert extractor(capture) == template_factory()


@pytest.mark.parametrize(
    "template_factory",
    [
        claude_templates,
        codex_templates,
        opencode_templates,
        pi_templates,
        grok_templates,
    ],
)
def test_template_results_are_independent_copies(template_factory):
    first = template_factory()
    first.clear()
    assert template_factory()


@pytest.mark.parametrize(
    "agent_id", ["claude", "codex", "opencode", "pi", "grok"],
)
def test_adapter_does_not_expose_a_format_version_registry(agent_id):
    adapter = create_registry().get(agent_id)
    assert not hasattr(adapter, "formats")


def test_pi_current_structure_is_session_v3():
    current = pi_templates()
    assert current["session"]["version"] == 3
    assert {
        "message.user",
        "message.assistant",
        "message.toolResult",
        "message.bashExecution",
        "compaction",
    } <= current.keys()


def test_grok_current_structure_is_bundle_with_chat_v1():
    current = grok_templates()
    assert current["summary"]["chat_format_version"] == 1
    assert {
        "update.UserMessage",
        "update.AgentMessageChunk",
        "update.ToolCall",
        "update.ToolCallUpdate",
        "chat.user",
        "chat.assistant",
    } <= current.keys()


def test_fixtures_have_no_version_directory_layer():
    for agent_id in ("claude", "codex", "opencode", "pi", "grok"):
        expected = {"case-01-plain", "case-02-tools"}
        if agent_id == "pi":
            expected.add("case-03-branch-compaction")
        if agent_id == "grok":
            expected |= {"case-03-rewind", "case-04-chat-fallback"}
        assert {path.name for path in (FIXTURES / agent_id).iterdir()} == expected
