from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "engine"


def test_adapter_contract_has_no_plugin_layer():
    assert (ENGINE / "adapters/contracts.py").is_file()
    for agent in ("claude", "codex", "opencode"):
        assert (ENGINE / f"adapters/{agent}/adapter.py").is_file()
        assert not (ENGINE / f"adapters/{agent}/plugin.py").exists()
    assert not (ENGINE / "adapters/base/plugin.py").exists()


def test_business_capabilities_live_in_top_level_packages():
    organization = ENGINE / "organization"
    assert {
        path.name for path in organization.glob("*.py")
    } == {"__init__.py", "proposals.py", "summaries.py"}
    operations = ENGINE / "operations"
    assert {
        path.name for path in operations.glob("*.py")
    } == {
        "__init__.py",
        "delete.py",
        "edit.py",
        "metadata.py",
        "migrate.py",
        "service.py",
        "verification.py",
    }
    assert not (ENGINE / "application/organization").exists()
    assert not (ENGINE / "application/operations").exists()
    assert not (ENGINE / "application/organizing.py").exists()
    assert not (ENGINE / "application/summaries.py").exists()
