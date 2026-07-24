from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "engine"


def test_adapter_contract_has_no_plugin_layer():
    assert (ENGINE / "adapters/contracts.py").is_file()
    for agent in ("claude", "codex", "opencode"):
        assert (ENGINE / f"adapters/{agent}/adapter.py").is_file()
        assert not (ENGINE / f"adapters/{agent}/plugin.py").exists()
    assert not (ENGINE / "adapters/base/plugin.py").exists()


def test_organization_use_cases_have_a_dedicated_package():
    package = ENGINE / "application/organization"
    assert {
        path.name for path in package.glob("*.py")
    } == {"__init__.py", "proposals.py", "summaries.py"}
    assert not (ENGINE / "application/organizing.py").exists()
    assert not (ENGINE / "application/summaries.py").exists()
