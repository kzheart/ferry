import json
from pathlib import Path

import pytest

from engine.adapters.claude.formats import (
    FORMATS as CLAUDE_FORMATS,
    extract_templates as extract_claude,
)
from engine.adapters.codex.formats import (
    FORMATS as CODEX_FORMATS,
    extract_templates as extract_codex,
)
from engine.adapters.opencode.formats import (
    FORMATS as OPENCODE_FORMATS,
    extract_templates as extract_opencode,
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
    ("agent", "version", "registry", "extractor", "filename"),
    [
        ("claude", "2.1.204", CLAUDE_FORMATS, extract_claude, "session.jsonl"),
        ("codex", "0.144.0", CODEX_FORMATS, extract_codex, "session.jsonl"),
        ("opencode", "1.18.3", OPENCODE_FORMATS, extract_opencode, "session.json"),
    ],
)
def test_native_fixture_extracts_current_profile(
    agent, version, registry, extractor, filename
):
    path = FIXTURES / agent / version / "case-02-tools" / filename
    capture = json.loads(path.read_text()) if filename.endswith(".json") else _jsonl(path)
    assert extractor(capture) == registry.templates(version)


@pytest.mark.parametrize(
    ("registry", "verified", "compatible", "unsupported"),
    [
        (CLAUDE_FORMATS, "2.1.204", "2.1.215", "2.2.0"),
        (CODEX_FORMATS, "0.144.0", "0.145.0", "0.146.0"),
        (OPENCODE_FORMATS, "1.18.3", "1.18.4", "1.19.0"),
    ],
)
def test_format_status_distinguishes_tested_compatible_and_unsupported(
    registry, verified, compatible, unsupported
):
    assert registry.inspect(verified)["status"] == "verified"
    assert registry.inspect(compatible)["status"] == "compatible"
    assert registry.inspect(unsupported)["status"] == "unsupported"


def test_template_results_are_independent_copies():
    first = CLAUDE_FORMATS.templates()
    first["user"]["message"]["content"] = "changed"
    assert CLAUDE_FORMATS.templates()["user"]["message"]["content"] != "changed"


@pytest.mark.parametrize("agent_id", ["claude", "codex", "opencode"])
def test_every_agent_registers_native_format_profiles(agent_id):
    plugin = adapter(agent_id)
    assert plugin.formats is not None
    assert plugin.formats.agent == agent_id


@pytest.mark.parametrize(
    ("agent_id", "registry"),
    [
        ("claude", CLAUDE_FORMATS),
        ("codex", CODEX_FORMATS),
        ("opencode", OPENCODE_FORMATS),
    ],
)
def test_tested_versions_and_fixture_directories_are_bidirectional(
    agent_id, registry
):
    fixture_versions = {
        path.name
        for path in (FIXTURES / agent_id).iterdir()
        if path.is_dir()
    }
    tested_versions = {
        version
        for profile in registry.profiles
        for version in profile.tested_versions
    }
    assert fixture_versions == tested_versions
