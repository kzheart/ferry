import pytest

from scripts import build


def test_native_build_targets_keep_macos_and_windows():
    assert build.native_target(
        "Darwin", "arm64",
    ) == "aarch64-apple-darwin"
    assert build.native_target(
        "Windows", "AMD64",
    ) == "x86_64-pc-windows-msvc"
    with pytest.raises(ValueError, match="不支持的构建主机"):
        build.native_target("Linux", "x86_64")


def test_node_version_requirement_is_compared_numerically():
    assert build.parse_node_version("v22.19.0") == (22, 19, 0)
    assert build.parse_node_version("24.15.0") == (24, 15, 0)
    with pytest.raises(ValueError, match="无法识别 Node 版本"):
        build.parse_node_version("current")


def test_root_build_runs_both_sidecars_before_tauri(monkeypatch):
    calls = []
    monkeypatch.setattr(build, "verify_toolchain", lambda target: None)
    monkeypatch.setattr(
        build,
        "run",
        lambda command, cwd=build.ROOT: calls.append((command, cwd)),
    )

    build.build("aarch64-apple-darwin", install=False)

    assert calls == [
        (
            [
                "npm", "run", "build:sea", "--",
                "aarch64-apple-darwin",
            ],
            build.RUNTIME,
        ),
        (
            [
                build.sys.executable,
                str(build.ROOT / "scripts/build-sidecar.py"),
                "--clean",
                "--target",
                "aarch64-apple-darwin",
            ],
            build.ROOT,
        ),
        (
            ["npm", "run", "tauri", "--", "build"],
            build.APP,
        ),
    ]
