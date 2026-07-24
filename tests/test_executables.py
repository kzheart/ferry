"""executables 定位器：PATH 优先、常见目录兜底、未命中保留裸名。"""
import pytest

import engine.system.executables as executables


@pytest.fixture(autouse=True)
def _clean_cache():
    executables.resolve.cache_clear()
    yield
    executables.resolve.cache_clear()


def test_resolve_prefers_path_hit(monkeypatch):
    monkeypatch.setattr(
        executables.shutil, "which",
        lambda tool, path=None: "/usr/local/bin/x" if path is None else None)
    assert executables.resolve("x") == "/usr/local/bin/x"


def test_resolve_falls_back_to_known_dirs(monkeypatch, tmp_path):
    hit = str(tmp_path / "opencode")
    monkeypatch.setattr(executables, "_fallback_dirs", lambda: [tmp_path])
    monkeypatch.setattr(
        executables.shutil, "which",
        lambda tool, path=None: hit if path == str(tmp_path) else None)
    assert executables.resolve("opencode") == hit


def test_resolve_uses_adapter_declared_fallback_dirs(monkeypatch, tmp_path):
    hit = str(tmp_path / "agent")
    monkeypatch.setattr(executables, "_TOOL_FALLBACK_DIRS", {})
    monkeypatch.setattr(executables, "_fallback_dirs", lambda: [])
    monkeypatch.setattr(
        executables.shutil, "which",
        lambda tool, path=None: hit if path == str(tmp_path) else None)

    executables.register_fallback_dirs(("agent",), (str(tmp_path),))

    assert executables.resolve("agent") == hit


def test_argv_keeps_bare_name_when_missing(monkeypatch):
    monkeypatch.setattr(executables.shutil, "which",
                        lambda tool, path=None: None)
    monkeypatch.setattr(executables, "_fallback_dirs", lambda: [])
    assert executables.argv("claude", "--version") == ["claude", "--version"]
