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
from engine.adapters.registry import adapter


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
    ],
)
def test_native_fixture_matches_current_structure(
    agent, template_factory, extractor, filename
):
    path = FIXTURES / agent / "case-02-tools" / filename
    capture = json.loads(path.read_text()) if filename.endswith(".json") else _jsonl(path)
    assert extractor(capture) == template_factory()


@pytest.mark.parametrize(
    "template_factory",
    [
        claude_templates,
        codex_templates,
        opencode_templates,
    ],
)
def test_template_results_are_independent_copies(template_factory):
    first = template_factory()
    first.clear()
    assert template_factory()


@pytest.mark.parametrize("agent_id", ["claude", "codex", "opencode"])
def test_adapter_does_not_expose_a_format_version_registry(agent_id):
    plugin = adapter(agent_id)
    assert not hasattr(plugin, "formats")


def test_fixtures_have_no_version_directory_layer():
    for agent_id in ("claude", "codex", "opencode"):
        assert {path.name for path in (FIXTURES / agent_id).iterdir()} == {
            "case-01-plain",
            "case-02-tools",
        }
