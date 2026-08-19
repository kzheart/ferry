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


def test_engine_binary_keeps_the_external_bin_naming_rule():
    source, destination = build.engine_binary_paths("aarch64-apple-darwin")
    assert source == (
        build.ENGINE_CRATE
        / "target/aarch64-apple-darwin/release/ferry-engine"
    )
    assert destination == (
        build.BINARIES / "ferry-engine-aarch64-apple-darwin"
    )

    source, destination = build.engine_binary_paths("x86_64-pc-windows-msvc")
    assert source.name == "ferry-engine.exe"
    assert destination == (
        build.BINARIES / "ferry-engine-x86_64-pc-windows-msvc.exe"
    )


def test_engine_is_built_from_its_own_manifest():
    assert build.engine_build_command("aarch64-apple-darwin") == [
        "cargo",
        "build",
        "--release",
        "--locked",
        "--target",
        "aarch64-apple-darwin",
        "--manifest-path",
        str(build.ENGINE_MANIFEST),
    ]


def test_root_build_runs_both_sidecars_before_tauri(monkeypatch):
    calls = []
    monkeypatch.setattr(build, "verify_toolchain", lambda target: None)
    monkeypatch.setattr(build, "install_engine_binary", lambda target: None)
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
            build.engine_build_command("aarch64-apple-darwin"),
            build.ROOT,
        ),
        (
            ["npm", "run", "tauri", "--", "build"],
            build.APP,
        ),
    ]
