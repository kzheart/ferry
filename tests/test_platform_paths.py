from pathlib import Path

from engine.infrastructure.platform_paths import opencode_database_path


def test_opencode_database_path_uses_xdg_data_home():
    assert opencode_database_path(
        platform="posix",
        environ={"XDG_DATA_HOME": "/fixture/data"},
        home=Path("/fixture/home"),
    ) == Path("/fixture/data/opencode/opencode.db")


def test_opencode_database_path_has_windows_local_app_data_boundary():
    assert opencode_database_path(
        platform="nt",
        environ={"LOCALAPPDATA": "C:/Users/ferry/AppData/Local"},
        home=Path("C:/Users/ferry"),
    ) == Path("C:/Users/ferry/AppData/Local/opencode/opencode.db")


def test_opencode_database_path_allows_test_and_packaging_override():
    assert opencode_database_path(
        platform="nt",
        environ={"FERRY_OPENCODE_DB": "/fixture/opencode.db"},
    ) == Path("/fixture/opencode.db")
